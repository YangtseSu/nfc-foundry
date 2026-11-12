//! GTK-free NDEF message editor model.
//!
//! The workbench composes a logical message from typed record drafts. Every
//! draft encodes through `nfc-ndef`; unknown or malformed card records fall
//! back to a raw draft so a loaded message is never silently rewritten or
//! dropped. Card I/O and safety gates stay in `nfc-app`.

use std::path::Path;

use nfc_ndef::{
    BluetoothLeRecord, DppBootstrap, LeAddressType, LeRole, NdefMessage, NdefRecord, SmartPoster,
    SmartPosterAction, TextEncoding, TextRecord, Tlv, Tnf, UriRecord, VCard, WifiRecord,
};

/// Record kinds exposed by the editor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordKind {
    #[default]
    Text,
    Uri,
    Media,
    Wifi,
    VCard,
    BluetoothLe,
    Dpp,
    SmartPoster,
    Raw,
}

impl RecordKind {
    pub const ALL: [Self; 9] = [
        Self::Text,
        Self::Uri,
        Self::Media,
        Self::Wifi,
        Self::VCard,
        Self::BluetoothLe,
        Self::Dpp,
        Self::SmartPoster,
        Self::Raw,
    ];

    /// English source label for the type selector and table.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Uri => "URI",
            Self::Media => "MIME",
            Self::Wifi => "Wi-Fi",
            Self::VCard => "vCard",
            Self::BluetoothLe => "Bluetooth LE",
            Self::Dpp => "Wi-Fi DPP",
            Self::SmartPoster => "Smart Poster",
            Self::Raw => "Raw record",
        }
    }

    pub const fn stack_name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Uri => "uri",
            Self::Media => "media",
            Self::Wifi => "wifi",
            Self::VCard => "vcard",
            Self::BluetoothLe => "ble",
            Self::Dpp => "dpp",
            Self::SmartPoster => "poster",
            Self::Raw => "raw",
        }
    }

    /// A fresh draft with the minimum valid fields for this kind.
    pub fn default_draft(self) -> RecordDraft {
        match self {
            Self::Text => RecordDraft::Text(TextDraft::default()),
            Self::Uri => RecordDraft::Uri(UriDraft::default()),
            Self::Media => RecordDraft::Media(MediaDraft::default()),
            Self::Wifi => RecordDraft::Wifi(WifiDraft::default()),
            Self::VCard => RecordDraft::VCard(VCardDraft::default()),
            Self::BluetoothLe => RecordDraft::BluetoothLe(BluetoothDraft::default()),
            Self::Dpp => RecordDraft::Dpp(DppDraft::default()),
            Self::SmartPoster => RecordDraft::SmartPoster(SmartPosterDraft::default()),
            Self::Raw => RecordDraft::Raw(RawDraft::default()),
        }
    }
}

/// Text record draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextDraft {
    pub text: String,
    pub language: String,
    pub encoding: TextEncoding,
}

impl Default for TextDraft {
    fn default() -> Self {
        Self {
            text: String::new(),
            language: "en".to_owned(),
            encoding: TextEncoding::Utf8,
        }
    }
}

/// URI record draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UriDraft {
    pub uri: String,
}

/// Media-type record draft. The type and payload are bytes so a loaded
/// non-UTF-8 record survives an edit round trip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MediaDraft {
    pub media_type: Vec<u8>,
    pub payload: Vec<u8>,
}

/// Wi-Fi WSC credential draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiDraft {
    pub ssid: Vec<u8>,
    pub authentication: u16,
    pub encryption: u16,
    pub network_key: Option<Vec<u8>>,
    pub mac_address: Option<[u8; 6]>,
    pub credential_attributes: Vec<(u16, Vec<u8>)>,
    pub outer_attributes: Vec<(u16, Vec<u8>)>,
}

impl Default for WifiDraft {
    fn default() -> Self {
        Self {
            ssid: Vec::new(),
            authentication: 0x0001,
            encryption: 0x0001,
            network_key: None,
            mac_address: None,
            credential_attributes: Vec::new(),
            outer_attributes: Vec::new(),
        }
    }
}

/// vCard draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VCardDraft {
    pub formatted_name: String,
    pub organization: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub url: Option<String>,
}

/// Bluetooth LE out-of-band draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BluetoothDraft {
    pub address: Option<([u8; 6], LeAddressType)>,
    pub role: Option<LeRole>,
    pub name: Option<String>,
    pub appearance: Option<u16>,
    pub flags: Option<u8>,
    pub extra: Vec<(u8, Vec<u8>)>,
}

/// DPP carrier form.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DppCarrier {
    #[default]
    Uri,
    Media,
}

impl DppCarrier {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Uri => "URI record",
            Self::Media => "Media record",
        }
    }
}

/// Wi-Fi Easy Connect draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DppDraft {
    pub uri: String,
    pub carrier: DppCarrier,
}

/// Smart Poster draft. Extra titles beyond the form fields and unknown nested
/// records are preserved on edit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SmartPosterDraft {
    pub uri: String,
    pub titles: Vec<TextRecord>,
    pub action: Option<SmartPosterAction>,
    pub additional: Vec<NdefRecord>,
}

/// Raw record draft. This is the lossless fallback for any TNF the typed forms
/// do not cover.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawDraft {
    pub tnf: Tnf,
    pub record_type: Vec<u8>,
    pub id: Vec<u8>,
    pub payload: Vec<u8>,
}

/// One editable record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordDraft {
    Text(TextDraft),
    Uri(UriDraft),
    Media(MediaDraft),
    Wifi(WifiDraft),
    VCard(VCardDraft),
    BluetoothLe(BluetoothDraft),
    Dpp(DppDraft),
    SmartPoster(SmartPosterDraft),
    Raw(RawDraft),
}

impl RecordDraft {
    pub const fn kind(&self) -> RecordKind {
        match self {
            Self::Text(_) => RecordKind::Text,
            Self::Uri(_) => RecordKind::Uri,
            Self::Media(_) => RecordKind::Media,
            Self::Wifi(_) => RecordKind::Wifi,
            Self::VCard(_) => RecordKind::VCard,
            Self::BluetoothLe(_) => RecordKind::BluetoothLe,
            Self::Dpp(_) => RecordKind::Dpp,
            Self::SmartPoster(_) => RecordKind::SmartPoster,
            Self::Raw(_) => RecordKind::Raw,
        }
    }

    /// Decode a logical record. Recognized templates become typed drafts;
    /// everything else, including malformed known types, stays raw.
    ///
    /// A typed draft is accepted only when re-encoding it produces the exact
    /// same record. This keeps record IDs and template fields the form does
    /// not expose (for example unknown vCard properties) lossless.
    pub fn from_record(record: &NdefRecord) -> Self {
        if !record.id().is_empty() {
            return raw_draft(record);
        }
        if record.tnf() == Tnf::WellKnown {
            match record.record_type() {
                b"T" => {
                    if let Ok(decoded) = TextRecord::from_record(record) {
                        return Self::Text(TextDraft {
                            text: decoded.text,
                            language: decoded.language,
                            encoding: decoded.encoding,
                        });
                    }
                }
                b"U" => {
                    if let Ok(decoded) = UriRecord::from_record(record) {
                        return Self::Uri(UriDraft { uri: decoded.uri });
                    }
                }
                b"Sp" => {
                    if let Ok(decoded) = SmartPoster::from_record(record) {
                        return typed_or_raw(
                            record,
                            Self::SmartPoster(SmartPosterDraft {
                                uri: decoded.uri,
                                titles: decoded.titles,
                                action: decoded.action,
                                additional: decoded.additional,
                            }),
                        );
                    }
                }
                _ => {}
            }
        }
        if record.tnf() == Tnf::Media {
            match record.record_type() {
                b"application/vnd.wfa.wsc" => {
                    if let Ok(decoded) = WifiRecord::from_record(record) {
                        return typed_or_raw(
                            record,
                            Self::Wifi(WifiDraft {
                                ssid: decoded.ssid,
                                authentication: decoded.authentication.bits(),
                                encryption: decoded.encryption.bits(),
                                network_key: decoded.network_key,
                                mac_address: decoded.mac_address,
                                credential_attributes: decoded.credential_attributes,
                                outer_attributes: decoded.outer_attributes,
                            }),
                        );
                    }
                }
                b"application/vnd.bluetooth.le.oob" => {
                    if let Ok(decoded) = BluetoothLeRecord::from_record(record) {
                        return typed_or_raw(
                            record,
                            Self::BluetoothLe(BluetoothDraft {
                                address: decoded.address,
                                role: decoded.role,
                                name: decoded.name,
                                appearance: decoded.appearance,
                                flags: decoded.flags,
                                extra: decoded.extra,
                            }),
                        );
                    }
                }
                b"text/vcard" | b"text/x-vcard" => {
                    if let Ok(decoded) = VCard::from_record(record) {
                        return typed_or_raw(
                            record,
                            Self::VCard(VCardDraft {
                                formatted_name: decoded.formatted_name,
                                organization: decoded.organization,
                                phone: decoded.phone,
                                email: decoded.email,
                                url: decoded.url,
                            }),
                        );
                    }
                }
                b"application/vnd.wfa.dpp" => {
                    if let Ok(decoded) = DppBootstrap::from_media_record(record) {
                        return typed_or_raw(
                            record,
                            Self::Dpp(DppDraft {
                                uri: decoded.uri().to_owned(),
                                carrier: DppCarrier::Media,
                            }),
                        );
                    }
                }
                // Generic media records keep an editable MIME form. Known
                // template types that failed to decode stay raw above.
                _ => {
                    return typed_or_raw(
                        record,
                        Self::Media(MediaDraft {
                            media_type: record.record_type().to_vec(),
                            payload: record.payload().to_vec(),
                        }),
                    );
                }
            }
        }
        raw_draft(record)
    }

    /// Encode this draft through the NDEF codec.
    pub fn to_record(&self) -> nfc_core::Result<NdefRecord> {
        match self {
            Self::Text(draft) => TextRecord::with_encoding(
                draft.text.clone(),
                draft.language.clone(),
                draft.encoding,
            )?
            .to_record(),
            Self::Uri(draft) => UriRecord::new(draft.uri.clone())?.to_record(),
            Self::Media(draft) => {
                NdefRecord::media(draft.media_type.clone(), draft.payload.clone())
            }
            Self::Wifi(draft) => {
                let mut record = WifiRecord::new(
                    draft.ssid.clone(),
                    nfc_ndef::WifiAuthentication::from_bits(draft.authentication)?,
                    nfc_ndef::WifiEncryption::from_bits(draft.encryption)?,
                )?;
                if let Some(key) = &draft.network_key {
                    record = record.with_network_key(key.clone())?;
                }
                if let Some(address) = draft.mac_address {
                    record = record.with_mac_address(address)?;
                }
                record.credential_attributes = draft.credential_attributes.clone();
                record.outer_attributes = draft.outer_attributes.clone();
                record.to_record()
            }
            Self::VCard(draft) => {
                let mut card = VCard::new(draft.formatted_name.clone())?;
                card.organization = draft.organization.clone();
                card.phone = draft.phone.clone();
                card.email = draft.email.clone();
                card.url = draft.url.clone();
                card.to_record()
            }
            Self::BluetoothLe(draft) => BluetoothLeRecord {
                address: draft.address,
                role: draft.role,
                name: draft.name.clone(),
                appearance: draft.appearance,
                flags: draft.flags,
                extra: draft.extra.clone(),
            }
            .to_record(),
            Self::Dpp(draft) => {
                let dpp = DppBootstrap::new(draft.uri.clone())?;
                match draft.carrier {
                    DppCarrier::Uri => dpp.to_uri_record(),
                    DppCarrier::Media => dpp.to_media_record(),
                }
            }
            Self::SmartPoster(draft) => {
                let mut poster = SmartPoster::new(draft.uri.clone())?;
                for title in &draft.titles {
                    poster = poster.with_title(title.clone());
                }
                if let Some(action) = draft.action {
                    poster = poster.with_action(action);
                }
                poster.additional = draft.additional.clone();
                poster.to_record()
            }
            Self::Raw(draft) => NdefRecord::new(
                draft.tnf,
                draft.record_type.clone(),
                draft.id.clone(),
                draft.payload.clone(),
            ),
        }
    }

    /// Type label used in the record table.
    pub fn label(&self) -> String {
        match self {
            Self::Text(draft) => format!("Text · {}", draft.language),
            Self::Uri(_) => "URI".to_owned(),
            Self::Media(draft) => format!("Media · {}", text_or_hex(&draft.media_type)),
            Self::Wifi(draft) => format!("Wi-Fi · {}", text_or_hex(&draft.ssid)),
            Self::VCard(draft) => format!("vCard · {}", draft.formatted_name),
            Self::BluetoothLe(_) => "Bluetooth LE".to_owned(),
            Self::Dpp(_) => "Wi-Fi DPP".to_owned(),
            Self::SmartPoster(_) => "Smart Poster".to_owned(),
            Self::Raw(draft) => raw_label(draft.tnf, &draft.record_type),
        }
    }

    /// Short, single-line preview for the table and inspector.
    pub fn preview(&self) -> String {
        match self {
            Self::Text(draft) => preview_text(&draft.text),
            Self::Uri(draft) => preview_text(&draft.uri),
            Self::Media(draft) => preview_bytes(&draft.payload),
            Self::Wifi(draft) => {
                let mut parts = vec![format!("SSID {}", preview_bytes(&draft.ssid))];
                if draft.network_key.is_some() {
                    parts.push("network key set".to_owned());
                }
                parts.join(" · ")
            }
            Self::VCard(draft) => {
                let mut parts = vec![draft.formatted_name.clone()];
                if let Some(organization) = &draft.organization {
                    parts.push(organization.clone());
                }
                preview_text(&parts.join(" · "))
            }
            Self::BluetoothLe(draft) => {
                let mut parts = Vec::new();
                if let Some((address, _)) = draft.address {
                    parts.push(format_hex(&address, ":"));
                }
                if let Some(name) = &draft.name {
                    parts.push(name.clone());
                }
                if parts.is_empty() {
                    "Bluetooth LE out-of-band".to_owned()
                } else {
                    parts.join(" · ")
                }
            }
            Self::Dpp(draft) => preview_text(&draft.uri),
            Self::SmartPoster(draft) => preview_text(&draft.uri),
            Self::Raw(draft) => preview_bytes(&draft.payload),
        }
    }

    /// Exact encoded size of this record in a one-record message.
    pub fn encoded_len(&self) -> nfc_core::Result<usize> {
        NdefMessage::single(self.to_record()?)?.encoded_len()
    }
}

fn raw_draft(record: &NdefRecord) -> RecordDraft {
    RecordDraft::Raw(RawDraft {
        tnf: record.tnf(),
        record_type: record.record_type().to_vec(),
        id: record.id().to_vec(),
        payload: record.payload().to_vec(),
    })
}

/// Keep the typed draft only if it reproduces the original record byte for
/// byte. Temporary ordering differences fall back to the raw form.
fn typed_or_raw(record: &NdefRecord, draft: RecordDraft) -> RecordDraft {
    match draft.to_record() {
        Ok(encoded) if encoded == *record => draft,
        _ => raw_draft(record),
    }
}

/// One table row derived from the editor model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordRow {
    pub index: usize,
    pub kind: String,
    pub preview: String,
    pub size: String,
    pub valid: bool,
}

/// A message being edited.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NdefEditor {
    records: Vec<RecordDraft>,
    selected: Option<usize>,
    baseline: Option<NdefMessage>,
}

impl NdefEditor {
    pub fn records(&self) -> &[RecordDraft] {
        &self.records
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn select(&mut self, index: Option<usize>) {
        self.selected = index.filter(|index| *index < self.records.len());
    }

    /// Replace the editor with the message stored on the card. `None` is a
    /// formatted card with an empty message.
    pub fn load_message(&mut self, message: Option<&NdefMessage>) {
        self.records = message
            .map(|message| {
                message
                    .records()
                    .iter()
                    .map(RecordDraft::from_record)
                    .collect()
            })
            .unwrap_or_default();
        self.selected = (!self.records.is_empty()).then_some(0);
        self.baseline = message.cloned();
    }

    /// Replace the editor with a raw `.ndef` file.
    pub fn load_file(&mut self, path: &Path) -> Result<(), String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let message = NdefMessage::decode(&bytes)
            .map_err(|error| format!("{} is not a valid NDEF message: {error}", path.display()))?;
        self.load_message(Some(&message));
        Ok(())
    }

    /// Keep the current records but forget the card/file baseline. Used by the
    /// "new message" action.
    pub fn clear(&mut self) {
        self.records.clear();
        self.selected = None;
        self.baseline = None;
    }

    /// The card now stores exactly the current editor message.
    pub fn mark_written(&mut self, message: &NdefMessage) {
        self.baseline = Some(message.clone());
    }

    /// The card NDEF area was formatted to an empty message.
    pub fn mark_formatted(&mut self) {
        self.load_message(None);
    }

    pub fn is_dirty(&self) -> bool {
        if self.records.is_empty() {
            return self.baseline.is_some();
        }
        match self.message() {
            Ok(message) => self.baseline.as_ref() != Some(&message),
            Err(_) => true,
        }
    }

    /// Validate and build the logical message.
    pub fn message(&self) -> Result<NdefMessage, String> {
        if self.records.is_empty() {
            return Err("An NDEF message needs at least one record.".to_owned());
        }
        let records = self
            .records
            .iter()
            .enumerate()
            .map(|(index, draft)| {
                draft
                    .to_record()
                    .map_err(|error| format!("Record {} is invalid: {error}", index + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        NdefMessage::new(records).map_err(|error| error.to_string())
    }

    pub fn encoded(&self) -> Result<Vec<u8>, String> {
        self.message()
            .and_then(|message| message.encode().map_err(|error| error.to_string()))
    }

    pub fn encoded_len(&self) -> Result<usize, String> {
        self.message()
            .and_then(|message| message.encoded_len().map_err(|error| error.to_string()))
    }

    /// `"N records · M bytes"`, or the first model error.
    pub fn summary(&self) -> String {
        if self.records.is_empty() {
            return "An NDEF message needs at least one record.".to_owned();
        }
        match self.encoded_len() {
            Ok(bytes) => format!("{} record(s) · {} bytes", self.records.len(), bytes),
            Err(error) => error,
        }
    }

    pub fn rows(&self) -> Vec<RecordRow> {
        self.records
            .iter()
            .enumerate()
            .map(|(index, draft)| {
                let encoded = draft.encoded_len();
                RecordRow {
                    index,
                    kind: draft.label(),
                    preview: draft.preview(),
                    size: encoded
                        .as_ref()
                        .map(|bytes| format!("{bytes} B"))
                        .unwrap_or_else(|_| "—".to_owned()),
                    valid: encoded.is_ok(),
                }
            })
            .collect()
    }

    pub fn add(&mut self, draft: RecordDraft) -> usize {
        self.records.push(draft);
        let index = self.records.len() - 1;
        self.selected = Some(index);
        index
    }

    pub fn replace(&mut self, index: usize, draft: RecordDraft) -> bool {
        let Some(record) = self.records.get_mut(index) else {
            return false;
        };
        *record = draft;
        self.selected = Some(index);
        true
    }

    pub fn remove(&mut self, index: usize) -> bool {
        if index >= self.records.len() {
            return false;
        }
        self.records.remove(index);
        self.selected = if self.records.is_empty() {
            None
        } else {
            Some(index.min(self.records.len() - 1))
        };
        true
    }

    pub fn move_up(&mut self, index: usize) -> bool {
        if index == 0 || index >= self.records.len() {
            return false;
        }
        self.records.swap(index - 1, index);
        self.selected = Some(index - 1);
        true
    }

    pub fn move_down(&mut self, index: usize) -> bool {
        if index + 1 >= self.records.len() {
            return false;
        }
        self.records.swap(index, index + 1);
        self.selected = Some(index + 1);
        true
    }

    /// Encoded NDEF Message TLV, capped at `limit` displayed bytes.
    pub fn tlv_preview(&self, limit: usize) -> Result<(usize, String), String> {
        let message = self.message()?;
        let tlv = Tlv::ndef(&message).map_err(|error| error.to_string())?;
        let bytes = tlv.encode().map_err(|error| error.to_string())?;
        Ok((bytes.len(), hex_preview(&bytes, limit)))
    }
}

/// The TNF values the raw form can encode. `Unchanged` is chunk-only.
pub const RAW_TNFS: [Tnf; 6] = [
    Tnf::Empty,
    Tnf::WellKnown,
    Tnf::Media,
    Tnf::AbsoluteUri,
    Tnf::External,
    Tnf::Unknown,
];

pub const fn tnf_label(tnf: Tnf) -> &'static str {
    match tnf {
        Tnf::Empty => "Empty",
        Tnf::WellKnown => "Well-known",
        Tnf::Media => "Media",
        Tnf::AbsoluteUri => "Absolute URI",
        Tnf::External => "External",
        Tnf::Unknown => "Unknown",
        Tnf::Unchanged => "Chunk",
    }
}

fn raw_label(tnf: Tnf, record_type: &[u8]) -> String {
    match tnf {
        Tnf::Empty => "Empty".to_owned(),
        Tnf::WellKnown => format!("Well-known · {}", text_or_hex(record_type)),
        Tnf::Media => format!("Media · {}", text_or_hex(record_type)),
        Tnf::AbsoluteUri => format!("Absolute URI · {}", text_or_hex(record_type)),
        Tnf::External => format!("External · {}", text_or_hex(record_type)),
        Tnf::Unknown => "Unknown".to_owned(),
        Tnf::Unchanged => "Chunk".to_owned(),
    }
}

/// Render UTF-8 bytes as text, otherwise as compact uppercase hex.
pub fn text_or_hex(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) if !text.is_empty() => text.to_owned(),
        _ if bytes.is_empty() => "—".to_owned(),
        _ => format_hex(bytes, " "),
    }
}

pub fn format_hex(bytes: &[u8], separator: &str) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(separator)
}

/// Parse hexadecimal with optional spaces, colons, or hyphens.
pub fn parse_hex(text: &str, field: &str) -> Result<Vec<u8>, String> {
    let compact: String = text
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, ':' | '-'))
        .collect();
    if !compact.len().is_multiple_of(2) {
        return Err(format!("{field} must contain whole hexadecimal bytes"));
    }
    let mut bytes = Vec::with_capacity(compact.len() / 2);
    for index in (0..compact.len()).step_by(2) {
        let pair = &compact[index..index + 2];
        bytes.push(
            u8::from_str_radix(pair, 16)
                .map_err(|_| format!("{field} has an invalid hexadecimal byte {pair:?}"))?,
        );
    }
    Ok(bytes)
}

pub fn parse_u8_hex(text: &str, field: &str) -> Result<u8, String> {
    let bytes = parse_hex(text, field)?;
    match bytes.as_slice() {
        [value] => Ok(*value),
        _ => Err(format!("{field} must be one byte")),
    }
}

pub fn parse_u16_hex(text: &str, field: &str) -> Result<u16, String> {
    let bytes = parse_hex(text, field)?;
    match bytes.as_slice() {
        [high, low] => Ok(u16::from_be_bytes([*high, *low])),
        _ => Err(format!("{field} must be two bytes")),
    }
}

pub fn hex_preview(bytes: &[u8], limit: usize) -> String {
    let shown = bytes
        .iter()
        .take(limit)
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if bytes.len() > limit {
        format!("{shown} …")
    } else {
        shown
    }
}

fn preview_text(text: &str) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 160 {
        let mut preview = one_line.chars().take(160).collect::<String>();
        preview.push('…');
        preview
    } else {
        one_line
    }
}

fn preview_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text)
            if !text.is_empty()
                && !text.chars().any(|character| {
                    character.is_control() && character != '\n' && character != '\t'
                }) =>
        {
            preview_text(text)
        }
        _ if bytes.is_empty() => "—".to_owned(),
        _ => hex_preview(bytes, 64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_message() -> NdefMessage {
        NdefMessage::new(vec![
            TextRecord::new("Hello", "en")
                .expect("text")
                .to_record()
                .expect("record"),
            UriRecord::new("https://example.com/a")
                .expect("uri")
                .to_record()
                .expect("record"),
        ])
        .expect("message")
    }

    #[test]
    fn text_and_uri_round_trip() {
        for draft in [
            RecordDraft::Text(TextDraft {
                text: "Hello".to_owned(),
                language: "en".to_owned(),
                encoding: TextEncoding::Utf8,
            }),
            RecordDraft::Text(TextDraft {
                text: "Grüße".to_owned(),
                language: "de".to_owned(),
                encoding: TextEncoding::Utf16,
            }),
            RecordDraft::Uri(UriDraft {
                uri: "https://example.com/a".to_owned(),
            }),
        ] {
            let record = draft.to_record().expect("record");
            assert_eq!(RecordDraft::from_record(&record), draft);
        }
    }

    #[test]
    fn wifi_vcard_bluetooth_dpp_and_poster_round_trip() {
        let wifi = WifiDraft {
            ssid: b"Foundry".to_vec(),
            authentication: 0x0020,
            encryption: 0x0008,
            network_key: Some(b"secret-pass".to_vec()),
            mac_address: Some([0x00, 0x11, 0x22, 0x33, 0x44, 0x55]),
            credential_attributes: vec![(0x1234, vec![1, 2])],
            outer_attributes: vec![(0x4321, vec![3])],
        };
        let record = RecordDraft::Wifi(wifi.clone()).to_record().expect("wifi");
        assert_eq!(RecordDraft::from_record(&record), RecordDraft::Wifi(wifi));

        let vcard = VCardDraft {
            formatted_name: "Ada Lovelace".to_owned(),
            organization: Some("Analytical Engines".to_owned()),
            phone: Some("+1234".to_owned()),
            email: Some("ada@example.com".to_owned()),
            url: Some("https://example.com".to_owned()),
        };
        let record = RecordDraft::VCard(vcard.clone())
            .to_record()
            .expect("vcard");
        assert_eq!(RecordDraft::from_record(&record), RecordDraft::VCard(vcard));

        let bluetooth = BluetoothDraft {
            address: Some(([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF], LeAddressType::Random)),
            role: Some(LeRole::PeripheralOnly),
            name: Some("Foundry".to_owned()),
            appearance: Some(0x0341),
            flags: Some(0x06),
            extra: vec![(0x12, vec![9])],
        };
        let record = RecordDraft::BluetoothLe(bluetooth.clone())
            .to_record()
            .expect("ble");
        assert_eq!(
            RecordDraft::from_record(&record),
            RecordDraft::BluetoothLe(bluetooth)
        );

        let dpp = DppDraft {
            uri: "DPP:C:81/1;M:001122334455;K:303132333435363738393A3B3C3D3E3F;;".to_owned(),
            carrier: DppCarrier::Media,
        };
        let record = RecordDraft::Dpp(dpp.clone()).to_record().expect("dpp");
        assert_eq!(RecordDraft::from_record(&record), RecordDraft::Dpp(dpp));

        let poster = SmartPosterDraft {
            uri: "https://example.com".to_owned(),
            titles: vec![TextRecord::new("Example", "en").expect("title")],
            action: Some(SmartPosterAction::Execute),
            additional: vec![NdefRecord::media(b"text/plain", b"note").expect("extra")],
        };
        let record = RecordDraft::SmartPoster(poster.clone())
            .to_record()
            .expect("poster");
        assert_eq!(
            RecordDraft::from_record(&record),
            RecordDraft::SmartPoster(poster)
        );
    }

    #[test]
    fn generic_media_records_use_the_mime_form() {
        let record =
            NdefRecord::media(b"application/json", br#"{"ok":true}"#).expect("media record");
        let draft = RecordDraft::from_record(&record);
        assert_eq!(draft.kind(), RecordKind::Media);
        assert_eq!(draft.to_record().expect("media"), record);
    }

    #[test]
    fn record_ids_and_unknown_vcard_fields_stay_raw() {
        let with_id = NdefRecord::new(Tnf::WellKnown, b"T", vec![7], b"\x02enHi").expect("text");
        let draft = RecordDraft::from_record(&with_id);
        assert_eq!(draft.kind(), RecordKind::Raw);
        assert_eq!(draft.to_record().expect("raw"), with_id);

        let vcard = NdefRecord::media(
            b"text/vcard",
            b"BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ada\r\nNOTE:keep me\r\nEND:VCARD\r\n".to_vec(),
        )
        .expect("vcard");
        let draft = RecordDraft::from_record(&vcard);
        assert_eq!(draft.kind(), RecordKind::Raw);
        assert_eq!(draft.to_record().expect("raw"), vcard);
    }

    #[test]
    fn unknown_and_malformed_records_stay_raw() {
        let external = NdefRecord::external(b"example.com:tag", b"value").expect("external");
        let draft = RecordDraft::from_record(&external);
        assert_eq!(draft.kind(), RecordKind::Raw);
        assert_eq!(draft.to_record().expect("raw"), external);

        let malformed = NdefRecord::well_known(b"T", Vec::new()).expect("malformed text");
        let draft = RecordDraft::from_record(&malformed);
        assert_eq!(draft.kind(), RecordKind::Raw);
        assert_eq!(draft.to_record().expect("raw"), malformed);
    }

    #[test]
    fn editor_tracks_baseline_and_selection() {
        let message = text_message();
        let mut editor = NdefEditor::default();
        editor.load_message(Some(&message));
        assert!(!editor.is_dirty());
        assert_eq!(editor.records().len(), 2);
        assert_eq!(editor.selected(), Some(0));

        editor.add(RecordDraft::Media(MediaDraft {
            media_type: b"application/json".to_vec(),
            payload: b"{}".to_vec(),
        }));
        assert!(editor.is_dirty());
        assert_eq!(editor.selected(), Some(2));
        assert!(editor.move_up(2));
        assert_eq!(editor.selected(), Some(1));
        assert!(editor.move_down(1));
        assert_eq!(editor.selected(), Some(2));
        assert!(editor.remove(2));
        assert_eq!(editor.records().len(), 2);
        assert!(!editor.is_dirty());

        editor.remove(0);
        assert!(editor.is_dirty());
        editor.mark_written(&text_message());
        assert!(editor.is_dirty());
        editor.load_message(Some(&text_message()));
        editor.mark_written(&text_message());
        assert!(!editor.is_dirty());
    }

    #[test]
    fn empty_editor_rejects_a_message() {
        let editor = NdefEditor::default();
        assert!(editor.message().is_err());
        assert!(!editor.is_dirty());
        assert_eq!(editor.rows().len(), 0);
    }

    #[test]
    fn tlv_preview_includes_the_ndef_tag_and_length() {
        let mut editor = NdefEditor::default();
        editor.load_message(Some(&text_message()));
        let (len, preview) = editor.tlv_preview(4).expect("TLV");
        assert!(len > 4);
        assert!(preview.starts_with("03 "));
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn file_loading_rejects_trailing_bytes() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("message.ndef");
        let mut bytes = text_message().encode().expect("encode");
        std::fs::write(&path, &bytes).expect("write");
        let mut editor = NdefEditor::default();
        editor.load_file(&path).expect("load");
        assert!(!editor.is_dirty());

        bytes.push(0);
        std::fs::write(&path, &bytes).expect("write");
        assert!(editor.load_file(&path).is_err());
        assert_eq!(editor.records().len(), 2);
    }

    #[test]
    fn hex_parsing_accepts_separators_and_rejects_odd_input() {
        assert_eq!(parse_hex("04:E1 9C", "UID").expect("hex"), [4, 0xE1, 0x9C]);
        assert!(parse_hex("0", "UID").is_err());
        assert_eq!(parse_u16_hex("00 20", "bits").expect("u16"), 0x20);
        assert!(parse_u8_hex("00 20", "byte").is_err());
    }
}
