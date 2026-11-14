use std::collections::BTreeMap;
use std::fmt;

use nfc_core::{BLOCK_SIZE, CardInfo, CardType, Key, KeyType, NfcError, Result};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

use crate::access::{AccessBits, AccessCondition};
use crate::bcc::BccStatus;
use crate::{BlockStatus, Dump, DumpKey, DumpMetadata, Layout};

const PROJECT_VERSION: u32 = 2;
const PROJECT_MIN_VERSION: u32 = 1;
const CREATED_BY: &str = "NFC Foundry";

/// Dump interchange formats. The project format is the only one that can
/// represent partial images, masked bytes, and invalid trailer state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DumpFormat {
    /// Complete images export as raw, partial images as the project format.
    /// Loading with `auto` detects the format from the content.
    Auto,
    Project,
    Raw,
    Mct,
    Proxmark3,
}

impl DumpFormat {
    pub fn parse(name: &str) -> Result<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "project" => Ok(Self::Project),
            "raw" => Ok(Self::Raw),
            "mct" => Ok(Self::Mct),
            "proxmark3" => Ok(Self::Proxmark3),
            other => Err(NfcError::InvalidArgument(format!(
                "unknown dump format {other:?}; use auto, project, raw, mct, or proxmark3"
            ))),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Project => "project",
            Self::Raw => "raw",
            Self::Mct => "mct",
            Self::Proxmark3 => "proxmark3",
        }
    }

    /// Concrete format used when exporting this choice for `dump`.
    pub fn resolve(self, dump: &Dump) -> Self {
        match self {
            Self::Auto if dump.complete() => Self::Raw,
            Self::Auto => Self::Project,
            concrete => concrete,
        }
    }

    /// Whether this format can represent the image. External formats need
    /// every byte to be known; the project format keeps unknown bytes.
    pub fn can_represent(self, dump: &Dump) -> bool {
        match self.resolve(dump) {
            Self::Project => true,
            _ => dump.complete(),
        }
    }

    /// Default file name for this format. `auto` resolves first.
    pub const fn default_file_name(self) -> &'static str {
        match self {
            Self::Auto | Self::Raw => "card-dump.bin",
            Self::Mct => "card-dump.mct",
            Self::Proxmark3 => "card-dump.mfc.json",
            Self::Project => "card-dump.nfc-foundry.json",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DumpLoad {
    pub format: DumpFormat,
    pub dump: Dump,
    pub warnings: Vec<String>,
}

/// Detects a dump format from its contents. JSON project files and Proxmark3
/// exports are distinguished by their required fields.
pub fn detect_dump_format(data: &[u8]) -> Result<DumpFormat> {
    let Some(first) = data.iter().find(|byte| !byte.is_ascii_whitespace()) else {
        return Err(NfcError::InvalidArgument("dump file is empty".to_owned()));
    };
    if *first == b'{' {
        let value = parse_json_value(data)?;
        return detect_json_format(&value);
    }
    match data.len() {
        size if size == Layout::Classic1K.dump_size() => return Ok(DumpFormat::Raw),
        size if size == Layout::Classic4K.dump_size() => return Ok(DumpFormat::Raw),
        _ => {}
    }
    if is_mct_text(data) {
        return Ok(DumpFormat::Mct);
    }
    Err(NfcError::InvalidArgument(format!(
        "unrecognized dump of {} bytes; expected {} or {} raw bytes, MCT text, or JSON",
        data.len(),
        Layout::Classic1K.dump_size(),
        Layout::Classic4K.dump_size()
    )))
}

/// Detects and loads a dump.
pub fn load(data: &[u8]) -> Result<DumpLoad> {
    let format = detect_dump_format(data)?;
    load_as(data, format)
}

/// Loads a dump in a known format. External formats reject incomplete images,
/// malformed trailers, and inconsistent BCC bytes; project files preserve state
/// exactly.
pub fn load_as(data: &[u8], format: DumpFormat) -> Result<DumpLoad> {
    match format {
        DumpFormat::Auto => load(data),
        DumpFormat::Project => Ok(DumpLoad {
            format,
            dump: parse_project(data)?,
            warnings: Vec::new(),
        }),
        DumpFormat::Raw => {
            let dump = Dump::from_raw(data)?;
            dump.validate_trailers()?;
            Ok(DumpLoad {
                format,
                dump,
                warnings: vec![
                    "raw dump has no card identity; BCC validation is not possible".to_owned(),
                ],
            })
        }
        DumpFormat::Mct => {
            let dump = parse_mct(data)?;
            dump.validate_trailers()?;
            Ok(DumpLoad {
                format,
                dump,
                warnings: vec![
                    "MCT text dump has no card identity; BCC validation is not possible".to_owned(),
                ],
            })
        }
        DumpFormat::Proxmark3 => parse_proxmark(data).map(|mut load| {
            load.format = format;
            load
        }),
    }
}

/// Serializes a dump. Raw, MCT, and Proxmark3 exports require a complete image
/// with valid trailers; the project format preserves partial state.
pub fn export(dump: &Dump, format: DumpFormat) -> Result<Vec<u8>> {
    dump.validate_geometry()?;
    if let Some(card) = &dump.card {
        validate_card_identity(card)?;
    }
    match format.resolve(dump) {
        DumpFormat::Auto => unreachable!("resolve never returns auto"),
        DumpFormat::Project => encode_project(dump),
        DumpFormat::Raw => {
            validate_external(dump)?;
            dump.raw()
        }
        DumpFormat::Mct => {
            validate_external(dump)?;
            encode_mct(dump)
        }
        DumpFormat::Proxmark3 => {
            validate_external(dump)?;
            encode_proxmark(dump)
        }
    }
}

fn validate_card_identity(card: &CardInfo) -> Result<()> {
    if !matches!(card.uid.len(), 0 | 4 | 7 | 10) {
        return Err(NfcError::InvalidArgument(format!(
            "card UID must contain 0, 4, 7, or 10 bytes, found {}",
            card.uid.len()
        )));
    }
    Ok(())
}

fn validate_external(dump: &Dump) -> Result<()> {
    if !dump.complete() {
        return Err(NfcError::VerificationFailed(
            "external dump formats require a complete image".to_owned(),
        ));
    }
    dump.validate_trailers()?;
    if let Some(card) = &dump.card {
        match dump.validate_bcc(card.uid.len())? {
            BccStatus::Valid | BccStatus::NotApplicable => {}
            BccStatus::Invalid { expected, actual } => {
                return Err(NfcError::VerificationFailed(format!(
                    "manufacturer block BCC is {actual:02X}, expected {expected:02X}"
                )));
            }
        }
    }
    Ok(())
}

// ---- project format -------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct ProjectDocument {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    complete: Option<bool>,
    layout: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    card: Option<ProjectCard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    #[serde(default, rename = "startedAt", skip_serializing_if = "Option::is_none")]
    started_at: Option<u64>,
    #[serde(
        default,
        rename = "finishedAt",
        skip_serializing_if = "Option::is_none"
    )]
    finished_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    keys: Vec<ProjectKey>,
    blocks: Vec<ProjectBlock>,
}

#[derive(Serialize, Deserialize)]
struct ProjectKey {
    sector: usize,
    #[serde(rename = "type")]
    key_type: String,
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ProjectCard {
    #[serde(default)]
    uid: String,
    #[serde(default)]
    atqa: String,
    #[serde(default)]
    sak: u8,
}

#[derive(Serialize, Deserialize)]
struct ProjectBlock {
    block: usize,
    sector: usize,
    data: String,
    #[serde(rename = "knownMask")]
    known_mask: String,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn encode_project(dump: &Dump) -> Result<Vec<u8>> {
    let document = ProjectDocument {
        version: PROJECT_VERSION,
        complete: Some(dump.complete()),
        layout: dump.layout.name().to_owned(),
        card: dump.card.as_ref().map(project_card),
        device: dump.metadata.device.clone(),
        started_at: dump.metadata.started_at,
        finished_at: dump.metadata.finished_at,
        keys: dump
            .metadata
            .keys
            .iter()
            .map(|entry| ProjectKey {
                sector: entry.sector,
                key_type: entry.key_type.label().to_string(),
                key: entry.key.to_hex(),
                source: entry.source.clone(),
            })
            .collect(),
        blocks: dump
            .blocks
            .iter()
            .map(|block| ProjectBlock {
                block: block.number,
                sector: block.sector,
                data: encode_hex(&block.data),
                known_mask: format!("{:04X}", block.known_mask),
                status: status_name(block.status).to_owned(),
                error: block.error.clone(),
            })
            .collect(),
    };
    let mut encoded = serde_json::to_vec_pretty(&document).map_err(|error| {
        NfcError::InvalidArgument(format!("cannot encode dump project: {error}"))
    })?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn parse_project(data: &[u8]) -> Result<Dump> {
    let document: ProjectDocument = parse_json(data)?;
    if document.version < PROJECT_MIN_VERSION || document.version > PROJECT_VERSION {
        return Err(NfcError::InvalidArgument(format!(
            "unsupported dump project version {}; expected {}..={PROJECT_VERSION}",
            document.version, PROJECT_MIN_VERSION
        )));
    }
    let layout = Layout::from_name(&document.layout)?;
    let mut dump = Dump::new(layout);
    dump.card = document
        .card
        .as_ref()
        .map(decode_project_card)
        .transpose()?
        .flatten();
    dump.metadata.device = document.device.filter(|value| !value.trim().is_empty());
    dump.metadata.started_at = document.started_at;
    dump.metadata.finished_at = document.finished_at;
    dump.metadata.keys = document
        .keys
        .into_iter()
        .map(|entry| decode_project_key(layout, entry))
        .collect::<Result<Vec<_>>>()?;
    if document.blocks.len() != layout.block_count() {
        return Err(NfcError::InvalidArgument(format!(
            "dump project contains {} blocks, expected {}",
            document.blocks.len(),
            layout.block_count()
        )));
    }
    for (number, stored) in document.blocks.into_iter().enumerate() {
        let expected_sector = layout
            .sector_for_block(number)
            .expect("index within the layout");
        if stored.block != number || stored.sector != expected_sector {
            return Err(NfcError::InvalidArgument(format!(
                "dump project block {number} has inconsistent address"
            )));
        }
        let data = decode_hex("block data", &stored.data, BLOCK_SIZE)?;
        let known_mask = decode_known_mask(number, &stored.known_mask)?;
        let status = parse_status(&stored.status)?;
        dump.blocks[number].data = data.try_into().expect("decode_hex checked the block size");
        dump.blocks[number].known_mask = known_mask;
        dump.blocks[number].status = status;
        dump.blocks[number].error = stored.error;
    }
    if let Some(complete) = document.complete
        && complete != dump.complete()
    {
        return Err(NfcError::InvalidArgument(
            "dump project completeness flag does not match block certainty".to_owned(),
        ));
    }
    dump.validate_metadata()?;
    Ok(dump)
}

fn decode_project_key(layout: Layout, stored: ProjectKey) -> Result<DumpKey> {
    if stored.sector >= layout.sector_count() {
        return Err(NfcError::InvalidArgument(format!(
            "dump project key names sector {}, outside the {} layout",
            stored.sector,
            layout.name()
        )));
    }
    let key_type = match stored.key_type.trim().to_ascii_uppercase().as_str() {
        "A" => KeyType::A,
        "B" => KeyType::B,
        other => {
            return Err(NfcError::InvalidArgument(format!(
                "dump project sector {} key type {other:?} must be A or B",
                stored.sector
            )));
        }
    };
    let key = Key::from_hex(&stored.key).map_err(|error| {
        NfcError::InvalidArgument(format!(
            "dump project sector {} key {}: {error}",
            stored.sector,
            key_type.label()
        ))
    })?;
    Ok(DumpKey {
        sector: stored.sector,
        key_type,
        key,
        source: stored.source.filter(|value| !value.trim().is_empty()),
    })
}

fn project_card(card: &CardInfo) -> ProjectCard {
    ProjectCard {
        uid: encode_hex(&card.uid),
        atqa: encode_hex(&card.atqa),
        sak: card.sak,
    }
}

fn decode_project_card(card: &ProjectCard) -> Result<Option<CardInfo>> {
    decode_card_fields(&card.uid, &card.atqa, card.sak)
}

fn decode_known_mask(block: usize, value: &str) -> Result<u16> {
    let value = value.trim();
    if value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(NfcError::InvalidArgument(format!(
            "dump project block {block} has an invalid known mask"
        )));
    }
    u16::from_str_radix(value, 16).map_err(|_| {
        NfcError::InvalidArgument(format!(
            "dump project block {block} has an invalid known mask"
        ))
    })
}

// ---- image-only sidecars --------------------------------------------------

const SIDECAR_VERSION: u32 = 1;

/// Metadata recovered from an `<image>.nfcx.json` sidecar. The image bytes
/// stay authoritative: a sidecar only restores provenance, card identity, and
/// authenticated sector keys.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SidecarLoad {
    pub layout: Layout,
    pub complete: Option<bool>,
    pub card: Option<CardInfo>,
    pub metadata: DumpMetadata,
    pub warnings: Vec<String>,
    /// False when the version is unknown. The caller ignores the metadata and
    /// reports the warning; the image still loads.
    pub usable: bool,
}

#[derive(Serialize)]
struct SidecarExport {
    version: u32,
    complete: bool,
    layout: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    card: Option<ProjectCard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    #[serde(rename = "startedAt", skip_serializing_if = "Option::is_none")]
    started_at: Option<u64>,
    #[serde(rename = "finishedAt", skip_serializing_if = "Option::is_none")]
    finished_at: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    keys: Vec<ProjectKey>,
}

#[derive(Deserialize)]
struct SidecarDocument {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    complete: Option<bool>,
    layout: String,
    #[serde(default)]
    card: Option<ProjectCard>,
    #[serde(default)]
    device: Option<String>,
    #[serde(default, rename = "startedAt")]
    started_at: Option<u64>,
    #[serde(default, rename = "finishedAt")]
    finished_at: Option<u64>,
    #[serde(default)]
    keys: Vec<SidecarKey>,
    /// Legacy project files carry the full block image. It is validated for
    /// the layout and discarded; the image file stays authoritative.
    #[serde(default)]
    blocks: Option<Value>,
}

#[derive(Deserialize)]
struct SidecarKey {
    sector: usize,
    #[serde(default, rename = "type")]
    key_type: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default, rename = "keyA")]
    key_a: Option<String>,
    #[serde(default, rename = "keyB")]
    key_b: Option<String>,
}

/// Serializes the metadata sidecar for an image-only export.
pub fn encode_sidecar(dump: &Dump) -> Result<Vec<u8>> {
    dump.validate_geometry()?;
    if let Some(card) = &dump.card {
        validate_card_identity(card)?;
    }
    let document = SidecarExport {
        version: SIDECAR_VERSION,
        complete: dump.complete(),
        layout: dump.layout.name().to_owned(),
        card: dump.card.as_ref().map(project_card),
        device: dump.metadata.device.clone(),
        started_at: dump.metadata.started_at,
        finished_at: dump.metadata.finished_at,
        keys: dump
            .metadata
            .keys
            .iter()
            .map(|entry| ProjectKey {
                sector: entry.sector,
                key_type: entry.key_type.label().to_string(),
                key: entry.key.to_hex(),
                source: entry.source.clone(),
            })
            .collect(),
    };
    let mut encoded = serde_json::to_vec_pretty(&document).map_err(|error| {
        NfcError::InvalidArgument(format!("cannot encode dump sidecar: {error}"))
    })?;
    encoded.push(b'\n');
    Ok(encoded)
}

/// Parses a sidecar. Unknown versions and legacy key/block shapes are accepted
/// here so the caller can decide whether to apply the metadata.
pub fn parse_sidecar(data: &[u8]) -> Result<SidecarLoad> {
    let document: SidecarDocument = parse_json(data)?;
    let layout = Layout::from_name(&document.layout)?;
    if document.version != Some(SIDECAR_VERSION) {
        let version = document
            .version
            .map(|version| version.to_string())
            .unwrap_or_else(|| "missing".to_owned());
        return Ok(SidecarLoad {
            layout,
            complete: None,
            card: None,
            metadata: DumpMetadata::default(),
            warnings: vec![format!(
                "sidecar version {version} is not supported; metadata ignored"
            )],
            usable: false,
        });
    }
    if let Some(blocks) = &document.blocks {
        validate_sidecar_blocks(layout, blocks)?;
    }
    let mut metadata = DumpMetadata {
        device: document.device.filter(|value| !value.trim().is_empty()),
        started_at: document.started_at,
        finished_at: document.finished_at,
        keys: Vec::new(),
    };
    for entry in document.keys {
        metadata.keys.extend(decode_sidecar_key(layout, entry)?);
    }
    let card = document
        .card
        .as_ref()
        .map(decode_project_card)
        .transpose()?
        .flatten();
    let mut probe = Dump::new(layout);
    probe.card = card.clone();
    probe.metadata = metadata.clone();
    probe.validate_metadata()?;
    Ok(SidecarLoad {
        layout,
        complete: document.complete,
        card,
        metadata,
        warnings: Vec::new(),
        usable: true,
    })
}

fn decode_sidecar_key(layout: Layout, stored: SidecarKey) -> Result<Vec<DumpKey>> {
    if let Some(key_type) = stored.key_type {
        let key = stored.key.ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "sidecar sector {} key type {} has no key",
                stored.sector, key_type
            ))
        })?;
        return Ok(vec![decode_project_key(
            layout,
            ProjectKey {
                sector: stored.sector,
                key_type,
                key,
                source: stored.source,
            },
        )?]);
    }
    if stored.key.is_some() {
        return Err(NfcError::InvalidArgument(format!(
            "sidecar sector {} key has no type",
            stored.sector
        )));
    }
    if stored.sector >= layout.sector_count() {
        return Err(NfcError::InvalidArgument(format!(
            "sidecar key names sector {}, outside the {} layout",
            stored.sector,
            layout.name()
        )));
    }
    let source = stored.source.filter(|value| !value.trim().is_empty());
    let mut keys = Vec::new();
    for (label, value, key_type) in [
        ("keyA", stored.key_a, KeyType::A),
        ("keyB", stored.key_b, KeyType::B),
    ] {
        let Some(value) = value.filter(|value| !value.trim().is_empty()) else {
            continue;
        };
        let key = Key::from_hex(&value).map_err(|error| {
            NfcError::InvalidArgument(format!("sidecar sector {} {label}: {error}", stored.sector))
        })?;
        keys.push(DumpKey {
            sector: stored.sector,
            key_type,
            key,
            source: source.clone(),
        });
    }
    if keys.is_empty() {
        return Err(NfcError::InvalidArgument(format!(
            "sidecar sector {} has no key",
            stored.sector
        )));
    }
    Ok(keys)
}

fn validate_sidecar_blocks(layout: Layout, blocks: &Value) -> Result<()> {
    let count = match blocks {
        Value::Array(entries) => entries.len(),
        Value::Object(entries) => {
            for name in entries.keys() {
                let number: usize = name.trim().parse().map_err(|_| {
                    NfcError::InvalidArgument(format!(
                        "sidecar legacy block name {name:?} is not a block number"
                    ))
                })?;
                if number >= layout.block_count() {
                    return Err(NfcError::InvalidArgument(format!(
                        "sidecar legacy block {number} is outside the {} layout",
                        layout.name()
                    )));
                }
            }
            entries.len()
        }
        _ => {
            return Err(NfcError::InvalidArgument(
                "sidecar legacy blocks must be an array or object".to_owned(),
            ));
        }
    };
    if count != layout.block_count() {
        return Err(NfcError::InvalidArgument(format!(
            "sidecar has {count} legacy blocks, expected {} for {}",
            layout.block_count(),
            layout.name()
        )));
    }
    Ok(())
}

fn status_name(status: BlockStatus) -> &'static str {
    match status {
        BlockStatus::Unread => "unread",
        BlockStatus::Read => "read",
        BlockStatus::AuthFailed => "auth_failed",
        BlockStatus::ReadFailed => "read_failed",
        BlockStatus::Synthetic => "synthetic",
        BlockStatus::Verified => "verified",
    }
}

fn parse_status(status: &str) -> Result<BlockStatus> {
    match status {
        "unread" => Ok(BlockStatus::Unread),
        "read" => Ok(BlockStatus::Read),
        "auth_failed" => Ok(BlockStatus::AuthFailed),
        "read_failed" => Ok(BlockStatus::ReadFailed),
        "synthetic" => Ok(BlockStatus::Synthetic),
        "verified" => Ok(BlockStatus::Verified),
        other => Err(NfcError::InvalidArgument(format!(
            "unknown dump block status {other:?}"
        ))),
    }
}

// ---- raw and MCT formats --------------------------------------------------

fn parse_mct(data: &[u8]) -> Result<Dump> {
    let text = std::str::from_utf8(data)
        .map_err(|_| NfcError::InvalidArgument("MCT dump is not valid UTF-8 text".to_owned()))?;
    let mut sectors: BTreeMap<usize, Vec<[u8; BLOCK_SIZE]>> = BTreeMap::new();
    let mut current = None;
    let mut four_k = false;
    for (line_number, raw_line) in text.lines().enumerate() {
        let line_number = line_number + 1;
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(sector) = parse_mct_header(line)? {
            if sector >= 16 {
                four_k = true;
            }
            if sectors.insert(sector, Vec::new()).is_some() {
                return Err(NfcError::InvalidArgument(format!(
                    "MCT line {line_number}: sector {sector} appears more than once"
                )));
            }
            current = Some(sector);
            continue;
        }
        let Some(sector) = current else {
            return Err(NfcError::InvalidArgument(format!(
                "MCT line {line_number}: block data appears before a +Sector header"
            )));
        };
        let bytes = decode_hex(&format!("MCT line {line_number} block"), line, BLOCK_SIZE)?;
        sectors
            .get_mut(&sector)
            .expect("current sector was inserted")
            .push(bytes.try_into().expect("decode_hex checked the block size"));
    }
    let layout = if four_k {
        Layout::Classic4K
    } else {
        Layout::Classic1K
    };
    let raw = mct_sectors_to_raw(layout, &sectors)?;
    Dump::from_raw(&raw)
}

fn parse_mct_header(line: &str) -> Result<Option<usize>> {
    let bytes = line.as_bytes();
    let Some(prefix) = bytes.get(..8) else {
        return Ok(None);
    };
    if !prefix.eq_ignore_ascii_case(b"+Sector:") {
        return Ok(None);
    }
    let value = line.get(8..).unwrap_or_default().trim();
    if value.is_empty() {
        return Err(NfcError::InvalidArgument(
            "+Sector header has no sector number".to_owned(),
        ));
    }
    let sector = value
        .parse::<usize>()
        .map_err(|_| NfcError::InvalidArgument(format!("invalid MCT sector number {value:?}")))?;
    if sector >= Layout::Classic4K.sector_count() {
        return Err(NfcError::InvalidArgument(format!(
            "MCT sector {sector} is outside the Classic 4K layout"
        )));
    }
    Ok(Some(sector))
}

fn mct_sectors_to_raw(
    layout: Layout,
    sectors: &BTreeMap<usize, Vec<[u8; BLOCK_SIZE]>>,
) -> Result<Vec<u8>> {
    let mut raw = vec![0u8; layout.dump_size()];
    for sector in 0..layout.sector_count() {
        let blocks = sectors.get(&sector).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "MCT dump is missing sector {sector} for {}",
                layout.name()
            ))
        })?;
        let expected = layout
            .blocks_in_sector(sector)
            .expect("sector within the layout");
        if blocks.len() != expected {
            return Err(NfcError::InvalidArgument(format!(
                "MCT sector {sector} has {} blocks, expected {expected}",
                blocks.len()
            )));
        }
        let first = layout
            .first_block(sector)
            .expect("sector within the layout");
        for (index, block) in blocks.iter().enumerate() {
            let offset = (first + index) * BLOCK_SIZE;
            raw[offset..offset + BLOCK_SIZE].copy_from_slice(block);
        }
    }
    for sector in sectors.keys() {
        if *sector >= layout.sector_count() {
            return Err(NfcError::InvalidArgument(format!(
                "MCT dump contains sector {sector} outside the {} layout",
                layout.name()
            )));
        }
    }
    Ok(raw)
}

fn encode_mct(dump: &Dump) -> Result<Vec<u8>> {
    let mut text = String::new();
    for sector in 0..dump.layout.sector_count() {
        text.push_str(&format!("+Sector: {sector}\n"));
        let first = dump
            .layout
            .first_block(sector)
            .expect("sector within the layout");
        let count = dump
            .layout
            .blocks_in_sector(sector)
            .expect("sector within the layout");
        for block in dump.blocks.iter().skip(first).take(count) {
            text.push_str(&encode_hex(&block.data));
            text.push('\n');
        }
    }
    Ok(text.into_bytes())
}

// ---- Proxmark3 JSON -------------------------------------------------------

#[derive(Deserialize)]
struct ProxmarkDocument {
    #[serde(rename = "FileType")]
    file_type: String,
    #[serde(rename = "Card")]
    card: Option<ProxmarkCard>,
    #[serde(default)]
    blocks: BTreeMap<String, String>,
    #[serde(rename = "SectorKeys", default)]
    sector_keys: Option<BTreeMap<String, ProxmarkSectorKeys>>,
}

#[derive(Deserialize)]
struct ProxmarkCard {
    #[serde(rename = "UID", default)]
    uid: String,
    #[serde(rename = "ATQA", default)]
    atqa: String,
    #[serde(rename = "SAK", default)]
    sak: String,
    #[serde(rename = "ATS", default)]
    ats: Option<String>,
    #[serde(rename = "SIGNATURE", default)]
    signature: Option<String>,
}

#[derive(Deserialize)]
struct ProxmarkSectorKeys {
    #[serde(rename = "KeyA", default)]
    key_a: String,
    #[serde(rename = "KeyB", default)]
    key_b: String,
    #[serde(rename = "AccessConditions", default)]
    access_conditions: String,
}

fn parse_proxmark(data: &[u8]) -> Result<DumpLoad> {
    let document: ProxmarkDocument = parse_json(data)?;
    let version = document.file_type.trim().to_ascii_lowercase();
    if version != "mfc v2" && version != "mfc v3" {
        return Err(NfcError::InvalidArgument(format!(
            "Proxmark3 JSON FileType is {:?}, expected mfc v2 or mfc v3",
            document.file_type
        )));
    }
    let card = document
        .card
        .as_ref()
        .ok_or_else(|| {
            NfcError::InvalidArgument("Proxmark3 JSON dump has no Card object".to_owned())
        })
        .and_then(decode_proxmark_card)?;
    let raw = proxmark_blocks_to_raw(&document.blocks)?;
    let mut dump = Dump::from_raw(&raw)?;
    dump.validate_trailers()?;
    if let Some(card) = card {
        match dump.validate_bcc(card.uid.len())? {
            BccStatus::Valid | BccStatus::NotApplicable => {}
            BccStatus::Invalid { expected, actual } => {
                return Err(NfcError::VerificationFailed(format!(
                    "manufacturer block BCC is {actual:02X}, expected {expected:02X}"
                )));
            }
        }
        dump.card = Some(card);
    }
    if let Some(keys) = &document.sector_keys {
        validate_proxmark_sector_keys(dump.layout, &dump, keys)?;
    }
    let mut warnings = Vec::new();
    if dump.card.as_ref().is_none_or(|card| card.uid.is_empty()) {
        warnings.push("Proxmark3 JSON has no card UID; BCC validation is not possible".to_owned());
    }
    if version == "mfc v3" {
        if document
            .card
            .as_ref()
            .and_then(|card| card.ats.as_ref())
            .is_some_and(|ats| !ats.trim().is_empty())
        {
            warnings.push(
                "Proxmark3 mfc v3 Card.ATS was dropped; the dump model does not store ATS"
                    .to_owned(),
            );
        }
        if document
            .card
            .as_ref()
            .and_then(|card| card.signature.as_ref())
            .is_some_and(|signature| !signature.trim().is_empty())
        {
            warnings.push(
                "Proxmark3 mfc v3 Card.SIGNATURE was dropped; the dump model does not store signatures"
                    .to_owned(),
            );
        }
    }
    Ok(DumpLoad {
        format: DumpFormat::Proxmark3,
        dump,
        warnings,
    })
}

fn decode_proxmark_card(card: &ProxmarkCard) -> Result<Option<CardInfo>> {
    let sak = if card.sak.trim().is_empty() {
        0
    } else {
        let bytes = decode_hex("card SAK", &card.sak, 1).map_err(|_| {
            NfcError::InvalidArgument("card SAK must contain 2 hexadecimal digits".into())
        })?;
        bytes[0]
    };
    decode_card_fields(&card.uid, &card.atqa, sak)
}

fn proxmark_blocks_to_raw(blocks: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    let layout = match blocks.len() {
        count if count == Layout::Classic1K.block_count() => Layout::Classic1K,
        count if count == Layout::Classic4K.block_count() => Layout::Classic4K,
        count => {
            return Err(NfcError::InvalidArgument(format!(
                "Proxmark3 dump has {count} blocks; expected {} or {}",
                Layout::Classic1K.block_count(),
                Layout::Classic4K.block_count()
            )));
        }
    };
    let mut raw = vec![0u8; layout.dump_size()];
    let mut seen = vec![false; layout.block_count()];
    for (key, value) in blocks {
        let block = key.trim().parse::<usize>().map_err(|_| {
            NfcError::InvalidArgument(format!(
                "Proxmark3 block key {key:?} is not a decimal block number"
            ))
        })?;
        if block >= layout.block_count() {
            return Err(NfcError::InvalidArgument(format!(
                "Proxmark3 block {block} is outside the {} layout",
                layout.name()
            )));
        }
        if seen[block] {
            return Err(NfcError::InvalidArgument(format!(
                "Proxmark3 block {block} appears more than once"
            )));
        }
        let bytes = decode_hex(&format!("Proxmark3 block {block}"), value, BLOCK_SIZE)?;
        let offset = block * BLOCK_SIZE;
        raw[offset..offset + BLOCK_SIZE].copy_from_slice(&bytes);
        seen[block] = true;
    }
    if let Some(missing) = seen.iter().position(|seen| !seen) {
        return Err(NfcError::InvalidArgument(format!(
            "Proxmark3 block {missing} is missing"
        )));
    }
    Ok(raw)
}

fn validate_proxmark_sector_keys(
    layout: Layout,
    dump: &Dump,
    keys: &BTreeMap<String, ProxmarkSectorKeys>,
) -> Result<()> {
    for (key, stored) in keys {
        let sector = key.trim().parse::<usize>().map_err(|_| {
            NfcError::InvalidArgument(format!("SectorKeys key {key:?} is not a sector number"))
        })?;
        if sector >= layout.sector_count() {
            return Err(NfcError::InvalidArgument(format!(
                "SectorKeys sector {sector} is outside the {} layout",
                layout.name()
            )));
        }
        let trailer = layout.trailer_block(sector)?;
        let trailer = &dump.blocks[trailer].data;
        if !stored.key_a.trim().is_empty() {
            validate_sector_key("Key A", sector, &stored.key_a, &trailer[0..6])?;
        }
        if !stored.key_b.trim().is_empty() {
            validate_sector_key("Key B", sector, &stored.key_b, &trailer[10..16])?;
        }
        if !stored.access_conditions.trim().is_empty() {
            let access =
                decode_hex("AccessConditions", &stored.access_conditions, 4).map_err(|_| {
                    NfcError::InvalidArgument(format!(
                        "sector {sector} AccessConditions must contain 8 hexadecimal digits"
                    ))
                })?;
            if access != trailer[6..10] {
                return Err(NfcError::InvalidArgument(format!(
                    "sector {sector} AccessConditions do not match the trailer block"
                )));
            }
        }
    }
    Ok(())
}

fn validate_sector_key(name: &str, sector: usize, value: &str, expected: &[u8]) -> Result<()> {
    let decoded = decode_hex(name, value, 6).map_err(|_| {
        NfcError::InvalidArgument(format!(
            "sector {sector} {name} must contain 12 hexadecimal digits"
        ))
    })?;
    if decoded != expected {
        return Err(NfcError::InvalidArgument(format!(
            "sector {sector} {name} does not match the trailer block"
        )));
    }
    Ok(())
}

#[derive(Serialize)]
struct ProxmarkExport {
    #[serde(rename = "Created")]
    created: &'static str,
    #[serde(rename = "FileType")]
    file_type: &'static str,
    #[serde(rename = "Card")]
    card: ProxmarkCardExport,
    blocks: BTreeMap<String, String>,
    #[serde(rename = "SectorKeys")]
    sector_keys: BTreeMap<String, ProxmarkSectorExport>,
}

#[derive(Serialize)]
struct ProxmarkCardExport {
    #[serde(rename = "UID")]
    uid: String,
    #[serde(rename = "ATQA")]
    atqa: String,
    #[serde(rename = "SAK")]
    sak: String,
}

#[derive(Serialize)]
struct ProxmarkSectorExport {
    #[serde(rename = "KeyA")]
    key_a: String,
    #[serde(rename = "KeyB")]
    key_b: String,
    #[serde(rename = "AccessConditions")]
    access_conditions: String,
    #[serde(
        rename = "AccessConditionsText",
        skip_serializing_if = "Option::is_none"
    )]
    access_conditions_text: Option<BTreeMap<String, String>>,
}

fn encode_proxmark(dump: &Dump) -> Result<Vec<u8>> {
    let (uid, atqa, sak) = match &dump.card {
        Some(card) => (
            encode_hex(&card.uid),
            encode_hex(&card.atqa),
            format!("{:02X}", card.sak),
        ),
        None => (String::new(), String::new(), "00".to_owned()),
    };
    let blocks = dump
        .blocks
        .iter()
        .enumerate()
        .map(|(number, block)| (number.to_string(), encode_hex(&block.data)))
        .collect();
    let mut sector_keys = BTreeMap::new();
    for sector in 0..dump.layout.sector_count() {
        let trailer = dump
            .layout
            .trailer_block(sector)
            .expect("sector within the layout");
        let trailer = &dump.blocks[trailer].data;
        sector_keys.insert(
            sector.to_string(),
            ProxmarkSectorExport {
                key_a: encode_hex(&trailer[0..6]),
                key_b: encode_hex(&trailer[10..16]),
                access_conditions: encode_hex(&trailer[6..10]),
                access_conditions_text: access_conditions_text(dump.layout, sector, trailer),
            },
        );
    }
    let document = ProxmarkExport {
        created: CREATED_BY,
        file_type: "mfc v2",
        card: ProxmarkCardExport { uid, atqa, sak },
        blocks,
        sector_keys,
    };
    let mut encoded = serde_json::to_vec_pretty(&document).map_err(|error| {
        NfcError::InvalidArgument(format!("cannot encode Proxmark3 dump: {error}"))
    })?;
    encoded.push(b'\n');
    Ok(encoded)
}

fn access_conditions_text(
    layout: Layout,
    sector: usize,
    trailer: &[u8; BLOCK_SIZE],
) -> Option<BTreeMap<String, String>> {
    let bits = AccessBits::decode([trailer[6], trailer[7], trailer[8]]).ok()?;
    let first = layout.first_block(sector).ok()?;
    let count = layout.blocks_in_sector(sector).ok()?;
    let trailer_block = first + count - 1;
    let labels = if count == 16 {
        [
            format!("blocks{first}-{}", first + 4),
            format!("blocks{}-{}", first + 5, first + 9),
            format!("blocks{}-{}", first + 10, first + 14),
            format!("block{trailer_block}"),
        ]
    } else {
        [
            format!("block{first}"),
            format!("block{}", first + 1),
            format!("block{}", first + 2),
            format!("block{trailer_block}"),
        ]
    };
    let mut text = BTreeMap::new();
    for (group, label) in labels.into_iter().enumerate() {
        text.insert(
            label,
            access_condition_description(bits.groups[group], group == 3).to_owned(),
        );
    }
    text.insert("UserData".to_owned(), format!("{:02X}", trailer[9]));
    Some(text)
}

fn access_condition_description(condition: AccessCondition, trailer: bool) -> &'static str {
    if trailer {
        match condition.code() {
            0 => "Key A write A; access bits read A, write A; Key B read A, write A",
            1 => "Key A write A; access bits read A, write never; Key B read A, write A",
            2 => {
                "Key A write never; access bits read A, write never; Key B read never, write never"
            }
            3 => "Key A write B; access bits read A|B, write B; Key B write B",
            4 => "Key A write B; access bits read A|B, write never; Key B read never, write B",
            _ => {
                "Key A write never; access bits read A|B, write never; Key B read never, write never"
            }
        }
    } else {
        match condition.code() {
            0 => "read A|B, write A|B, increment A|B, decrement A|B",
            1 => "read A|B, write never, increment never, decrement A|B",
            2 => "read A|B, write never, increment never, decrement never",
            3 => "read B, write B, increment never, decrement never",
            4 => "read A|B, write B, increment never, decrement never",
            5 => "read B, write never, increment never, decrement never",
            6 => "read A|B, write B, increment B, decrement B",
            _ => "read never, write never, increment never, decrement never",
        }
    }
}

// ---- shared helpers -------------------------------------------------------

fn decode_card_fields(uid: &str, atqa: &str, sak: u8) -> Result<Option<CardInfo>> {
    let uid_bytes = decode_optional_hex("card UID", uid, &[0, 4, 7, 10])?;
    let atqa_bytes = decode_optional_hex("card ATQA", atqa, &[0, 2])?;
    if uid_bytes.is_empty() && atqa_bytes.is_empty() && sak == 0 {
        return Ok(None);
    }
    let mut atqa = [0u8; 2];
    atqa[..atqa_bytes.len()].copy_from_slice(&atqa_bytes);
    Ok(Some(CardInfo::new(
        uid_bytes,
        atqa,
        sak,
        CardType::infer(atqa, sak),
    )))
}

fn decode_optional_hex(label: &str, value: &str, allowed: &[usize]) -> Result<Vec<u8>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Vec::new());
    }
    if !value.len().is_multiple_of(2) || !allowed.contains(&(value.len() / 2)) {
        return Err(NfcError::InvalidArgument(format!(
            "{label} must contain an even number of hexadecimal digits with a supported length"
        )));
    }
    decode_hex(label, value, value.len() / 2)
}

fn decode_hex(label: &str, value: &str, expected_bytes: usize) -> Result<Vec<u8>> {
    let value = value.trim();
    if value.len() != expected_bytes * 2 {
        return Err(NfcError::InvalidArgument(format!(
            "{label} must contain {} hexadecimal digits, found {}",
            expected_bytes * 2,
            value.len()
        )));
    }
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(expected_bytes);
    for pair in bytes.as_chunks::<2>().0 {
        let high = hex_nibble(pair[0]).ok_or_else(|| {
            NfcError::InvalidArgument(format!("{label} contains a non-hexadecimal character"))
        })?;
        let low = hex_nibble(pair[1]).ok_or_else(|| {
            NfcError::InvalidArgument(format!("{label} contains a non-hexadecimal character"))
        })?;
        decoded.push(high << 4 | low);
    }
    Ok(decoded)
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn is_mct_text(data: &[u8]) -> bool {
    data.iter()
        .all(|byte| byte.is_ascii_graphic() || byte.is_ascii_whitespace())
        && contains_mct_header(data)
}

fn contains_mct_header(data: &[u8]) -> bool {
    data.split(|byte| *byte == b'\n').any(|line| {
        let line = trim_ascii_whitespace(line);
        line.get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"+Sector:"))
    })
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn parse_json<T: serde::de::DeserializeOwned>(data: &[u8]) -> Result<T> {
    let UniqueValue(value) = serde_json::from_slice(data).map_err(json_error)?;
    serde_json::from_value(value).map_err(json_error)
}

fn parse_json_value(data: &[u8]) -> Result<Value> {
    let UniqueValue(value) = serde_json::from_slice(data).map_err(json_error)?;
    Ok(value)
}

fn json_error(error: serde_json::Error) -> NfcError {
    NfcError::InvalidArgument(format!("invalid JSON dump: {error}"))
}

fn detect_json_format(value: &Value) -> Result<DumpFormat> {
    let object = value.as_object().ok_or_else(|| {
        NfcError::InvalidArgument("JSON dump top level must be an object".to_owned())
    })?;
    if object.get("version").is_some_and(Value::is_number)
        && object.get("layout").is_some_and(Value::is_string)
        && object.get("blocks").is_some_and(Value::is_array)
    {
        return Ok(DumpFormat::Project);
    }
    if let Some(file_type) = object.get("FileType").and_then(Value::as_str) {
        let file_type = file_type.trim().to_ascii_lowercase();
        if file_type == "mfc v2" || file_type == "mfc v3" {
            return Ok(DumpFormat::Proxmark3);
        }
        return Err(NfcError::InvalidArgument(format!(
            "unsupported JSON dump FileType {file_type:?}"
        )));
    }
    Err(NfcError::InvalidArgument(
        "JSON dump has no recognized project fields or FileType".to_owned(),
    ))
}

/// JSON value that rejects duplicate object keys at every depth. `serde_json`'s
/// own map type would silently keep the last duplicate block entry.
struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(
            Number::from_f64(value).map_or(Value::Null, Value::Number),
        ))
    }

    fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_unit<E>(self) -> std::result::Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        UniqueValue::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueValue>()? {
            values.push(value.0);
        }
        Ok(UniqueValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut access: A) -> std::result::Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some((key, value)) = access.next_entry::<String, UniqueValue>()? {
            if object.contains_key(&key) {
                return Err(de::Error::custom(format!("duplicate JSON key {key:?}")));
            }
            object.insert(key, value.0);
        }
        Ok(UniqueValue(Value::Object(object)))
    }
}
