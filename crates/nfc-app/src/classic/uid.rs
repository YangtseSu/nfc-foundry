//! Guarded manufacturer-block (UID) writes for magic MIFARE Classic 1K cards.

use nfc_classic::{AccessBits, Dump, Layout, bcc_for_uid};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, CardType, Key, KeyType, NfcError, Result};

use crate::operation::Operation;

use super::backup::{BackupRecord, BackupStore};
use super::keys::KeyStore;
use super::plan::{WRITE_PLAN_TTL, WriteKeyTypes, WriteToken, unix_now};
use super::read::{candidate_keys, layout_for_card};
use super::workbench::dump_fingerprint;
use super::write::{backup_label, check_expected_card};

/// Supported UID length. Cascade UIDs are out of scope.
pub const UID_LEN: usize = 4;

/// One way the service can reach manufacturer block 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UidWriteBackend {
    /// ACR122U Gen1A 7-bit `0x40` / `0x43` unlock and a protected block write.
    Gen1a,
    /// Ordinary MIFARE write to block 0, permitted by sector-0 access bits.
    Standard,
}

impl UidWriteBackend {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Gen1a => "Gen1A protected writer",
            Self::Standard => "ordinary block-0 write",
        }
    }

    pub const fn note(self) -> &'static str {
        match self {
            Self::Gen1a => {
                "unlocks the card with the Gen1A backdoor; sector-0 access bits are not consulted"
            }
            Self::Standard => "uses the sector-0 access bits and the authenticated sector key",
        }
    }
}

/// Parses a four-byte UID written as compact hex, or with spaces, colons, or
/// dashes between bytes.
pub fn parse_uid(text: &str) -> Result<[u8; UID_LEN]> {
    let compact = normalize_uid_text(text);
    if compact.len() != UID_LEN * 2 {
        return Err(NfcError::InvalidArgument(format!(
            "a UID is {} hexadecimal digits, got {}",
            UID_LEN * 2,
            compact.len()
        )));
    }
    let mut uid = [0u8; UID_LEN];
    for (index, slot) in uid.iter_mut().enumerate() {
        let start = index * 2;
        *slot = u8::from_str_radix(&compact[start..start + 2], 16)
            .map_err(|_| NfcError::InvalidArgument(format!("{text:?} is not a hexadecimal UID")))?;
    }
    Ok(uid)
}

/// Uppercase compact text used for display and typed confirmation.
pub fn format_uid(uid: &[u8]) -> String {
    let mut text = String::with_capacity(uid.len() * 2);
    for byte in uid {
        text.push_str(&format!("{byte:02X}"));
    }
    text
}

/// Normalizes a typed UID for comparison: removes separators and uppercases.
pub fn normalize_uid_text(text: &str) -> String {
    text.chars()
        .filter(|character| !matches!(character, ':' | '-' | ' ' | '\t'))
        .collect::<String>()
        .to_ascii_uppercase()
}

/// Read-only request for a UID write plan.
pub struct ClassicUidPreflight<'a> {
    pub target_uid: [u8; UID_LEN],
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    /// Identity of the image the request was composed from.
    pub expected_card: Option<CardInfo>,
    /// Advanced mode is a hard precondition.
    pub advanced: bool,
    pub cancel: CancelToken,
}

impl<'a> ClassicUidPreflight<'a> {
    pub fn new(target_uid: [u8; UID_LEN], keys: &'a KeyStore) -> Self {
        Self {
            target_uid,
            keys,
            include_common_keys: true,
            expected_card: None,
            advanced: false,
            cancel: CancelToken::new(),
        }
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn expected_card(mut self, card: CardInfo) -> Self {
        self.expected_card = Some(card);
        self
    }

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// A validated UID write plan bound to one card state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicUidPlan {
    pub token: WriteToken,
    pub card: CardInfo,
    pub layout: Layout,
    pub current_uid: [u8; UID_LEN],
    pub target_uid: [u8; UID_LEN],
    pub old_block0: [u8; BLOCK_SIZE],
    pub new_block0: [u8; BLOCK_SIZE],
    pub old_trailer: [u8; BLOCK_SIZE],
    /// Key that authenticated sector 0 during the preflight. Used for the
    /// post-write read-back.
    pub sector0_key: (KeyType, Key),
    /// Key that authenticates the card for an ordinary block-0 write. `None`
    /// when only the protected writer can reach block 0.
    pub write_key: Option<(KeyType, Key)>,
    /// Key types the sector-0 access bits permit for an ordinary write.
    pub write_keys: WriteKeyTypes,
    /// Backends in execution order.
    pub backends: Vec<UidWriteBackend>,
    /// False when the target UID already matches block 0.
    pub changed: bool,
    pub warnings: Vec<String>,
    /// Unix seconds after which the token is refused.
    pub expires_at: u64,
    /// Fingerprint of the four sector-0 blocks and the card identity.
    pub card_fingerprint: u64,
}

/// Request that executes a stored plan.
pub struct ClassicUidExecution<'a> {
    pub token: WriteToken,
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    pub backup: &'a BackupStore,
    pub advanced: bool,
    pub confirmed: bool,
    pub cancel: CancelToken,
}

impl<'a> ClassicUidExecution<'a> {
    pub fn new(token: WriteToken, keys: &'a KeyStore, backup: &'a BackupStore) -> Self {
        Self {
            token,
            keys,
            include_common_keys: true,
            backup,
            advanced: false,
            confirmed: false,
            cancel: CancelToken::new(),
        }
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
    }

    pub fn confirmed(mut self, confirmed: bool) -> Self {
        self.confirmed = confirmed;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Pre-write record for a UID change. The project backup holds the complete
/// old sector 0; this record pairs it with the planned manufacturer block.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UidBackupRecord {
    pub backup: BackupRecord,
    pub old_block0: [u8; BLOCK_SIZE],
    pub new_block0: [u8; BLOCK_SIZE],
    pub old_uid: [u8; UID_LEN],
    pub target_uid: [u8; UID_LEN],
}

/// Outcome of a UID write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicUidResult {
    pub card_before: CardInfo,
    pub card_after: CardInfo,
    pub backend: Option<UidWriteBackend>,
    pub old_block0: [u8; BLOCK_SIZE],
    pub new_block0: [u8; BLOCK_SIZE],
    pub changed: bool,
    pub backup: Option<UidBackupRecord>,
    pub warnings: Vec<String>,
}

struct SectorZero {
    card: CardInfo,
    blocks: [[u8; BLOCK_SIZE]; 4],
    key: (KeyType, Key),
}

impl SectorZero {
    fn image(&self) -> Dump {
        let mut dump = Dump::new(Layout::Classic1K).with_card(self.card.clone());
        for (number, data) in self.blocks.iter().enumerate() {
            dump.blocks[number].set_read(*data);
        }
        dump
    }

    fn access(&self) -> Result<AccessBits> {
        AccessBits::decode([self.blocks[3][6], self.blocks[3][7], self.blocks[3][8]]).map_err(
            |error| {
                NfcError::InvalidArgument(format!(
                    "sector 0 trailer access bits are invalid: {error}"
                ))
            },
        )
    }

    fn current_uid(&self) -> [u8; UID_LEN] {
        self.blocks[0][0..UID_LEN]
            .try_into()
            .expect("four-byte UID slice")
    }
}

/// Builds a plan without mutating the card.
pub fn preflight(
    operation: &mut Operation<'_>,
    request: &ClassicUidPreflight<'_>,
    token: WriteToken,
) -> Result<ClassicUidPlan> {
    request.cancel.check()?;
    if !request.advanced {
        return Err(NfcError::InvalidArgument(
            "UID writes require Advanced mode".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let target_bcc = bcc_for_uid(&request.target_uid)?;
    let sector = read_sector_zero(operation, request.keys, request.include_common_keys)?;
    let current_uid = sector.current_uid();
    let expected_bcc = bcc_for_uid(&current_uid)?;
    if sector.blocks[0][4] != expected_bcc {
        return Err(NfcError::VerificationFailed(format!(
            "manufacturer block 0 BCC is {:02X}, expected {expected_bcc:02X}; the card image is inconsistent",
            sector.blocks[0][4]
        )));
    }
    let access = sector.access()?;
    let write_mask = access.groups[0].data_write_keys();
    let write_keys = WriteKeyTypes {
        a: write_mask.allows_a(),
        b: write_mask.allows_b(),
    };
    let mut write_key = None;
    if write_keys.any() {
        for (key_type, key, _source) in
            candidate_keys(request.keys, &sector.card, request.include_common_keys, 0)
        {
            if !write_keys.allows(key_type) {
                continue;
            }
            request.cancel.check()?;
            match operation.authenticate(0, key_type, key) {
                Ok(()) => {
                    write_key = Some((key_type, key));
                    break;
                }
                Err(NfcError::AuthenticationFailed) => continue,
                Err(error) => return Err(error),
            }
        }
    }

    let mut backends = Vec::new();
    if operation.has_manufacturer_writer() {
        backends.push(UidWriteBackend::Gen1a);
    }
    if write_key.is_some() {
        backends.push(UidWriteBackend::Standard);
    }
    if backends.is_empty() {
        return Err(if write_keys.any() {
            NfcError::AuthenticationFailed
        } else {
            NfcError::Unsupported(
                "sector 0 access bits deny data writes and the reader has no protected manufacturer writer",
            )
        });
    }

    let mut new_block0 = sector.blocks[0];
    new_block0[0..UID_LEN].copy_from_slice(&request.target_uid);
    new_block0[4] = target_bcc;
    let changed = new_block0 != sector.blocks[0];

    let mut warnings = Vec::new();
    if !write_keys.any() {
        warnings.push(
            "sector 0 access bits deny data writes; only the protected Gen1A writer can change this UID"
                .to_owned(),
        );
    } else if write_key.is_none() {
        warnings.push(
            "no candidate key is allowed by the sector-0 access bits; only the protected Gen1A writer is available"
                .to_owned(),
        );
    } else if backends.contains(&UidWriteBackend::Gen1a) {
        warnings.push(
            "the protected Gen1A writer is tried first; the card is reselected and block 0 is read back"
                .to_owned(),
        );
    }
    if !changed {
        warnings.push("the target UID already matches the card; no write is needed".to_owned());
    }

    Ok(ClassicUidPlan {
        token,
        card: sector.card.clone(),
        layout: Layout::Classic1K,
        current_uid,
        target_uid: request.target_uid,
        old_block0: sector.blocks[0],
        new_block0,
        old_trailer: sector.blocks[3],
        sector0_key: sector.key,
        write_key,
        write_keys,
        backends,
        changed,
        warnings,
        expires_at: unix_now() + WRITE_PLAN_TTL.as_secs(),
        card_fingerprint: dump_fingerprint(&sector.image()),
    })
}

/// Executes a plan after rechecking the token, card, and sector-0 state.
pub fn execute(
    operation: &mut Operation<'_>,
    request: ClassicUidExecution<'_>,
    plan: &ClassicUidPlan,
) -> Result<ClassicUidResult> {
    if !request.advanced {
        return Err(NfcError::InvalidArgument(
            "UID writes require Advanced mode".to_owned(),
        ));
    }
    if !request.confirmed {
        return Err(NfcError::InvalidArgument(
            "the UID write must be confirmed explicitly".to_owned(),
        ));
    }
    request.cancel.check()?;
    if plan.layout != Layout::Classic1K {
        return Err(NfcError::Unsupported(
            "UID writes support MIFARE Classic 1K only",
        ));
    }
    check_expected_card(operation.card(), Some(&plan.card))?;
    let sector = read_sector_zero(operation, request.keys, request.include_common_keys)?;
    if dump_fingerprint(&sector.image()) != plan.card_fingerprint {
        return Err(NfcError::CardChanged);
    }

    let backends: Vec<UidWriteBackend> = plan
        .backends
        .iter()
        .copied()
        .filter(|backend| match backend {
            UidWriteBackend::Gen1a => operation.has_manufacturer_writer(),
            UidWriteBackend::Standard => plan.write_key.is_some(),
        })
        .collect();
    if backends.is_empty() {
        return Err(NfcError::Unsupported(
            "no planned UID write backend is available on the selected reader",
        ));
    }

    if !plan.changed {
        return Ok(ClassicUidResult {
            card_before: sector.card.clone(),
            card_after: sector.card,
            backend: None,
            old_block0: plan.old_block0,
            new_block0: plan.new_block0,
            changed: false,
            backup: None,
            warnings: plan.warnings.clone(),
        });
    }

    let image = sector.image();
    let backup = request
        .backup
        .save(&image, &backup_label(&image, plan.layout, "uid"))?;
    let backup_path = backup.path.display().to_string();
    let record = UidBackupRecord {
        backup,
        old_block0: plan.old_block0,
        new_block0: plan.new_block0,
        old_uid: sector.current_uid(),
        target_uid: plan.target_uid,
    };
    let mut warnings = plan.warnings.clone();
    let mut last_error = None;

    for backend in backends {
        request.cancel.check()?;
        let attempt = match backend {
            UidWriteBackend::Gen1a => operation.write_manufacturer_block(plan.new_block0),
            UidWriteBackend::Standard => {
                let Some((key_type, key)) = plan.write_key else {
                    return Err(NfcError::Unsupported(
                        "the UID plan has no allowed standard write key",
                    ));
                };
                operation
                    .authenticate(0, key_type, key)
                    .and_then(|()| operation.write_block_unverified(0, plan.new_block0))
            }
        };
        match verify_after_attempt(operation, plan) {
            Ok(VerifyOutcome::Applied(card_after)) => {
                if let Err(error) = attempt {
                    warnings.push(format!(
                        "the write acknowledgement was lost ({error}); the read-back confirmed block 0"
                    ));
                }
                return Ok(ClassicUidResult {
                    card_before: sector.card,
                    card_after,
                    backend: Some(backend),
                    old_block0: plan.old_block0,
                    new_block0: plan.new_block0,
                    changed: true,
                    backup: Some(record),
                    warnings,
                });
            }
            Ok(VerifyOutcome::Unchanged) => {
                last_error = Some(match attempt {
                    Err(error) => error,
                    Ok(()) => NfcError::VerificationFailed(format!(
                        "the {} reported success but block 0 did not change",
                        backend.label()
                    )),
                });
            }
            Ok(VerifyOutcome::Different(card_after)) => {
                return Err(NfcError::VerificationFailed(format!(
                    "the {} left an unexpected card state (UID {}); the card may have changed. Backup: {backup_path}",
                    backend.label(),
                    card_after.uid_hex()
                )));
            }
            Err(error) => {
                return Err(NfcError::VerificationFailed(format!(
                    "the {} outcome is unknown because the verification read failed ({error}); the card may have changed. Backup: {backup_path}",
                    backend.label()
                )));
            }
        }
    }

    let error = last_error.unwrap_or_else(|| {
        NfcError::VerificationFailed("no UID write backend was attempted".to_owned())
    });
    if matches!(error, NfcError::Cancelled) {
        return Err(error);
    }
    Err(NfcError::VerificationFailed(format!(
        "no block-0 write succeeded; the card was not changed ({error}). Backup: {backup_path}"
    )))
}

/// Reads and authenticates sector 0. The caller must not assume any other
/// sector is readable.
fn read_sector_zero(
    operation: &mut Operation<'_>,
    keys: &KeyStore,
    include_common_keys: bool,
) -> Result<SectorZero> {
    let card = operation.card().clone();
    if card.card_type != CardType::MifareClassic1K {
        return Err(NfcError::Unsupported(
            "UID writes require a MIFARE Classic 1K card",
        ));
    }
    if card.uid.len() != UID_LEN {
        return Err(NfcError::Unsupported(
            "UID writes require a four-byte UID MIFARE Classic 1K card",
        ));
    }
    let layout = layout_for_card(&card)?;
    if layout != Layout::Classic1K {
        return Err(NfcError::Unsupported(
            "UID writes support MIFARE Classic 1K only",
        ));
    }

    let mut authenticated = None;
    for (key_type, key, _source) in candidate_keys(keys, &card, include_common_keys, 0) {
        operation.cancel().check()?;
        match operation.authenticate(0, key_type, key) {
            Ok(()) => {
                authenticated = Some((key_type, key));
                break;
            }
            Err(NfcError::AuthenticationFailed) => continue,
            Err(error) => return Err(error),
        }
    }
    let key = authenticated.ok_or(NfcError::AuthenticationFailed)?;

    let mut blocks = [[0u8; BLOCK_SIZE]; 4];
    for (number, block) in blocks.iter_mut().enumerate() {
        *block = operation.read_block(number as u8)?;
    }

    let sector = SectorZero { card, blocks, key };
    let current_uid = sector.current_uid();
    if current_uid != sector.card.uid.as_slice() {
        return Err(NfcError::CardChanged);
    }
    Ok(sector)
}

enum VerifyOutcome {
    Applied(CardInfo),
    Unchanged,
    Different(CardInfo),
}

/// Reselects and reads block 0 after an attempt. This path deliberately
/// ignores cancellation: once a write may have reached the card, the caller
/// must learn the outcome instead of treating a cancel as "not written".
fn verify_after_attempt(
    operation: &mut Operation<'_>,
    plan: &ClassicUidPlan,
) -> Result<VerifyOutcome> {
    let reader = operation.reader_mut();
    let card = reader.card_info()?;
    reader.authenticate(0, plan.sector0_key.0, plan.sector0_key.1)?;
    let block0 = reader.read_block(0)?;
    let trailer = reader.read_block(3)?;
    let target_bcc = bcc_for_uid(&plan.target_uid)?;
    let bcc_ok = block0[0..UID_LEN] == plan.target_uid
        && block0[4] == target_bcc
        && block0 == plan.new_block0;
    if card.card_type == CardType::MifareClassic1K
        && card.uid.as_slice() == plan.target_uid.as_slice()
        && bcc_ok
        && trailer == plan.old_trailer
    {
        return Ok(VerifyOutcome::Applied(card));
    }
    if card.uid.as_slice() == &plan.old_block0[0..UID_LEN]
        && block0 == plan.old_block0
        && trailer == plan.old_trailer
    {
        return Ok(VerifyOutcome::Unchanged);
    }
    Ok(VerifyOutcome::Different(card))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uid_text_accepts_compact_and_separated_forms() {
        let expected = [0x04, 0xA2, 0x24, 0x8A];
        for text in ["04A2248A", "04 A2 24 8A", "04:a2:24:8a", "04-A2-24-8A"] {
            assert_eq!(parse_uid(text).expect("uid"), expected, "{text}");
        }
        assert_eq!(format_uid(&expected), "04A2248A");
        assert_eq!(normalize_uid_text("04 a2:24-8a"), "04A2248A");
    }

    #[test]
    fn uid_text_rejects_wrong_length_and_non_hex() {
        for text in ["", "04A224", "04A2248AFF", "04A2248G"] {
            assert!(matches!(parse_uid(text), Err(NfcError::InvalidArgument(_))));
        }
    }

    #[test]
    fn backend_labels_cover_both_paths() {
        assert!(UidWriteBackend::Gen1a.label().contains("Gen1A"));
        assert!(UidWriteBackend::Standard.label().contains("block-0"));
    }
}
