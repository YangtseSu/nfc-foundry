//! Controlled Amiibo / NTAG215 read, decrypt, encrypt, and write workflows.
//!
//! The user supplies the Nintendo master keys and the dump. The application
//! loads them for one operation, verifies both signatures, and never stores or
//! logs key bytes. Writes are Advanced, backup-first, limited to a complete
//! unlocked NTAG215, and re-signed for the selected tag's UID.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nfc_amiibo::{AmiiboIdentity, AmiiboKeys, AmiiboPlain, AmiiboTag, pack, unpack};
use nfc_core::{CancelToken, CardInfo, NfcError, PAGE_SIZE, Result};
use nfc_type2::{
    Configuration, Type2Auth, Type2AuthSession, Type2Image, Type2Product, WritePolicy,
    check_writable, read_tag,
};

use crate::classic::backup::{BackupRecord, BackupStore};
use crate::classic::write::check_expected_card;
use crate::fsutil::{read_bytes, write_atomic};
use crate::operation::Operation;
use crate::type2::Type2ReadRequest;

const PLAN_TTL: Duration = Duration::from_secs(120);
const AMIIBO_PACK: [u8; 2] = [0x80, 0x80];

/// The source representation for an Amiibo write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AmiiboSourceFormat {
    /// A raw NTAG215 dump (532, 540, or more bytes).
    Raw,
    /// An internal plaintext dump (520 or 540 bytes).
    Plain,
}

/// Load the 160-byte combined key file.
pub fn load_keys(path: &Path) -> Result<AmiiboKeys> {
    AmiiboKeys::from_combined(&read_bytes(path)?)
}

/// Load the two 80-byte key files.
pub fn load_key_parts(data: &Path, tag: &Path) -> Result<AmiiboKeys> {
    AmiiboKeys::from_parts(&read_bytes(data)?, &read_bytes(tag)?)
}

/// The public Amiibo password algorithm for a seven-byte NTAG215 UID.
pub fn password_for_uid(uid: &[u8; 7]) -> [u8; 4] {
    [
        uid[1] ^ uid[3] ^ 0xAA,
        uid[2] ^ uid[4] ^ 0x55,
        uid[3] ^ uid[5] ^ 0xAA,
        uid[4] ^ uid[6] ^ 0x55,
    ]
}

fn canonical_tail(password: [u8; 4], pack: [u8; 2]) -> [u8; 20] {
    [
        0x01,
        0x00,
        0x0F,
        0xBD, // dynamic lock + RFUI
        0x00,
        0x00,
        0x00,
        0x04, // CFG0: AUTH0 = 04
        0x5F,
        0x00,
        0x00,
        0x00, // CFG1: ACCESS = 5F
        password[0],
        password[1],
        password[2],
        password[3],
        pack[0],
        pack[1],
        0x00,
        0x00,
    ]
}

/// A read-only request for the selected NTAG215.
pub struct AmiiboReadRequest {
    pub auth: Option<Type2Auth>,
    pub keys: Option<AmiiboKeys>,
    pub cancel: CancelToken,
}

impl AmiiboReadRequest {
    pub fn new(cancel: CancelToken) -> Self {
        Self {
            auth: None,
            keys: None,
            cancel,
        }
    }

    pub fn auth(mut self, auth: Option<Type2Auth>) -> Self {
        self.auth = auth;
        self
    }

    pub fn keys(mut self, keys: Option<AmiiboKeys>) -> Self {
        self.keys = keys;
        self
    }
}

/// A live Amiibo read. `plain` and `identity` are present only when key files
/// were supplied and both signatures verified.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AmiiboReadResult {
    pub card: CardInfo,
    pub image: Type2Image,
    pub raw: AmiiboTag,
    pub plain: Option<AmiiboPlain>,
    pub identity: Option<AmiiboIdentity>,
    pub warnings: Vec<String>,
}

/// One file conversion request.
pub struct AmiiboFileRequest {
    pub keys: AmiiboKeys,
    pub input: PathBuf,
    pub output: Option<PathBuf>,
    pub cancel: CancelToken,
}

/// Result of decrypting or encrypting a file.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AmiiboFileResult {
    pub source_uid: [u8; 7],
    pub identity: AmiiboIdentity,
    pub input_bytes: usize,
    pub output_bytes: usize,
    pub output: Option<PathBuf>,
    pub warnings: Vec<String>,
}

/// A write request. The source is re-signed for the selected tag's UID.
pub struct AmiiboWriteRequest {
    pub keys: AmiiboKeys,
    pub source: PathBuf,
    pub format: AmiiboSourceFormat,
    pub auth: Option<Type2Auth>,
    pub advanced: bool,
    pub backup: BackupStore,
    pub cancel: CancelToken,
}

/// A preflight plan. Execution consumes `token`; the plan carries no printable
/// password or PACK.
#[derive(Clone)]
pub struct AmiiboWritePlan {
    token: u64,
    card: CardInfo,
    target_raw: [u8; 540],
    expected: [u8; 540],
    plain: AmiiboPlain,
    keys: AmiiboKeys,
    source_uid: [u8; 7],
    target_uid: [u8; 7],
    identity: AmiiboIdentity,
    pages: Vec<u8>,
    auth: Option<Type2Auth>,
    backup_path: PathBuf,
    warnings: Vec<String>,
    expires: Instant,
}

impl AmiiboWritePlan {
    pub const fn token(&self) -> u64 {
        self.token
    }

    pub fn card(&self) -> &CardInfo {
        &self.card
    }

    pub const fn source_uid(&self) -> [u8; 7] {
        self.source_uid
    }

    pub const fn target_uid(&self) -> [u8; 7] {
        self.target_uid
    }

    pub const fn identity(&self) -> AmiiboIdentity {
        self.identity
    }

    pub fn pages(&self) -> &[u8] {
        &self.pages
    }

    pub fn backup_path(&self) -> &Path {
        &self.backup_path
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn expected_page(&self, page: u8) -> Result<[u8; 4]> {
        page_data(&self.expected, page)
    }

    pub fn expires(&self) -> Instant {
        self.expires
    }

    pub fn credential_policy(&self) -> &'static str {
        "PWD and PACK are derived from the target UID; PACK is 8080"
    }
}

impl fmt::Debug for AmiiboWritePlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AmiiboWritePlan")
            .field("token", &self.token)
            .field("target_uid", &self.target_uid)
            .field("source_uid", &self.source_uid)
            .field("identity", &self.identity)
            .field("pages", &self.pages.len())
            .field("backup_path", &self.backup_path)
            .finish()
    }
}

pub(crate) struct PendingAmiiboWrite {
    pub plan: AmiiboWritePlan,
    pub expires: Instant,
}

/// Execute a preflight plan. The token is single-use.
pub struct AmiiboWriteExecution {
    pub token: u64,
    pub confirm: bool,
    pub cancel: CancelToken,
}

impl AmiiboWriteExecution {
    pub fn new(token: u64) -> Self {
        Self {
            token,
            confirm: false,
            cancel: CancelToken::new(),
        }
    }

    pub const fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of a verified Amiibo write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AmiiboWriteResult {
    pub card: CardInfo,
    pub image: Type2Image,
    pub target_uid: [u8; 7],
    pub source_uid: [u8; 7],
    pub identity: AmiiboIdentity,
    pub written_pages: usize,
    pub backup: BackupRecord,
    pub warnings: Vec<String>,
}

fn normalize_raw(bytes: &[u8]) -> Result<([u8; 540], Vec<String>)> {
    let mut warnings = Vec::new();
    let mut raw = [0u8; 540];
    match bytes.len() {
        540 => raw.copy_from_slice(bytes),
        532 => {
            raw[..532].copy_from_slice(bytes);
            warnings.push(
                "the source omitted the write-only PWD/PACK pages; zeros were used".to_owned(),
            );
        }
        length if length > 540 => {
            raw.copy_from_slice(&bytes[..540]);
            warnings.push(format!(
                "the source is {length} bytes; only the first 540 NTAG215 bytes were used"
            ));
        }
        length => {
            return Err(NfcError::InvalidArgument(format!(
                "a raw Amiibo dump is at least 532 bytes; got {length}"
            )));
        }
    }
    Ok((raw, warnings))
}

fn source_plain(request: &AmiiboWriteRequest) -> Result<(AmiiboPlain, Vec<String>)> {
    let bytes = read_bytes(&request.source)?;
    match request.format {
        AmiiboSourceFormat::Raw => {
            let (raw, warnings) = normalize_raw(&bytes)?;
            let tag = AmiiboTag::from_bytes(&raw)?;
            let plain = unpack(&request.keys, &tag)?;
            Ok((plain, warnings))
        }
        AmiiboSourceFormat::Plain => {
            let plain = AmiiboPlain::from_bytes(&bytes)?;
            plain.validate()?;
            Ok((plain, Vec::new()))
        }
    }
}

fn image_to_raw(image: &Type2Image) -> Result<[u8; 540]> {
    if image.layout.product != Type2Product::Ntag215 || image.layout.total_pages != 135 {
        return Err(NfcError::Unsupported(
            "Amiibo images need an NTAG215 Type 2 layout",
        ));
    }
    if !image.is_complete() {
        return Err(NfcError::VerificationFailed(
            "the NTAG215 read was partial; an Amiibo image needs all 135 pages".to_owned(),
        ));
    }
    let mut raw = [0u8; 540];
    for page in 0..135u8 {
        let data = *image
            .page(page)
            .ok_or_else(|| NfcError::VerificationFailed(format!("page {page} was not readable")))?;
        let start = usize::from(page) * PAGE_SIZE;
        raw[start..start + PAGE_SIZE].copy_from_slice(&data);
    }
    Ok(raw)
}

fn target_uid(image: &Type2Image) -> Result<[u8; 7]> {
    let page0 = image
        .page(0)
        .ok_or_else(|| NfcError::VerificationFailed("page 0 is not readable".to_owned()))?;
    let page1 = image
        .page(1)
        .ok_or_else(|| NfcError::VerificationFailed("page 1 is not readable".to_owned()))?;
    let uid = [
        page0[0], page0[1], page0[2], page1[0], page1[1], page1[2], page1[3],
    ];
    if uid[0] != 0x04 {
        return Err(NfcError::Unsupported(
            "the target UID does not use the NXP manufacturer byte 04",
        ));
    }
    if page0[3] != nfc_amiibo::bcc0(&uid) {
        return Err(NfcError::VerificationFailed(
            "the target UID page has an invalid BCC0".to_owned(),
        ));
    }
    if image.uid.as_slice() != uid {
        return Err(NfcError::CardChanged);
    }
    Ok(uid)
}

fn check_capability_container(raw: &[u8; 540]) -> Result<()> {
    const WANTED: [u8; 4] = [0xF1, 0x10, 0xFF, 0xEE];
    let current = &raw[0x0C..0x10];
    // A blank magic tag may accept the container directly. A standard NTAG215
    // container is also accepted as a candidate: a magic tag can replace it,
    // while a genuine tag ANDs the write and fails the page-3 read-back before
    // any user page is touched. Otherwise the current bytes must already
    // contain every wanted bit.
    if current.iter().all(|byte| *byte == 0) || current == [0xE1, 0x10, 0x3E, 0x00] {
        return Ok(());
    }
    for (index, wanted) in WANTED.iter().enumerate() {
        if current[index] & wanted != *wanted {
            return Err(NfcError::Unsupported(
                "the target capability container cannot become the Amiibo container F1 10 FF EE",
            ));
        }
    }
    Ok(())
}

fn filter_cc_warning(warnings: &mut Vec<String>) {
    warnings.retain(|warning| !warning.contains("no valid Type 2 capability container"));
}

/// Read the selected tag and optionally decrypt it.
pub fn read(operation: &mut Operation<'_>, request: AmiiboReadRequest) -> Result<AmiiboReadResult> {
    let cancel = request.cancel;
    let result = crate::type2::read(
        operation,
        Type2ReadRequest::new(cancel.clone()).auth(request.auth),
    )?;
    let raw = image_to_raw(&result.image)?;
    let tag = AmiiboTag::from_bytes(&raw)?;
    let (plain, identity) = match request.keys.as_ref() {
        Some(keys) => {
            let plain = unpack(keys, &tag)?;
            let identity = plain.identity()?;
            (Some(plain), Some(identity))
        }
        None => (None, None),
    };
    operation.check_card()?;
    let mut warnings = result.warnings;
    filter_cc_warning(&mut warnings);
    Ok(AmiiboReadResult {
        card: result.card,
        image: result.image,
        raw: tag,
        plain,
        identity,
        warnings,
    })
}

fn read_raw_bytes(
    keys: &AmiiboKeys,
    bytes: &[u8],
) -> Result<(AmiiboTag, AmiiboPlain, Vec<String>)> {
    let (raw, warnings) = normalize_raw(bytes)?;
    let tag = AmiiboTag::from_bytes(&raw)?;
    let plain = unpack(keys, &tag)?;
    Ok((tag, plain, warnings))
}

/// Decrypt a raw dump and write the plaintext.
pub fn decrypt_file(request: AmiiboFileRequest) -> Result<AmiiboFileResult> {
    request.cancel.check()?;
    let bytes = read_bytes(&request.input)?;
    let (tag, plain, warnings) = read_raw_bytes(&request.keys, &bytes)?;
    let identity = plain.identity()?;
    let source_uid = tag.uid()?;
    let output_bytes = plain.as_bytes().len();
    if let Some(output) = &request.output {
        write_atomic(output, plain.as_bytes())?;
    }
    request.cancel.check()?;
    Ok(AmiiboFileResult {
        source_uid,
        identity,
        input_bytes: bytes.len(),
        output_bytes,
        output: request.output,
        warnings,
    })
}

/// Encrypt and sign a plaintext dump.
pub fn encrypt_file(request: AmiiboFileRequest) -> Result<AmiiboFileResult> {
    request.cancel.check()?;
    let bytes = read_bytes(&request.input)?;
    let plain = AmiiboPlain::from_bytes(&bytes)?;
    plain.validate()?;
    let identity = plain.identity()?;
    let source_uid = plain.uid()?;
    let tag = pack(&request.keys, &plain)?;
    if let Some(output) = &request.output {
        write_atomic(output, tag.as_bytes())?;
    }
    request.cancel.check()?;
    Ok(AmiiboFileResult {
        source_uid,
        identity,
        input_bytes: bytes.len(),
        output_bytes: tag.as_bytes().len(),
        output: request.output,
        warnings: Vec::new(),
    })
}

/// Page order: CC first so a genuine tag fails before user data, then user
/// pages, write-only credentials, static lock, dynamic lock, and configuration.
fn write_order() -> Vec<u8> {
    std::iter::once(3)
        .chain(4..=129)
        .chain([133, 134, 2, 130, 131, 132])
        .collect()
}

/// Build a write plan without writing the card.
pub fn write_preflight(
    operation: &mut Operation<'_>,
    request: AmiiboWriteRequest,
) -> Result<AmiiboWritePlan> {
    if !request.advanced {
        return Err(NfcError::InvalidArgument(
            "Amiibo writes need Advanced mode".to_owned(),
        ));
    }
    request.cancel.check()?;
    let (plain, mut warnings) = source_plain(&request)?;
    plain.validate()?;
    let identity = plain.identity()?;
    let source_uid = plain.uid()?;

    let read = crate::type2::read(
        operation,
        Type2ReadRequest::new(request.cancel.clone()).auth(request.auth),
    )?;
    warnings.extend(read.warnings.iter().cloned());
    filter_cc_warning(&mut warnings);
    let target_raw = image_to_raw(&read.image)?;
    let target = target_uid(&read.image)?;
    check_capability_container(&target_raw)?;

    if let Ok(config) = Configuration::from_image(&read.image) {
        if config.cfg_locked() {
            return Err(NfcError::Unsupported(
                "the target configuration is permanently locked",
            ));
        }
        let write_pages = write_order();
        if write_pages
            .iter()
            .any(|page| config.requires_password(*page))
            && request.auth.is_none()
        {
            return Err(NfcError::AuthenticationRequired(
                "the target protects its write range; supply the current PWD and PACK".to_owned(),
            ));
        }
    }

    for page in write_order() {
        check_writable(&read.image, page, WritePolicy::advanced())?;
    }

    let password = password_for_uid(&target);
    let rebound = plain.rebind_uid(&target)?;
    let packed = pack(&request.keys, &rebound)?;

    let mut expected = *packed.as_bytes();
    expected[..8].copy_from_slice(&target_raw[..8]);
    expected[8..12].copy_from_slice(&[nfc_amiibo::bcc1(&target), 0x48, 0x0F, 0xE0]);
    expected[0x208..0x21C].copy_from_slice(&canonical_tail(password, AMIIBO_PACK));
    let expected_tag = AmiiboTag::from_bytes(&expected)?;
    let canonical = unpack(&request.keys, &expected_tag)?;

    let pages = write_order();
    let backup_path = request
        .backup
        .plan_binary_path(&format!("{}-ntag215-amiibo-write", hex_uid(&target)));
    operation.check_card()?;

    Ok(AmiiboWritePlan {
        token: 0,
        card: operation.card().clone(),
        target_raw,
        expected,
        plain: canonical,
        keys: request.keys,
        source_uid,
        target_uid: target,
        identity,
        pages,
        auth: request.auth,
        backup_path,
        warnings,
        expires: Instant::now() + PLAN_TTL,
    })
}

fn hex_uid(uid: &[u8; 7]) -> String {
    uid.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn page_from_quad(quad: &[u8; nfc_core::PAGE_QUAD_SIZE], offset: u8) -> [u8; PAGE_SIZE] {
    let start = usize::from(offset) * PAGE_SIZE;
    let mut data = [0u8; PAGE_SIZE];
    data.copy_from_slice(&quad[start..start + PAGE_SIZE]);
    data
}

fn safe_quad_start(page: u8) -> u8 {
    let start = page & !0x03;
    if start + 4 <= 135 { start } else { 131 }
}

fn page_data(expected: &[u8; 540], page: u8) -> Result<[u8; PAGE_SIZE]> {
    let start = usize::from(page) * PAGE_SIZE;
    if start + PAGE_SIZE > expected.len() {
        return Err(NfcError::InvalidArgument(format!(
            "page {page} is outside the NTAG215 image"
        )));
    }
    let mut data = [0u8; PAGE_SIZE];
    data.copy_from_slice(&expected[start..start + PAGE_SIZE]);
    Ok(data)
}

fn stop_error(plan: &AmiiboWritePlan, page: u8, error: NfcError) -> NfcError {
    NfcError::Io(format!(
        "Amiibo write stopped at page {page}: {error}; backup {}",
        plan.backup_path.display()
    ))
}

fn write_verified_page(
    reader: &mut dyn nfc_transport::Reader,
    page: u8,
    data: [u8; PAGE_SIZE],
) -> Result<()> {
    reader.write_page(page, data)?;
    let start = safe_quad_start(page);
    let quad = reader.read_page_quad(start)?;
    let observed = page_from_quad(&quad, page - start);
    if page == 130 {
        if observed[..3] != data[..3] {
            return Err(NfcError::VerificationFailed(format!(
                "page {page} read back as {observed:02X?}, expected {data:02X?}"
            )));
        }
    } else if observed != data {
        return Err(NfcError::VerificationFailed(format!(
            "page {page} read back as {observed:02X?}, expected {data:02X?}"
        )));
    }
    Ok(())
}

fn execute_write(
    operation: &mut Operation<'_>,
    plan: &AmiiboWritePlan,
    cancel: &CancelToken,
) -> Result<(CardInfo, Type2Image)> {
    let card = operation.card().clone();
    let auth = plan.auth;
    let reader = operation.reader_mut();
    let mut access = Type2AuthSession::from_option(auth.as_ref());
    let read = read_tag(reader, &card, &mut access, cancel)?;
    let current = image_to_raw(&read.image)?;
    if current != plan.target_raw {
        return Err(NfcError::VerificationFailed(
            "the target image changed after the write plan was built".to_owned(),
        ));
    }

    write_atomic(&plan.backup_path, &plan.target_raw)?;

    let password = password_for_uid(&plan.target_uid);
    let new_auth = Type2Auth::new(password, AMIIBO_PACK);
    let mut first = true;
    for &page in &plan.pages {
        if let Err(error) = cancel.check() {
            if first {
                return Err(error);
            }
            return Err(stop_error(plan, page, error));
        }
        let data = match page_data(&plan.expected, page) {
            Ok(data) => data,
            Err(error) => return Err(stop_error(plan, page, error)),
        };
        let result = if matches!(page, 133 | 134) {
            reader.write_page(page, data)
        } else {
            write_verified_page(reader, page, data)
        };
        if let Err(error) = result {
            return Err(stop_error(plan, page, error));
        }
        first = false;
        if page == 134 {
            let mut session = Type2AuthSession::new(&new_auth);
            if let Err(error) = session.authenticate(reader, cancel) {
                return Err(stop_error(plan, page, error));
            }
        }
    }

    let card_after = match reader.card_info() {
        Ok(card) => card,
        Err(error) => {
            return Err(stop_error(
                plan,
                plan.pages.last().copied().unwrap_or(0),
                error,
            ));
        }
    };
    if !card_after.same_identity(&plan.card) {
        return Err(NfcError::VerificationFailed(format!(
            "the card identity changed after the Amiibo write; backup {}",
            plan.backup_path.display()
        )));
    }

    let mut session = Type2AuthSession::new(&new_auth);
    if let Err(error) = session.authenticate(reader, cancel) {
        return Err(stop_error(plan, 134, error));
    }
    let verified = match read_tag(reader, &card_after, &mut session, cancel) {
        Ok(read) => read,
        Err(error) => return Err(stop_error(plan, 134, error)),
    };
    let raw = match image_to_raw(&verified.image) {
        Ok(raw) => raw,
        Err(error) => return Err(stop_error(plan, 134, error)),
    };
    if raw[..0x214] != plan.expected[..0x214] {
        return Err(NfcError::VerificationFailed(format!(
            "the written Amiibo image did not match the plan; backup {}",
            plan.backup_path.display()
        )));
    }
    let tag = match AmiiboTag::from_bytes(&raw) {
        Ok(tag) => tag,
        Err(error) => return Err(stop_error(plan, 134, error)),
    };
    let plain = match unpack(&plan.keys, &tag) {
        Ok(plain) => plain,
        Err(error) => return Err(stop_error(plan, 134, error)),
    };
    if plain.uid().ok() != Some(plan.target_uid)
        || plain.as_bytes()[..0x208] != plan.plain.as_bytes()[..0x208]
        || plain.identity().ok() != Some(plan.identity)
    {
        return Err(NfcError::VerificationFailed(format!(
            "the written Amiibo identity or data did not verify; backup {}",
            plan.backup_path.display()
        )));
    }
    Ok((card_after, verified.image))
}

/// Execute a preflight token. The token is consumed even when confirmation is
/// missing, so a caller must build a fresh plan.
pub fn write_execute(
    service: &mut crate::Service,
    request: AmiiboWriteExecution,
) -> Result<AmiiboWriteResult> {
    let pending = service.take_amiibo_plan(request.token)?;
    if !request.confirm {
        return Err(NfcError::InvalidArgument(
            "writing an Amiibo needs explicit confirmation".to_owned(),
        ));
    }
    if Instant::now() >= pending.expires {
        return Err(NfcError::InvalidArgument(
            "the Amiibo write plan expired; read the card again".to_owned(),
        ));
    }
    let plan = pending.plan;
    let cancel = request.cancel;
    let mut operation =
        service.begin_operation(crate::operation::OperationKind::Write, cancel.clone())?;
    check_expected_card(operation.card(), Some(&plan.card))?;
    let (card, image) = execute_write(&mut operation, &plan, &cancel)?;
    let backup = BackupRecord {
        path: plan.backup_path.clone(),
        bytes: plan.target_raw.len(),
    };
    Ok(AmiiboWriteResult {
        card,
        image,
        target_uid: plan.target_uid,
        source_uid: plan.source_uid,
        identity: plan.identity,
        written_pages: plan.pages.len(),
        backup,
        warnings: plan.warnings.clone(),
    })
}

impl crate::Service {
    /// Read the selected NTAG215 and optionally decrypt it.
    pub fn amiibo_read(&mut self, request: AmiiboReadRequest) -> Result<AmiiboReadResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Read, cancel)?;
        read(&mut operation, request)
    }

    /// Decrypt a saved raw Amiibo dump.
    pub fn amiibo_decrypt(&mut self, request: AmiiboFileRequest) -> Result<AmiiboFileResult> {
        decrypt_file(request)
    }

    /// Encrypt and sign a saved plaintext Amiibo dump.
    pub fn amiibo_encrypt(&mut self, request: AmiiboFileRequest) -> Result<AmiiboFileResult> {
        encrypt_file(request)
    }

    /// Build a write plan without changing the card.
    pub fn amiibo_write_preflight(
        &mut self,
        request: AmiiboWriteRequest,
    ) -> Result<AmiiboWritePlan> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Read, cancel)?;
        let mut plan = write_preflight(&mut operation, request)?;
        drop(operation);
        let now = Instant::now();
        self.pending_amiibo_writes
            .retain(|_, pending| pending.expires > now);
        plan.token = self.next_amiibo_token;
        self.next_amiibo_token = self.next_amiibo_token.wrapping_add(1);
        self.pending_amiibo_writes.insert(
            plan.token,
            PendingAmiiboWrite {
                expires: plan.expires,
                plan: plan.clone(),
            },
        );
        Ok(plan)
    }

    /// Execute a preflight token with a confirmation.
    pub fn amiibo_write_execute(
        &mut self,
        request: AmiiboWriteExecution,
    ) -> Result<AmiiboWriteResult> {
        write_execute(self, request)
    }

    pub(crate) fn take_amiibo_plan(&mut self, token: u64) -> Result<PendingAmiiboWrite> {
        self.pending_amiibo_writes.remove(&token).ok_or_else(|| {
            NfcError::InvalidArgument(
                "the Amiibo write token is unknown or was already used".to_owned(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_algorithm_b_matches_the_known_vectors() {
        let uid = [0x04, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        assert_eq!(password_for_uid(&uid), [0x88, 0x33, 0xCC, 0x77]);
        assert_eq!(
            password_for_uid(&[0x04, 0, 0, 0, 0, 0, 0]),
            [0xAA, 0x55, 0xAA, 0x55]
        );
    }

    #[test]
    fn raw_lengths_are_normalized() {
        let (_, warnings) = normalize_raw(&[0u8; 532]).expect("532");
        assert_eq!(warnings.len(), 1);
        let (_, warnings) = normalize_raw(&[0u8; 572]).expect("572");
        assert_eq!(warnings.len(), 1);
        assert!(normalize_raw(&[0u8; 540]).expect("540").1.is_empty());
        assert!(normalize_raw(&[0u8; 531]).is_err());
    }
}
