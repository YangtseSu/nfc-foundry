//! Type 4 NDEF workflows with pre-mutation backups and identity checks.
//!
//! Reader I/O stays behind [`crate::Operation`]. NDEF framing and capacity
//! checks live in `nfc-type4`; this layer adds the application policy.

use nfc_core::{CancelToken, NfcError, Result};
use nfc_ndef::NdefMessage;
use nfc_type4::exchange;
use nfc_type4::{Type4CapabilityContainer, Type4Ndef};

use crate::classic::{BackupRecord, BackupStore};
use crate::operation::{Operation, OperationKind};

/// A complete Type 4 read.
pub type Type4ReadResult = nfc_type4::Type4ReadResult;

/// NDEF write request.
pub struct Type4NdefWrite<'a> {
    pub message: NdefMessage,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type4NdefWrite<'a> {
    pub fn new(message: NdefMessage, backup: &'a BackupStore) -> Self {
        Self {
            message,
            backup,
            cancel: CancelToken::new(),
        }
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Clear request. Clearing removes the NDEF message, so it needs explicit
/// confirmation at the service boundary.
pub struct Type4Clear<'a> {
    pub confirm: bool,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type4Clear<'a> {
    pub fn new(backup: &'a BackupStore) -> Self {
        Self {
            confirm: false,
            backup,
            cancel: CancelToken::new(),
        }
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of a verified Type 4 mutation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type4MutationResult {
    pub cc: Type4CapabilityContainer,
    pub ndef: Type4Ndef,
    /// Encoded NDEF message bytes written; zero for a clear.
    pub written: usize,
    pub backup: BackupRecord,
}

/// Read the CC and NDEF file.
pub fn read(operation: &mut Operation<'_>, cancel: &CancelToken) -> Result<Type4ReadResult> {
    let result = read_current(operation, cancel)?;
    operation.check_iso_dep_card()?;
    Ok(result)
}

/// Write one NDEF message with a pre-write backup and read-back verification.
pub fn write_ndef(
    operation: &mut Operation<'_>,
    request: Type4NdefWrite<'_>,
) -> Result<Type4MutationResult> {
    request.cancel.check()?;
    let current = read_current(operation, &request.cancel)?;
    ensure_writable(&current.cc)?;
    request.message.validate()?;
    let encoded = request.message.encode()?;
    if encoded.len() > current.ndef.capacity() {
        return Err(NfcError::InvalidArgument(format!(
            "the NDEF message is {} bytes but the file holds at most {}",
            encoded.len(),
            current.ndef.capacity()
        )));
    }
    let backup = save_backup(operation, &current, request.backup, "ndef-write")?;
    let card = operation.card().clone();
    let written = nfc_type4::write_ndef(
        operation.reader_mut(),
        &card,
        &current.cc,
        &request.message,
        &request.cancel,
    )?;
    operation.check_iso_dep_card()?;
    Ok(Type4MutationResult {
        cc: current.cc,
        ndef: written.ndef,
        written: written.written,
        backup,
    })
}

/// Clear the NDEF file after an explicit confirmation.
pub fn clear(
    operation: &mut Operation<'_>,
    request: Type4Clear<'_>,
) -> Result<Type4MutationResult> {
    request.cancel.check()?;
    if !request.confirm {
        return Err(NfcError::InvalidArgument(
            "clearing the Type 4 NDEF file needs explicit confirmation".to_owned(),
        ));
    }
    let current = read_current(operation, &request.cancel)?;
    ensure_writable(&current.cc)?;
    let backup = save_backup(operation, &current, request.backup, "ndef-clear")?;
    let card = operation.card().clone();
    let cleared =
        nfc_type4::clear_ndef(operation.reader_mut(), &card, &current.cc, &request.cancel)?;
    operation.check_iso_dep_card()?;
    Ok(Type4MutationResult {
        cc: current.cc,
        ndef: cleared.ndef,
        written: 0,
        backup,
    })
}

/// Read the tag and save a project JSON backup without mutating it.
pub fn dump(
    operation: &mut Operation<'_>,
    backup: &BackupStore,
    cancel: &CancelToken,
) -> Result<BackupRecord> {
    let current = read_current(operation, cancel)?;
    operation.check_iso_dep_card()?;
    save_backup(operation, &current, backup, "dump")
}

impl crate::Service {
    /// Read the Type 4 CC and NDEF file.
    pub fn type4_read(&mut self, cancel: CancelToken) -> Result<Type4ReadResult> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel.clone())?;
        read(&mut operation, &cancel)
    }

    /// Write one NDEF message with a backup and verification.
    pub fn type4_ndef_write(&mut self, request: Type4NdefWrite<'_>) -> Result<Type4MutationResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        write_ndef(&mut operation, request)
    }

    /// Clear the NDEF file after confirmation.
    pub fn type4_clear(&mut self, request: Type4Clear<'_>) -> Result<Type4MutationResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(OperationKind::Write, cancel)?;
        clear(&mut operation, request)
    }

    /// Read the tag and save a Type 4 project backup.
    pub fn type4_dump(
        &mut self,
        backup: &BackupStore,
        cancel: CancelToken,
    ) -> Result<BackupRecord> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel.clone())?;
        dump(&mut operation, backup, &cancel)
    }
}

fn read_current(operation: &mut Operation<'_>, cancel: &CancelToken) -> Result<Type4ReadResult> {
    let card = operation.card().clone();
    nfc_type4::read_tag(operation.reader_mut(), &card, cancel)
}

fn ensure_writable(cc: &Type4CapabilityContainer) -> Result<()> {
    let control = cc.ndef_file()?;
    if let Some(reason) = control.write_denied_reason() {
        return Err(NfcError::Unsupported(reason));
    }
    Ok(())
}

fn save_backup(
    operation: &Operation<'_>,
    current: &Type4ReadResult,
    backup: &BackupStore,
    action: &str,
) -> Result<BackupRecord> {
    let bytes = exchange::export(operation.card(), current)?;
    backup.save_bytes(&label(operation.card(), action), &bytes)
}

fn label(card: &nfc_core::CardInfo, action: &str) -> String {
    let uid = card.uid_hex().replace(' ', "");
    format!("{uid}-TYPE4-{action}")
}

/// A human-readable NDEF state label.
pub fn ndef_label(state: &nfc_type4::NdefState) -> String {
    match state {
        nfc_type4::NdefState::Empty => "empty".to_owned(),
        nfc_type4::NdefState::Message(message) => {
            format!("{} record(s)", message.records().len())
        }
    }
}
