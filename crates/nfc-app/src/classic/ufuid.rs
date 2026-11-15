//! Guarded UFUID sealing for Gen1A magic MIFARE Classic 1K cards.
//!
//! An unsealed UFUID answers the Gen1A `0x40`/`0x43` unlock and can be
//! rewritten like a UID card. Sealing writes the magic configuration that
//! disables the backdoor permanently; the card then behaves as an ordinary
//! MIFARE Classic. The seal is irreversible and cannot be detected before the
//! command is attempted, so the workflow proves the backdoor is present,
//! backs up sector 0, and verifies both the unchanged data and the missing
//! backdoor after the attempt.

use std::time::{Duration, Instant};

use nfc_classic::{Dump, Layout, bcc_for_uid};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, CardType, NfcError, Result};
use nfc_transport::UFUID_SEAL_CONFIG;

const SEAL_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
const SEAL_PROBE_INTERVAL: Duration = Duration::from_millis(100);

use crate::operation::Operation;

use super::backup::{BackupRecord, BackupStore};
use super::keys::KeyStore;
use super::plan::{WRITE_PLAN_TTL, WriteToken, unix_now};
use super::uid::{UID_LEN, read_sector_zero};
use super::workbench::dump_fingerprint;
use super::write::{backup_label, check_expected_card};

/// Read-only request for a UFUID seal plan.
pub struct ClassicSealPreflight<'a> {
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    /// Identity of the image the request was composed from.
    pub expected_card: Option<CardInfo>,
    /// Advanced mode is a hard precondition.
    pub advanced: bool,
    pub cancel: CancelToken,
}

impl<'a> ClassicSealPreflight<'a> {
    pub fn new(keys: &'a KeyStore) -> Self {
        Self {
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

/// A validated seal plan bound to one card state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicSealPlan {
    pub token: WriteToken,
    pub card: CardInfo,
    pub layout: Layout,
    pub uid: [u8; UID_LEN],
    pub block0: [u8; BLOCK_SIZE],
    pub trailer: [u8; BLOCK_SIZE],
    /// Key that authenticated sector 0 during the preflight.
    pub sector0_key: (nfc_core::KeyType, nfc_core::Key),
    pub config: [u8; BLOCK_SIZE],
    pub warnings: Vec<String>,
    /// Unix seconds after which the token is refused.
    pub expires_at: u64,
    /// Fingerprint of the four sector-0 blocks and the card identity.
    pub card_fingerprint: u64,
}

/// Request that executes a stored seal plan.
pub struct ClassicSealExecution<'a> {
    pub token: WriteToken,
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    pub backup: &'a BackupStore,
    pub advanced: bool,
    pub confirmed: bool,
    pub cancel: CancelToken,
}

impl<'a> ClassicSealExecution<'a> {
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

/// Pre-seal record. The project backup holds the complete old sector 0.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UfuidSealBackupRecord {
    pub backup: BackupRecord,
    pub block0: [u8; BLOCK_SIZE],
    pub trailer: [u8; BLOCK_SIZE],
    pub uid: [u8; UID_LEN],
    pub config: [u8; BLOCK_SIZE],
}

/// Outcome of a successful seal.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicSealResult {
    pub card_before: CardInfo,
    pub card_after: CardInfo,
    pub uid: [u8; UID_LEN],
    pub block0: [u8; BLOCK_SIZE],
    pub trailer: [u8; BLOCK_SIZE],
    pub config: [u8; BLOCK_SIZE],
    pub backup: UfuidSealBackupRecord,
    pub warnings: Vec<String>,
}

fn require_manufacturer_seal(operation: &mut Operation<'_>) -> Result<()> {
    if !operation.capabilities().manufacturer_seal {
        return Err(NfcError::Unsupported(
            "the selected reader does not advertise the UFUID seal; sealing is unavailable",
        ));
    }
    if !operation.has_manufacturer_sealer() {
        return Err(NfcError::Unsupported(
            "the selected reader has no protected UFUID sealer",
        ));
    }
    Ok(())
}

/// Builds a seal plan without changing the card. The Gen1A probe is read-only;
/// a card that does not answer it is refused before a backup or write.
pub fn preflight(
    operation: &mut Operation<'_>,
    request: &ClassicSealPreflight<'_>,
    token: WriteToken,
) -> Result<ClassicSealPlan> {
    require_manufacturer_seal(operation)?;
    request.cancel.check()?;
    if !request.advanced {
        return Err(NfcError::InvalidArgument(
            "sealing a UFUID requires Advanced mode".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let card = operation.card().clone();
    if card.card_type != CardType::MifareClassic1K {
        return Err(NfcError::Unsupported(
            "UFUID sealing requires a MIFARE Classic 1K card",
        ));
    }
    if card.uid.len() != UID_LEN {
        return Err(NfcError::Unsupported(
            "UFUID sealing requires a four-byte UID MIFARE Classic 1K card",
        ));
    }
    if !operation.probe_gen1a()? {
        return Err(NfcError::Unsupported(
            "the card did not answer the Gen1A unlock; only an unsealed UFUID can be sealed",
        ));
    }

    let sector = read_sector_zero(operation, request.keys, request.include_common_keys, None)?;
    let uid = sector.current_uid();
    let expected_bcc = bcc_for_uid(&uid)?;
    if sector.blocks[0][4] != expected_bcc {
        return Err(NfcError::VerificationFailed(format!(
            "manufacturer block 0 BCC is {:02X}, expected {expected_bcc:02X}; the card image is inconsistent",
            sector.blocks[0][4]
        )));
    }

    let warnings = vec![
        "sealing permanently disables the Gen1A backdoor; the card becomes an ordinary MIFARE Classic and cannot be unsealed".to_owned(),
        "only a UFUID implements this seal; a Gen1A card without seal support refuses the command unchanged".to_owned(),
        "a sector-0 backup is written first; sealing changes card configuration, not sector data".to_owned(),
        "the operation is verified by re-reading sector 0 and polling until two consecutive Gen1A probes stop answering".to_owned(),
    ];
    request.cancel.check()?;

    Ok(ClassicSealPlan {
        token,
        card: sector.card.clone(),
        layout: Layout::Classic1K,
        uid,
        block0: sector.blocks[0],
        trailer: sector.blocks[3],
        sector0_key: sector.key,
        config: UFUID_SEAL_CONFIG,
        warnings,
        expires_at: unix_now() + WRITE_PLAN_TTL.as_secs(),
        card_fingerprint: dump_fingerprint(&sector.image()),
    })
}

enum SealVerification {
    Sealed(CardInfo),
    StillUnsealed(CardInfo),
    Different(CardInfo),
}

/// Rechecks the sealed card. Cancellation is ignored here: once the command
/// may have reached the card, the caller must learn the outcome.
fn verify_after_attempt(
    operation: &mut Operation<'_>,
    plan: &ClassicSealPlan,
) -> Result<SealVerification> {
    let card = {
        let reader = operation.reader_mut();
        let card = reader.card_info()?;
        if !card.same_identity(&plan.card) {
            return Ok(SealVerification::Different(card));
        }
        reader.authenticate(0, plan.sector0_key.0, plan.sector0_key.1)?;
        let mut dump = Dump::new(Layout::Classic1K).with_card(card.clone());
        for (number, block) in dump.blocks.iter_mut().take(4).enumerate() {
            block.set_read(reader.read_block(number as u8)?);
        }
        if dump_fingerprint(&dump) != plan.card_fingerprint {
            return Ok(SealVerification::Different(card));
        }
        card
    };

    // The configuration can take effect after the write acknowledgement, and
    // a reader may still hold a stale ACK. Poll until two consecutive probes
    // are negative instead of trusting the first exchange.
    let deadline = Instant::now() + SEAL_SETTLE_TIMEOUT;
    let mut consecutive_negative = 0;
    loop {
        if operation.probe_gen1a_after_attempt()? {
            consecutive_negative = 0;
        } else {
            consecutive_negative += 1;
        }
        if consecutive_negative >= 2 {
            return Ok(SealVerification::Sealed(card));
        }
        if Instant::now() >= deadline {
            return Ok(SealVerification::StillUnsealed(card));
        }
        std::thread::sleep(SEAL_PROBE_INTERVAL);
    }
}

/// Executes a seal plan after rechecking the token, card, and sector-0 state.
pub fn execute(
    operation: &mut Operation<'_>,
    request: ClassicSealExecution<'_>,
    plan: &ClassicSealPlan,
) -> Result<ClassicSealResult> {
    require_manufacturer_seal(operation)?;
    if !request.advanced {
        return Err(NfcError::InvalidArgument(
            "sealing a UFUID requires Advanced mode".to_owned(),
        ));
    }
    if !request.confirmed {
        return Err(NfcError::InvalidArgument(
            "the UFUID seal must be confirmed explicitly".to_owned(),
        ));
    }
    request.cancel.check()?;
    if plan.layout != Layout::Classic1K {
        return Err(NfcError::Unsupported(
            "UFUID sealing supports MIFARE Classic 1K only",
        ));
    }
    check_expected_card(operation.card(), Some(&plan.card))?;

    if !operation.probe_gen1a()? {
        return Err(NfcError::VerificationFailed(
            "the card no longer answers the Gen1A unlock; the seal was not attempted".to_owned(),
        ));
    }
    let sector = read_sector_zero(
        operation,
        request.keys,
        request.include_common_keys,
        Some(plan.sector0_key),
    )?;
    if dump_fingerprint(&sector.image()) != plan.card_fingerprint {
        return Err(NfcError::CardChanged);
    }

    let image = sector.image();
    let backup = request
        .backup
        .save(&image, &backup_label(&image, plan.layout, "ufuid-seal"))?;
    let backup_path = backup.path.display().to_string();
    let record = UfuidSealBackupRecord {
        backup,
        block0: plan.block0,
        trailer: plan.trailer,
        uid: plan.uid,
        config: plan.config,
    };

    let attempt = operation.seal_manufacturer_card();
    let mut warnings = plan.warnings.clone();
    match verify_after_attempt(operation, plan) {
        Ok(SealVerification::Sealed(card_after)) => {
            if let Err(error) = attempt {
                warnings.push(format!(
                    "the seal acknowledgement was lost ({error}); the read-back confirmed the backdoor is disabled"
                ));
            }
            Ok(ClassicSealResult {
                card_before: sector.card,
                card_after,
                uid: plan.uid,
                block0: plan.block0,
                trailer: plan.trailer,
                config: plan.config,
                backup: record,
                warnings,
            })
        }
        Ok(SealVerification::StillUnsealed(card_after)) => {
            if matches!(attempt, Err(NfcError::Cancelled)) {
                return Err(NfcError::Cancelled);
            }
            let detail = match &attempt {
                Ok(()) => String::new(),
                Err(error) => format!("; the command reported: {error}"),
            };
            Err(NfcError::VerificationFailed(format!(
                "the UFUID seal did not disable the Gen1A backdoor on UID {}; sector 0 is unchanged{detail}. Backup: {backup_path}",
                card_after.uid_hex()
            )))
        }
        Ok(SealVerification::Different(card_after)) => Err(NfcError::VerificationFailed(format!(
            "the seal attempt left an unexpected card state (UID {}); the card may have changed. Backup: {backup_path}",
            card_after.uid_hex()
        ))),
        Err(error) => Err(NfcError::VerificationFailed(format!(
            "the seal outcome is unknown because the verification failed ({error}); the card may have changed. Backup: {backup_path}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_config_is_canonical() {
        assert_eq!(UFUID_SEAL_CONFIG[0], 0x85);
        assert_eq!(UFUID_SEAL_CONFIG[1], 0x00);
        assert_eq!(UFUID_SEAL_CONFIG[15], 0x08);
        assert!(UFUID_SEAL_CONFIG[2..15].iter().all(|byte| *byte == 0));
    }
}
