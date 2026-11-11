//! Single-block writes with access checks, backups, and read-back verification.

use crate::operation::Operation;
use nfc_classic::{AccessBits, Dump, KeyMask, Layout};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, Key, KeyType, NfcError, Result};

use super::backup::{BackupRecord, BackupStore};
use super::keys::KeyStore;
use super::read::{candidate_keys, layout_for_card, read_dump};

/// Request to write one Classic block.
pub struct ClassicBlockWrite<'a> {
    pub block: usize,
    pub data: [u8; BLOCK_SIZE],
    pub keys: &'a KeyStore,
    pub include_common_keys: bool,
    /// Optional explicit key. Without it, the first allowed candidate that the
    /// card accepts is used.
    pub key: Option<(KeyType, Key)>,
    /// Sector trailers require an explicit confirmation at the service
    /// boundary.
    pub confirm_trailer: bool,
    /// Identity of the image the write was composed from. A live card with a
    /// different UID, ATQA, or SAK is refused before the backup.
    pub expected_card: Option<CardInfo>,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> ClassicBlockWrite<'a> {
    pub fn new(
        block: usize,
        data: [u8; BLOCK_SIZE],
        keys: &'a KeyStore,
        backup: &'a BackupStore,
    ) -> Self {
        Self {
            block,
            data,
            keys,
            include_common_keys: true,
            key: None,
            confirm_trailer: false,
            expected_card: None,
            backup,
            cancel: CancelToken::new(),
        }
    }

    pub fn key(mut self, key_type: KeyType, key: Key) -> Self {
        self.key = Some((key_type, key));
        self
    }

    pub fn include_common_keys(mut self, include: bool) -> Self {
        self.include_common_keys = include;
        self
    }

    pub fn confirm_trailer(mut self, confirm: bool) -> Self {
        self.confirm_trailer = confirm;
        self
    }

    pub fn expected_card(mut self, card: CardInfo) -> Self {
        self.expected_card = Some(card);
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Outcome of a single-block write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ClassicWriteResult {
    pub block: usize,
    pub changed: bool,
    pub backup: Option<BackupRecord>,
}

/// Writes one block after validating access conditions and saving a backup.
/// Block 0 is never written by this path.
pub fn write_block(
    operation: &mut Operation<'_>,
    request: ClassicBlockWrite<'_>,
) -> Result<ClassicWriteResult> {
    request.cancel.check()?;
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let layout = layout_for_card(operation.card())?;
    if request.block >= layout.block_count() {
        return Err(NfcError::InvalidArgument(format!(
            "block {} is outside the {} layout",
            request.block,
            layout.name()
        )));
    }
    if request.block == 0 {
        return Err(NfcError::Unsupported(
            "manufacturer block 0 writes are not enabled",
        ));
    }
    let sector = layout
        .sector_for_block(request.block)
        .expect("block is within the layout");
    let is_trailer = layout
        .is_trailer(request.block)
        .expect("block is within the layout");
    if is_trailer && !request.confirm_trailer {
        return Err(NfcError::InvalidArgument(format!(
            "writing sector {sector} trailer block {} requires explicit confirmation",
            request.block
        )));
    }
    if is_trailer {
        AccessBits::decode([request.data[6], request.data[7], request.data[8]]).map_err(
            |error| {
                NfcError::InvalidArgument(format!(
                    "block {} trailer access bits are invalid: {error}",
                    request.block
                ))
            },
        )?;
    }

    let backup_dump = read_dump(operation, request.keys, request.include_common_keys)?.dump;
    let current = backup_dump
        .blocks
        .get(request.block)
        .ok_or_else(|| NfcError::InvalidArgument("block is outside the dump".to_owned()))?;
    if !current.complete() {
        return Err(NfcError::VerificationFailed(format!(
            "block {} could not be read before the write",
            request.block
        )));
    }
    if current.data == request.data {
        return Ok(ClassicWriteResult {
            block: request.block,
            changed: false,
            backup: None,
        });
    }

    let access = backup_dump.access_bits(sector)?;
    let required = if is_trailer {
        access.groups[3].full_trailer_write_keys()
    } else {
        let group = layout
            .access_group_for_block(request.block)
            .expect("block is within the layout");
        access.groups[group].data_write_keys()
    };
    let allowed = AllowedTypes::from(required);
    if !allowed.any() {
        return Err(NfcError::Unsupported(
            "the current access bits do not permit writing this block",
        ));
    }
    if let Some((key_type, _)) = request.key
        && !allowed.allows(key_type)
    {
        return Err(NfcError::Unsupported(
            "the current access bits do not permit writing this block with that key type",
        ));
    }

    let candidates = match request.key {
        Some((key_type, key)) => vec![(key_type, key)],
        None => candidate_keys(
            request.keys,
            operation.card(),
            request.include_common_keys,
            sector,
        )
        .into_iter()
        .filter(|(key_type, _, _)| allowed.allows(*key_type))
        .map(|(key_type, key, _)| (key_type, key))
        .collect(),
    };
    if candidates.is_empty() {
        return Err(NfcError::AuthenticationFailed);
    }

    let backup = request
        .backup
        .save(&backup_dump, &backup_label(&backup_dump, layout, "write"))?;
    let first = layout
        .first_block(sector)
        .expect("sector is within the layout");
    let mut authenticated = None;
    for (key_type, key) in candidates {
        request.cancel.check()?;
        match operation.authenticate(first as u8, key_type, key) {
            Ok(()) => {
                authenticated = Some((key_type, key));
                break;
            }
            Err(NfcError::AuthenticationFailed) => continue,
            Err(error) => return Err(error),
        }
    }
    let Some(authenticated) = authenticated else {
        return Err(NfcError::AuthenticationFailed);
    };

    if is_trailer {
        operation.write_trailer_verified(
            request.block as u8,
            request.data,
            authenticated,
            access,
        )?;
    } else {
        operation.write_block_verified(request.block as u8, request.data)?;
    }
    operation.check_card()?;
    Ok(ClassicWriteResult {
        block: request.block,
        changed: true,
        backup: Some(backup),
    })
}

/// Refuses an operation whose image came from a different card. The check
/// runs before any backup or mutation, so a card swap leaves no trace.
pub(crate) fn check_expected_card(actual: &CardInfo, expected: Option<&CardInfo>) -> Result<()> {
    match expected {
        Some(expected) if !expected.same_identity(actual) => Err(NfcError::CardChanged),
        _ => Ok(()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AllowedTypes {
    a: bool,
    b: bool,
}

impl AllowedTypes {
    pub(crate) fn all() -> Self {
        Self { a: true, b: true }
    }

    pub(crate) fn from(mask: KeyMask) -> Self {
        Self {
            a: mask.allows_a(),
            b: mask.allows_b(),
        }
    }

    pub(crate) fn intersect(&mut self, mask: KeyMask) {
        self.a &= mask.allows_a();
        self.b &= mask.allows_b();
    }

    pub(crate) fn any(&self) -> bool {
        self.a || self.b
    }

    pub(crate) fn allows(&self, key_type: KeyType) -> bool {
        match key_type {
            KeyType::A => self.a,
            KeyType::B => self.b,
        }
    }

    pub(crate) fn types(&self) -> super::plan::WriteKeyTypes {
        super::plan::WriteKeyTypes {
            a: self.a,
            b: self.b,
        }
    }
}

pub(crate) fn backup_label(dump: &Dump, layout: Layout, action: &str) -> String {
    let uid = dump
        .card
        .as_ref()
        .map(|card| card.uid_hex().replace(' ', ""));
    match uid {
        Some(uid) if !uid.is_empty() => format!("{uid}-{}-{action}", layout.name()),
        _ => format!("{}-{action}", layout.name()),
    }
}
