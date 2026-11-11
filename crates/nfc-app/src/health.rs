//! Card details and health derived from one image, the key store, and the
//! workbench dirty set.
//!
//! The model is hardware-free: the CLI and GUI feed it the same loaded image
//! and key store, so a details view and `nfctl` report the same counts and the
//! same validation findings.

use nfc_classic::{BccStatus, Dump, Layout};
use nfc_core::{CardType, NfcError};

use crate::classic::KeyStore;

/// Selection facts for the card that produced the image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardIdentityHealth {
    pub uid: String,
    pub uid_length: usize,
    pub atqa: [u8; 2],
    pub sak: u8,
    pub card_type: CardType,
}

/// How many sectors have a verified key for this card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyCoverage {
    pub sectors: usize,
    /// Sectors with at least one verified key.
    pub any_sectors: usize,
    pub key_a_sectors: usize,
    pub key_b_sectors: usize,
    pub both_sectors: usize,
}

/// One image's details, completeness, validation findings, key coverage, and
/// dirty state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardHealth {
    pub identity: Option<CardIdentityHealth>,
    pub layout: Option<Layout>,
    pub readable_blocks: usize,
    pub total_blocks: usize,
    pub readable_sectors: usize,
    pub total_sectors: usize,
    pub skipped_sectors: Vec<usize>,
    /// Every block in the image is fully known.
    pub complete: bool,
    /// Structural problems: invalid geometry, invalid access bits, or a BCC
    /// mismatch.
    pub errors: Vec<String>,
    /// Non-fatal problems: an incomplete image or an unreadable trailer.
    pub warnings: Vec<String>,
    pub key_coverage: KeyCoverage,
    /// Blocks changed in the workbench relative to the baseline.
    pub dirty_blocks: Vec<usize>,
    /// Keys recorded in the image metadata.
    pub dump_keys: usize,
    pub device: Option<String>,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
}

/// Builds the health report for one Classic image.
pub fn classic_health(dump: &Dump, keys: &KeyStore, dirty_blocks: &[usize]) -> CardHealth {
    let readability = dump.readability();
    let mut health = CardHealth {
        identity: dump.card.as_ref().map(|card| CardIdentityHealth {
            uid: card.uid_hex(),
            uid_length: card.uid.len(),
            atqa: card.atqa,
            sak: card.sak,
            card_type: card.card_type,
        }),
        layout: Some(dump.layout),
        readable_blocks: readability.readable_blocks,
        total_blocks: readability.total_blocks,
        readable_sectors: readability.readable_sectors,
        total_sectors: readability.total_sectors,
        skipped_sectors: readability.skipped_sectors.clone(),
        complete: true,
        errors: Vec::new(),
        warnings: Vec::new(),
        key_coverage: KeyCoverage {
            sectors: readability.total_sectors,
            ..KeyCoverage::default()
        },
        dirty_blocks: dirty_blocks.to_vec(),
        dump_keys: dump.metadata.keys.len(),
        device: dump.metadata.device.clone(),
        started_at: dump.metadata.started_at,
        finished_at: dump.metadata.finished_at,
    };
    health.dirty_blocks.sort_unstable();
    health.dirty_blocks.dedup();

    if let Err(error) = dump.validate_geometry() {
        health.complete = false;
        health.errors.push(error.to_string());
        health.warnings.push(
            "validation stopped at the image geometry; block and trailer checks did not run"
                .to_owned(),
        );
        return health;
    }

    health.complete = dump.complete();
    if !health.complete {
        health.warnings.push(format!(
            "incomplete image: {}/{} blocks readable",
            health.readable_blocks, health.total_blocks
        ));
        if !health.skipped_sectors.is_empty() {
            health.warnings.push(format!(
                "sectors without a complete read: {}",
                sector_ranges(&health.skipped_sectors)
            ));
        }
    }

    if let Some(card) = &dump.card {
        match dump.validate_bcc(card.uid.len()) {
            Ok(BccStatus::Invalid { expected, actual }) => health.errors.push(format!(
                "BCC {actual:02X} does not match the UID (expected {expected:02X})"
            )),
            Ok(BccStatus::Valid | BccStatus::NotApplicable) => {}
            Err(error @ NfcError::VerificationFailed(_)) => {
                health.warnings.push(format!("BCC not checked: {error}"));
            }
            Err(error) => health.errors.push(error.to_string()),
        }
    } else {
        health
            .warnings
            .push("BCC not checked: card identity is not recorded".to_owned());
    }

    for sector in 0..dump.layout.sector_count() {
        match dump.access_bits(sector) {
            Ok(_) => {}
            Err(NfcError::VerificationFailed(_)) => {}
            Err(error) => health
                .errors
                .push(format!("sector {sector} trailer: {error}")),
        }
    }

    for block in &dump.blocks {
        if let Some(error) = &block.error {
            health
                .warnings
                .push(format!("block {}: {error}", block.number));
        }
    }

    if let Some(card) = &dump.card {
        let mut covered = vec![false; dump.layout.sector_count()];
        for verification in keys.verifications_for(card) {
            if verification.sector >= covered.len() {
                continue;
            }
            covered[verification.sector] = true;
            match verification.key_type {
                nfc_core::KeyType::A => health.key_coverage.key_a_sectors += 1,
                nfc_core::KeyType::B => health.key_coverage.key_b_sectors += 1,
            }
        }
        health.key_coverage.any_sectors = covered.iter().filter(|covered| **covered).count();
        health.key_coverage.both_sectors = health
            .key_coverage
            .key_a_sectors
            .min(health.key_coverage.key_b_sectors);
    }

    health
}

/// Collapses sorted sector numbers into `1,3-5` form for compact warnings.
fn sector_ranges(sectors: &[usize]) -> String {
    let Some((&first, rest)) = sectors.split_first() else {
        return "none".to_owned();
    };
    let mut ranges = Vec::new();
    let mut start = first;
    let mut end = first;
    for &sector in rest {
        if sector == end + 1 {
            end = sector;
        } else {
            ranges.push((start, end));
            start = sector;
            end = sector;
        }
    }
    ranges.push((start, end));
    ranges
        .into_iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use nfc_core::{CardInfo, Key, KeyType};

    use super::*;
    use crate::classic::{AuthenticatedKey, KeySource};

    fn card() -> CardInfo {
        CardInfo::new(
            vec![0x04, 0xA2, 0x24, 0x8A],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        )
    }

    fn full_dump() -> Dump {
        let mut dump = Dump::new(Layout::Classic1K).with_card(card());
        for block in &mut dump.blocks {
            block.set_read([0; 16]);
        }
        dump.blocks[0].data[0..4].copy_from_slice(&[0x04, 0xA2, 0x24, 0x8A]);
        dump.blocks[0].data[4] = 0x04 ^ 0xA2 ^ 0x24 ^ 0x8A;
        for sector in 0..dump.layout.sector_count() {
            let trailer = dump.layout.trailer_block(sector).expect("trailer");
            dump.blocks[trailer].data[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
        }
        dump
    }

    fn key(byte: u8) -> Key {
        Key::from_hex(&format!(
            "{byte:02X}{byte:02X}{byte:02X}{byte:02X}{byte:02X}{byte:02X}"
        ))
        .expect("key")
    }

    #[test]
    fn a_complete_image_reports_identity_facts_and_no_findings() {
        let dump = full_dump();
        let store = KeyStore::new();
        let health = classic_health(&dump, &store, &[]);
        let identity = health.identity.expect("identity");
        assert_eq!(identity.uid, "04 A2 24 8A");
        assert_eq!(identity.uid_length, 4);
        assert_eq!(identity.atqa, [0x00, 0x04]);
        assert_eq!(identity.sak, 0x08);
        assert_eq!(identity.card_type, CardType::MifareClassic1K);
        assert_eq!(health.layout, Some(Layout::Classic1K));
        assert!(health.complete);
        assert_eq!(health.readable_blocks, 64);
        assert_eq!(health.readable_sectors, 16);
        assert!(health.errors.is_empty(), "{:?}", health.errors);
        assert!(health.warnings.is_empty(), "{:?}", health.warnings);
    }

    #[test]
    fn an_incomplete_image_is_a_warning_and_keeps_the_readable_counts() {
        let mut dump = full_dump();
        dump.blocks[5].known_mask = 0;
        dump.blocks[5].status = nfc_classic::BlockStatus::AuthFailed;
        dump.blocks[5].error = Some("no key".to_owned());
        let health = classic_health(&dump, &KeyStore::new(), &[]);
        assert!(!health.complete);
        assert_eq!(health.readable_blocks, 63);
        assert_eq!(health.skipped_sectors, vec![1]);
        assert!(health.warnings.iter().any(|line| line.contains("63/64")));
        assert!(health.warnings.iter().any(|line| line.contains("block 5")));
        assert!(health.errors.is_empty());
    }

    #[test]
    fn invalid_access_bits_and_a_wrong_bcc_are_errors() {
        let mut dump = full_dump();
        dump.blocks[0].data[4] ^= 0xFF;
        let trailer = dump.layout.trailer_block(2).expect("trailer");
        dump.blocks[trailer].data[7] = 0x00;
        let health = classic_health(&dump, &KeyStore::new(), &[]);
        assert!(
            health.errors.iter().any(|line| line.contains("BCC")),
            "{:?}",
            health.errors
        );
        assert!(
            health
                .errors
                .iter()
                .any(|line| line.contains("sector 2 trailer")),
            "{:?}",
            health.errors
        );
    }

    #[test]
    fn key_coverage_counts_verified_slots_and_dirty_state_is_sorted() {
        let dump = full_dump();
        let mut store = KeyStore::new();
        store
            .record_verified(
                &card(),
                &[
                    AuthenticatedKey {
                        sector: 0,
                        key_type: KeyType::A,
                        key: key(0xA0),
                        source: KeySource::ReadContext,
                    },
                    AuthenticatedKey {
                        sector: 0,
                        key_type: KeyType::B,
                        key: key(0xB0),
                        source: KeySource::ReadContext,
                    },
                    AuthenticatedKey {
                        sector: 3,
                        key_type: KeyType::A,
                        key: key(0xA1),
                        source: KeySource::ReadContext,
                    },
                ],
            )
            .expect("record");
        let health = classic_health(&dump, &store, &[9, 2, 9]);
        assert_eq!(health.key_coverage.sectors, 16);
        assert_eq!(health.key_coverage.any_sectors, 2);
        assert_eq!(health.key_coverage.key_a_sectors, 2);
        assert_eq!(health.key_coverage.key_b_sectors, 1);
        assert_eq!(health.key_coverage.both_sectors, 1);
        assert_eq!(health.dirty_blocks, vec![2, 9]);
    }

    #[test]
    fn an_image_without_recorded_identity_still_reports_geometry() {
        let mut dump = full_dump();
        dump.card = None;
        let health = classic_health(&dump, &KeyStore::new(), &[]);
        assert!(health.identity.is_none());
        assert_eq!(health.readable_blocks, 64);
        assert!(
            health
                .warnings
                .iter()
                .any(|line| line.contains("card identity is not recorded")),
            "{:?}",
            health.warnings
        );
        assert_eq!(health.key_coverage.sectors, 16);
        assert_eq!(health.key_coverage.key_a_sectors, 0);
        assert_eq!(health.key_coverage.key_b_sectors, 0);
    }
}
