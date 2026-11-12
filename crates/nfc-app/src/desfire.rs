//! DESFire inventory, authenticated read, and NDEF provisioning workflows.

use nfc_core::{CancelToken, CardType, NfcError, Result};
use nfc_desfire::Desfire;
pub use nfc_desfire::{
    NDEF_APPLICATION_AID, NDEF_APPLICATION_ISO_FILE, NDEF_APPLICATION_KEY,
    NDEF_APPLICATION_KEY_SETTINGS, NDEF_CAPABILITY_CONTAINER, NDEF_CC_ACCESS_RIGHTS, NDEF_CC_FILE,
    NDEF_CC_ISO_FILE, NDEF_CC_SIZE, NDEF_DATA_ACCESS_RIGHTS, NDEF_DATA_FILE, NDEF_DATA_ISO_FILE,
    NDEF_DATA_SIZE, NDEF_DF_NAME,
};
use nfc_type4::apdu;
use nfc_type4::{NdefState, Type4CapabilityContainer, Type4Ndef};

use crate::Service;
use crate::operation::{Operation, OperationKind};

/// One standard or backup file read after selecting an application.
///
/// The key is optional so a free-access file can be read without
/// authentication. The request deliberately does not implement `Debug`:
/// key material must not be logged.
pub struct DesfireReadRequest {
    pub aid: [u8; 3],
    pub file_id: u8,
    pub key: Option<Vec<u8>>,
    pub key_number: u8,
    pub offset: u32,
    pub length: Option<u32>,
}

impl DesfireReadRequest {
    pub fn new(aid: [u8; 3], file_id: u8) -> Self {
        Self {
            aid,
            file_id,
            key: None,
            key_number: 0,
            offset: 0,
            length: None,
        }
    }

    pub fn key(mut self, key: Vec<u8>) -> Self {
        self.key = Some(key);
        self
    }

    pub fn key_number(mut self, key_number: u8) -> Self {
        self.key_number = key_number;
        self
    }

    pub fn offset(mut self, offset: u32) -> Self {
        self.offset = offset;
        self
    }

    pub fn length(mut self, length: Option<u32>) -> Self {
        self.length = length;
        self
    }
}

/// File settings and data returned by one authenticated read.
pub struct DesfireRead {
    pub settings: Vec<u8>,
    pub data: Vec<u8>,
}

/// Create the AN11004 NDEF Tag Application on an unformatted card.
///
/// The key is the PICC master key and deliberately stays out of `Debug`. The
/// operation requires Advanced mode and explicit confirmation because the
/// card has no byte-for-byte backup; deleting the application is the undo
/// path.
pub struct DesfireNdefFormatRequest {
    key: Vec<u8>,
    pub key_number: u8,
    pub advanced: bool,
    pub confirm: bool,
    pub cancel: CancelToken,
}

impl DesfireNdefFormatRequest {
    pub fn new(key: Vec<u8>) -> Self {
        Self {
            key,
            key_number: 0,
            advanced: false,
            confirm: false,
            cancel: CancelToken::new(),
        }
    }

    pub fn key_number(mut self, key_number: u8) -> Self {
        self.key_number = key_number;
        self
    }

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
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

/// Delete the NDEF Tag Application created by [`Service::desfire_ndef_format`].
pub struct DesfireNdefDeleteRequest {
    key: Vec<u8>,
    pub key_number: u8,
    pub advanced: bool,
    pub confirm: bool,
    pub cancel: CancelToken,
}

impl DesfireNdefDeleteRequest {
    pub fn new(key: Vec<u8>) -> Self {
        Self {
            key,
            key_number: 0,
            advanced: false,
            confirm: false,
            cancel: CancelToken::new(),
        }
    }

    pub fn key_number(mut self, key_number: u8) -> Self {
        self.key_number = key_number;
        self
    }

    pub fn advanced(mut self, advanced: bool) -> Self {
        self.advanced = advanced;
        self
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

/// Verified NDEF Tag Application after formatting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireNdefFormatResult {
    pub cc: Type4CapabilityContainer,
    pub ndef: Type4Ndef,
}

/// Application inventory after deleting the NDEF Tag Application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesfireNdefDeleteResult {
    pub applications: Vec<[u8; 3]>,
}

impl Service {
    /// Read the DESFire version and application/file inventory without
    /// authentication.
    pub fn desfire_inventory(&mut self, cancel: CancelToken) -> Result<nfc_desfire::Inventory> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        nfc_desfire::inventory(operation.reader_mut())
    }

    /// Read one standard or backup file. Authentication is optional; when a
    /// key is supplied it is used with the legacy DES/2K3DES authenticator.
    pub fn desfire_read(
        &mut self,
        cancel: CancelToken,
        request: DesfireReadRequest,
    ) -> Result<DesfireRead> {
        let mut operation = self.begin_operation(OperationKind::Read, cancel)?;
        let mut desfire = nfc_desfire::Desfire::new(operation.reader_mut());
        desfire.select_application(request.aid)?;
        if let Some(key) = request.key.as_deref() {
            desfire.authenticate_legacy(request.key_number, key)?;
        }
        let settings = desfire.file_settings(request.file_id)?;
        let size = nfc_desfire::Desfire::data_file_size(&settings).ok_or(NfcError::Unsupported(
            "DESFire read currently supports standard and backup data files",
        ))?;
        if request.offset > size {
            return Err(NfcError::InvalidArgument(format!(
                "read offset {} is beyond file size {size}",
                request.offset
            )));
        }
        let remaining = size - request.offset;
        let length = request.length.unwrap_or(remaining);
        if length == 0 || length > remaining {
            return Err(NfcError::InvalidArgument(format!(
                "read length {length} is outside the remaining {remaining} bytes"
            )));
        }
        let data = desfire.read_data(request.file_id, request.offset, length)?;
        if data.len() as u32 != length {
            return Err(NfcError::VerificationFailed(format!(
                "read returned {} bytes, expected {length}",
                data.len()
            )));
        }
        Ok(DesfireRead { settings, data })
    }

    /// Create the AN11004 NDEF Tag Application and its CC/NDEF files.
    ///
    /// This is a full-card-scope mutation: no byte-for-byte backup exists.
    /// The matching undo is [`Service::desfire_ndef_delete`].
    pub fn desfire_ndef_format(
        &mut self,
        request: DesfireNdefFormatRequest,
    ) -> Result<DesfireNdefFormatResult> {
        let cancel = request.cancel.clone();
        if !request.advanced {
            return Err(NfcError::InvalidArgument(
                "DESFire NDEF formatting requires Advanced mode".to_owned(),
            ));
        }
        if !request.confirm {
            return Err(NfcError::InvalidArgument(
                "DESFire NDEF formatting needs explicit confirmation".to_owned(),
            ));
        }
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        nfc_desfire::LegacyKeyType::from_key(&request.key)?;
        let mut operation = self.begin_operation(OperationKind::Write, cancel.clone())?;
        format_ndef(&mut operation, &request.key, request.key_number, &cancel)
    }

    /// Delete only the NDEF Tag Application created by
    /// [`Service::desfire_ndef_format`], leaving the other applications alone.
    pub fn desfire_ndef_delete(
        &mut self,
        request: DesfireNdefDeleteRequest,
    ) -> Result<DesfireNdefDeleteResult> {
        let cancel = request.cancel.clone();
        if !request.advanced {
            return Err(NfcError::InvalidArgument(
                "deleting the DESFire NDEF application requires Advanced mode".to_owned(),
            ));
        }
        if !request.confirm {
            return Err(NfcError::InvalidArgument(
                "deleting the DESFire NDEF application needs explicit confirmation".to_owned(),
            ));
        }
        if request.key_number > 14 {
            return Err(NfcError::InvalidArgument(
                "DESFire application key numbers are 0..=14".to_owned(),
            ));
        }
        nfc_desfire::LegacyKeyType::from_key(&request.key)?;
        let mut operation = self.begin_operation(OperationKind::Write, cancel.clone())?;
        delete_ndef(&mut operation, &request.key, request.key_number, &cancel)
    }
}

fn format_ndef(
    operation: &mut Operation<'_>,
    key: &[u8],
    key_number: u8,
    cancel: &CancelToken,
) -> Result<DesfireNdefFormatResult> {
    ensure_iso_dep(operation)?;
    ensure_ndef_absent(operation, cancel)?;
    cancel.check()?;

    let mut created = false;
    let outcome = (|| -> Result<DesfireNdefFormatResult> {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application([0x00; 3])?;
        desfire.authenticate_legacy(key_number, key)?;
        cancel.check()?;
        desfire.create_application(
            NDEF_APPLICATION_AID,
            NDEF_APPLICATION_KEY_SETTINGS.0,
            NDEF_APPLICATION_KEY_SETTINGS.1,
            Some(NDEF_APPLICATION_ISO_FILE),
            &NDEF_DF_NAME,
        )?;
        created = true;

        desfire.select_application(NDEF_APPLICATION_AID)?;
        desfire.authenticate_legacy(0, &NDEF_APPLICATION_KEY)?;
        cancel.check()?;
        desfire.create_standard_file(
            NDEF_CC_FILE,
            Some(NDEF_CC_ISO_FILE),
            0,
            NDEF_CC_ACCESS_RIGHTS,
            NDEF_CC_SIZE,
        )?;
        desfire.create_standard_file(
            NDEF_DATA_FILE,
            Some(NDEF_DATA_ISO_FILE),
            0,
            NDEF_DATA_ACCESS_RIGHTS,
            NDEF_DATA_SIZE,
        )?;
        desfire.write_data(NDEF_CC_FILE, 0, &NDEF_CAPABILITY_CONTAINER)?;
        desfire.write_data(NDEF_DATA_FILE, 0, &[0x00, 0x00])?;
        cancel.check()?;
        desfire.commit_transaction()?;
        drop(desfire);

        let card = operation.card().clone();
        let read = nfc_type4::read_tag(operation.reader_mut(), &card, cancel)?;
        if read.cc.raw() != NDEF_CAPABILITY_CONTAINER.as_slice() {
            return Err(NfcError::VerificationFailed(format!(
                "the NDEF capability container read back as {:02X?}",
                read.cc.raw()
            )));
        }
        if read.ndef.state != NdefState::Empty {
            return Err(NfcError::VerificationFailed(
                "the NDEF file is not empty after formatting".to_owned(),
            ));
        }
        operation.check_iso_dep_card()?;
        Ok(DesfireNdefFormatResult {
            cc: read.cc,
            ndef: read.ndef,
        })
    })();

    match outcome {
        Ok(result) => Ok(result),
        Err(error) if created => match remove_ndef_application(operation, key, key_number) {
            Ok(()) => Err(error),
            Err(rollback) => Err(NfcError::VerificationFailed(format!(
                "NDEF formatting failed: {error}; rollback failed: {rollback}"
            ))),
        },
        Err(error) => Err(error),
    }
}

fn delete_ndef(
    operation: &mut Operation<'_>,
    key: &[u8],
    key_number: u8,
    cancel: &CancelToken,
) -> Result<DesfireNdefDeleteResult> {
    ensure_iso_dep(operation)?;
    cancel.check()?;
    let inventory = {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.inventory()?
    };
    if !inventory
        .applications
        .iter()
        .any(|application| application.aid == NDEF_APPLICATION_AID)
    {
        return Err(NfcError::InvalidArgument(
            "the NDEF application D27600 is not present".to_owned(),
        ));
    }
    validate_ndef_layout(operation, cancel)?;
    cancel.check()?;
    {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.select_application([0x00; 3])?;
        desfire.authenticate_legacy(key_number, key)?;
        desfire.delete_application(NDEF_APPLICATION_AID)?;
    }
    let applications = {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.application_ids()?
    };
    if applications.contains(&NDEF_APPLICATION_AID) {
        return Err(NfcError::VerificationFailed(
            "the NDEF application is still present after deletion".to_owned(),
        ));
    }
    operation.check_iso_dep_card()?;
    Ok(DesfireNdefDeleteResult { applications })
}

fn ensure_iso_dep(operation: &Operation<'_>) -> Result<()> {
    if operation.card().card_type != CardType::Iso14443 {
        return Err(NfcError::Unsupported(
            "DESFire operations need an ISO/IEC 14443-4 card",
        ));
    }
    Ok(())
}

fn ensure_ndef_absent(operation: &mut Operation<'_>, cancel: &CancelToken) -> Result<()> {
    cancel.check()?;
    let inventory = {
        let mut desfire = Desfire::new(operation.reader_mut());
        desfire.inventory()?
    };
    if inventory
        .applications
        .iter()
        .any(|application| application.aid == NDEF_APPLICATION_AID)
    {
        return Err(NfcError::InvalidArgument(
            "the NDEF application D27600 already exists; delete it before formatting".to_owned(),
        ));
    }
    cancel.check()?;
    if ndef_application_selected(operation)? {
        return Err(NfcError::InvalidArgument(
            "a valid NDEF Tag Application already exists on this card".to_owned(),
        ));
    }
    Ok(())
}

fn ndef_application_selected(operation: &mut Operation<'_>) -> Result<bool> {
    let response = apdu::transceive(
        operation.reader_mut(),
        &apdu::select_tag_application(),
        "selecting the NDEF tag application",
    )?;
    match response.sw {
        [0x90, 0x00] => Ok(true),
        [0x6A, 0x82] => Ok(false),
        _ => {
            let _ = apdu::check_ok(&response, "selecting the NDEF tag application")?;
            Ok(true)
        }
    }
}

fn validate_ndef_layout(operation: &mut Operation<'_>, cancel: &CancelToken) -> Result<()> {
    cancel.check()?;
    let mut desfire = Desfire::new(operation.reader_mut());
    desfire.select_application(NDEF_APPLICATION_AID)?;
    if desfire.file_ids()? != [NDEF_CC_FILE, NDEF_DATA_FILE] {
        return Err(NfcError::Unsupported(
            "the NDEF application contains files that this product did not create",
        ));
    }
    let cc_settings = desfire.file_settings(NDEF_CC_FILE)?;
    let ndef_settings = desfire.file_settings(NDEF_DATA_FILE)?;
    if !file_layout_matches(&cc_settings, [0x00, 0xE0], NDEF_CC_SIZE)
        || !file_layout_matches(&ndef_settings, [0xE0, 0xEE], NDEF_DATA_SIZE)
    {
        return Err(NfcError::Unsupported(
            "the NDEF application files do not match the AN11004 layout",
        ));
    }
    let cc = desfire.read_data(NDEF_CC_FILE, 0, NDEF_CC_SIZE)?;
    if cc.as_slice() != NDEF_CAPABILITY_CONTAINER.as_slice() {
        return Err(NfcError::Unsupported(
            "the NDEF capability container was not created by this product",
        ));
    }
    cancel.check()?;
    Ok(())
}

fn file_layout_matches(settings: &[u8], access: [u8; 2], size: u32) -> bool {
    settings.len() >= 7
        && settings[1] == 0
        && settings[2] == access[0]
        && settings[3] == access[1]
        && Desfire::data_file_size(settings) == Some(size)
}

fn remove_ndef_application(
    operation: &mut Operation<'_>,
    key: &[u8],
    key_number: u8,
) -> Result<()> {
    let mut desfire = Desfire::new(operation.reader_mut());
    let _ = desfire.abort_transaction();
    desfire.select_application([0x00; 3])?;
    desfire.authenticate_legacy(key_number, key)?;
    desfire.delete_application(NDEF_APPLICATION_AID)?;
    if desfire.application_ids()?.contains(&NDEF_APPLICATION_AID) {
        return Err(NfcError::VerificationFailed(
            "the NDEF application is still present after rollback".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_requires_advanced_and_confirmation_before_reader_selection() {
        let mut service = Service::default();
        let error = service
            .desfire_ndef_format(DesfireNdefFormatRequest::new(vec![0x00; 8]))
            .expect_err("Advanced mode is required");
        assert!(error.to_string().contains("Advanced mode"), "{error}");

        let error = service
            .desfire_ndef_format(DesfireNdefFormatRequest::new(vec![0x00; 8]).advanced(true))
            .expect_err("confirmation is required");
        assert!(error.to_string().contains("confirmation"), "{error}");
    }

    #[test]
    fn delete_requires_advanced_and_confirmation_before_reader_selection() {
        let mut service = Service::default();
        let error = service
            .desfire_ndef_delete(DesfireNdefDeleteRequest::new(vec![0x00; 8]))
            .expect_err("Advanced mode is required");
        assert!(error.to_string().contains("Advanced mode"), "{error}");

        let error = service
            .desfire_ndef_delete(DesfireNdefDeleteRequest::new(vec![0x00; 8]).advanced(true))
            .expect_err("confirmation is required");
        assert!(error.to_string().contains("confirmation"), "{error}");
    }

    #[test]
    fn invalid_legacy_key_length_fails_before_any_reader_command() {
        let mut service = Service::default();
        let error = service
            .desfire_ndef_format(
                DesfireNdefFormatRequest::new(vec![0x00; 7])
                    .advanced(true)
                    .confirm(true),
            )
            .expect_err("a 7-byte legacy key is invalid");
        assert!(error.to_string().contains("8-byte DES"), "{error}");
    }
}
