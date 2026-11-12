//! Type 2 / NTAG workflows with backups and verified writes.
//!
//! Reader I/O stays behind [`crate::Operation`]. This layer enforces the
//! user-memory default, Advanced confirmations, pre-mutation backups, and
//! post-write identity checks. NDEF details live in `nfc-type2`.

use nfc_core::{CancelToken, CardInfo, NfcError, PAGE_SIZE, Result};
use nfc_ndef::NdefMessage;
use nfc_type2::{
    LockUpdate, NdefState, PasswordUpdate, RestoreOptions, Type2Image, Type2Ndef, WritePolicy,
    WriteSummary, exchange, format_ndef, format_unformatted_ev1, lock_pages, read_ndef, read_tag,
    restore_image, set_password, write_ndef, write_page_verified,
};
pub use nfc_type2::{Type2Auth, Type2AuthSession};

use crate::classic::backup::{BackupRecord, BackupStore};
use crate::classic::write::check_expected_card;
use crate::operation::Operation;

/// A read-only Type 2 operation with optional PWD_AUTH credentials.
pub struct Type2ReadRequest {
    pub auth: Option<Type2Auth>,
    pub cancel: CancelToken,
}

impl Type2ReadRequest {
    pub fn new(cancel: CancelToken) -> Self {
        Self { auth: None, cancel }
    }

    /// Supply the PWD_AUTH password and expected PACK.
    pub fn auth(mut self, auth: Option<Type2Auth>) -> Self {
        self.auth = auth;
        self
    }
}

/// A read with its identity, image, and explicit partial-read warnings.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type2ReadResult {
    pub card: CardInfo,
    pub image: Type2Image,
    pub warnings: Vec<String>,
}

/// NDEF state and the image it was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type2NdefResult {
    pub card: CardInfo,
    pub image: Type2Image,
    pub ndef: Type2Ndef,
    pub warnings: Vec<String>,
}

/// One page write request.
pub struct Type2PageWrite<'a> {
    pub page: u8,
    pub data: [u8; PAGE_SIZE],
    /// Advanced pages need an explicit confirmation at the service boundary.
    pub advanced: bool,
    pub confirm: bool,
    /// Identity of the image the write was composed from. A live card with a
    /// different UID, ATQA, or SAK is refused before the backup.
    pub expected_card: Option<CardInfo>,
    pub auth: Option<Type2Auth>,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type2PageWrite<'a> {
    pub fn new(page: u8, data: [u8; PAGE_SIZE], backup: &'a BackupStore) -> Self {
        Self {
            page,
            data,
            advanced: false,
            confirm: false,
            expected_card: None,
            auth: None,
            backup,
            cancel: CancelToken::new(),
        }
    }

    /// Supply the PWD_AUTH password and expected PACK.
    pub fn auth(mut self, auth: Option<Type2Auth>) -> Self {
        self.auth = auth;
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

    pub fn expected_card(mut self, card: CardInfo) -> Self {
        self.expected_card = Some(card);
        self
    }

    pub fn cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }
}

/// Result of one page write.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type2PageResult {
    pub image: Type2Image,
    pub page: u8,
    pub previous: [u8; PAGE_SIZE],
    pub written: [u8; PAGE_SIZE],
    pub changed: bool,
    pub backup: Option<BackupRecord>,
    pub warnings: Vec<String>,
}

/// NDEF write or format request.
pub struct Type2NdefWrite<'a> {
    pub message: NdefMessage,
    /// Identity of the image the write was composed from.
    pub expected_card: Option<CardInfo>,
    pub auth: Option<Type2Auth>,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type2NdefWrite<'a> {
    pub fn new(message: NdefMessage, backup: &'a BackupStore) -> Self {
        Self {
            message,
            expected_card: None,
            auth: None,
            backup,
            cancel: CancelToken::new(),
        }
    }

    /// Supply the PWD_AUTH password and expected PACK.
    pub fn auth(mut self, auth: Option<Type2Auth>) -> Self {
        self.auth = auth;
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

/// Explicit NDEF format request.
pub struct Type2Format<'a> {
    pub confirm: bool,
    /// Allow the one-way capability-container write on an unformatted
    /// Ultralight EV1. The caller must obtain a separate confirmation.
    pub allow_otp_cc: bool,
    /// Identity of the image the format was composed from.
    pub expected_card: Option<CardInfo>,
    pub auth: Option<Type2Auth>,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type2Format<'a> {
    pub fn new(backup: &'a BackupStore) -> Self {
        Self {
            confirm: false,
            allow_otp_cc: false,
            expected_card: None,
            auth: None,
            backup,
            cancel: CancelToken::new(),
        }
    }

    /// Allow writing the OTP capability container on an unformatted EV1.
    pub fn allow_otp_cc(mut self, allow: bool) -> Self {
        self.allow_otp_cc = allow;
        self
    }

    /// Supply the PWD_AUTH password and expected PACK.
    pub fn auth(mut self, auth: Option<Type2Auth>) -> Self {
        self.auth = auth;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
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

/// Result of a multi-page NDEF or restore operation.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type2MutationResult {
    pub image: Type2Image,
    pub summary: WriteSummary,
    pub backup: BackupRecord,
    pub warnings: Vec<String>,
}

/// Restore a project dump onto the card.
pub struct Type2Restore<'a> {
    pub source: Type2Image,
    pub options: RestoreOptions,
    pub confirm: bool,
    /// Identity of the card the restore targets.
    pub expected_card: Option<CardInfo>,
    pub auth: Option<Type2Auth>,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type2Restore<'a> {
    pub fn new(source: Type2Image, backup: &'a BackupStore) -> Self {
        Self {
            source,
            options: RestoreOptions {
                restore_user_memory: true,
                ..RestoreOptions::default()
            },
            confirm: false,
            expected_card: None,
            auth: None,
            backup,
            cancel: CancelToken::new(),
        }
    }

    /// Supply the PWD_AUTH password and expected PACK.
    pub fn auth(mut self, auth: Option<Type2Auth>) -> Self {
        self.auth = auth;
        self
    }

    pub fn options(mut self, options: RestoreOptions) -> Self {
        self.options = options;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
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

/// One-way lock request.
pub struct Type2Lock<'a> {
    pub pages: Vec<u8>,
    pub confirm: bool,
    /// Identity of the card the lock applies to.
    pub expected_card: Option<CardInfo>,
    pub auth: Option<Type2Auth>,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type2Lock<'a> {
    pub fn new(pages: Vec<u8>, backup: &'a BackupStore) -> Self {
        Self {
            pages,
            confirm: false,
            expected_card: None,
            auth: None,
            backup,
            cancel: CancelToken::new(),
        }
    }

    /// Supply the PWD_AUTH password and expected PACK.
    pub fn auth(mut self, auth: Option<Type2Auth>) -> Self {
        self.auth = auth;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
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

/// Result of a lock request.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type2LockResult {
    pub image: Type2Image,
    pub update: LockUpdate,
    pub backup: Option<BackupRecord>,
    pub warnings: Vec<String>,
}

/// Advanced password/AUTH0/ACCESS request.
pub struct Type2Password<'a> {
    pub auth0: u8,
    pub access: u8,
    pub password: [u8; PAGE_SIZE],
    pub pack: [u8; 2],
    pub confirm: bool,
    /// Identity of the card the configuration applies to.
    pub expected_card: Option<CardInfo>,
    /// Credentials of the currently protected configuration.
    pub current: Option<Type2Auth>,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type2Password<'a> {
    pub fn new(
        auth0: u8,
        access: u8,
        password: [u8; PAGE_SIZE],
        pack: [u8; 2],
        backup: &'a BackupStore,
    ) -> Self {
        Self {
            auth0,
            access,
            password,
            pack,
            confirm: false,
            expected_card: None,
            current: None,
            backup,
            cancel: CancelToken::new(),
        }
    }

    /// Supply the current password and PACK when the tag is already protected.
    pub fn current(mut self, current: Option<Type2Auth>) -> Self {
        self.current = current;
        self
    }

    pub fn confirm(mut self, confirm: bool) -> Self {
        self.confirm = confirm;
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

/// Read the complete Type 2 memory.
pub fn read(operation: &mut Operation<'_>, request: Type2ReadRequest) -> Result<Type2ReadResult> {
    let cancel = request.cancel;
    let mut access = Type2AuthSession::from_option(request.auth.as_ref());
    let result = read_current(operation, &mut access, &cancel)?;
    operation.check_card()?;
    Ok(Type2ReadResult {
        card: operation.card().clone(),
        image: result.image,
        warnings: result.warnings,
    })
}

/// Read the NDEF state without mutating the tag.
pub fn read_ndef_state(
    operation: &mut Operation<'_>,
    request: Type2ReadRequest,
) -> Result<Type2NdefResult> {
    let cancel = request.cancel;
    let mut access = Type2AuthSession::from_option(request.auth.as_ref());
    let result = read_current(operation, &mut access, &cancel)?;
    let ndef = read_ndef(&result.image)?;
    operation.check_card()?;
    Ok(Type2NdefResult {
        card: operation.card().clone(),
        image: result.image,
        ndef,
        warnings: result.warnings,
    })
}

/// Write one page with a pre-write backup and read-back verification.
pub fn write_page(
    operation: &mut Operation<'_>,
    request: Type2PageWrite<'_>,
) -> Result<Type2PageResult> {
    request.cancel.check()?;
    if request.advanced && !request.confirm {
        return Err(NfcError::InvalidArgument(
            "this Type 2 page is Advanced and needs explicit confirmation".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let policy = if request.advanced {
        WritePolicy::advanced()
    } else {
        WritePolicy::user()
    };
    let mut access = Type2AuthSession::from_option(request.auth.as_ref());
    let read = read_current(operation, &mut access, &request.cancel)?;
    let mut image = read.image;
    let previous = *image.page(request.page).ok_or_else(|| {
        NfcError::VerificationFailed(format!("page {} is not readable", request.page))
    })?;
    if previous == request.data && !request.advanced {
        return Ok(Type2PageResult {
            image,
            page: request.page,
            previous,
            written: previous,
            changed: false,
            backup: None,
            warnings: read.warnings,
        });
    }
    let backup = save_backup(&image, request.backup, "write-page")?;
    let result = write_page_verified(
        operation.reader_mut(),
        &mut image,
        request.page,
        request.data,
        policy,
        &mut access,
        &request.cancel,
    )?;
    operation.check_card()?;
    Ok(Type2PageResult {
        image,
        page: result.page,
        previous: result.previous,
        written: result.written,
        changed: result.changed,
        backup: Some(backup),
        warnings: read.warnings,
    })
}

/// Write an NDEF message, preserving unknown TLVs.
pub fn write_ndef_message(
    operation: &mut Operation<'_>,
    request: Type2NdefWrite<'_>,
) -> Result<Type2MutationResult> {
    request.cancel.check()?;
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let mut access = Type2AuthSession::from_option(request.auth.as_ref());
    let read = read_current(operation, &mut access, &request.cancel)?;
    let mut image = read.image;
    ensure_ndef_writable(&image)?;
    let backup = save_backup(&image, request.backup, "ndef-write")?;
    let summary = write_ndef(
        operation.reader_mut(),
        &mut image,
        &request.message,
        &mut access,
        &request.cancel,
    )?;
    operation.check_card()?;
    Ok(Type2MutationResult {
        image,
        summary,
        backup,
        warnings: read.warnings,
    })
}

/// Format the NDEF data area after an explicit confirmation.
pub fn format_ndef_area(
    operation: &mut Operation<'_>,
    request: Type2Format<'_>,
) -> Result<Type2MutationResult> {
    request.cancel.check()?;
    if !request.confirm {
        return Err(NfcError::InvalidArgument(
            "formatting the NDEF data area needs explicit confirmation".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let mut access = Type2AuthSession::from_option(request.auth.as_ref());
    let read = read_current(operation, &mut access, &request.cancel)?;
    let mut image = read.image;
    ensure_ndef_writable(&image)?;
    let backup = save_backup(&image, request.backup, "ndef-format")?;
    let summary = if image.cc.is_present() {
        format_ndef(
            operation.reader_mut(),
            &mut image,
            &mut access,
            &request.cancel,
        )?
    } else {
        if !request.allow_otp_cc {
            return Err(NfcError::Unsupported(
                "the tag has no capability container; formatting an Ultralight EV1 writes OTP page 3 and needs the OTP confirmation",
            ));
        }
        format_unformatted_ev1(
            operation.reader_mut(),
            &mut image,
            &mut access,
            &request.cancel,
        )?
    };
    operation.check_card()?;
    Ok(Type2MutationResult {
        image,
        summary,
        backup,
        warnings: read.warnings,
    })
}

/// Restore a project dump after an explicit confirmation.
pub fn restore(
    operation: &mut Operation<'_>,
    request: Type2Restore<'_>,
) -> Result<Type2MutationResult> {
    request.cancel.check()?;
    if !request.confirm {
        return Err(NfcError::InvalidArgument(
            "restoring a Type 2 image needs explicit confirmation".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let mut access = Type2AuthSession::from_option(request.auth.as_ref());
    let read = read_current(operation, &mut access, &request.cancel)?;
    let mut image = read.image;
    let backup = save_backup(&image, request.backup, "restore")?;
    let summary = restore_image(
        operation.reader_mut(),
        &mut image,
        &request.source,
        request.options,
        &mut access,
        &request.cancel,
    )?;
    operation.check_card()?;
    Ok(Type2MutationResult {
        image,
        summary,
        backup,
        warnings: read.warnings,
    })
}

/// Apply one-way lock bits after an explicit confirmation.
pub fn lock(operation: &mut Operation<'_>, request: Type2Lock<'_>) -> Result<Type2LockResult> {
    request.cancel.check()?;
    if !request.confirm {
        return Err(NfcError::InvalidArgument(
            "locking Type 2 pages needs explicit confirmation".to_owned(),
        ));
    }
    if request.pages.is_empty() {
        return Err(NfcError::InvalidArgument(
            "select at least one page to lock".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let mut access = Type2AuthSession::from_option(request.auth.as_ref());
    let read = read_current(operation, &mut access, &request.cancel)?;
    let mut image = read.image;
    let backup = save_backup(&image, request.backup, "lock")?;
    let update = lock_pages(
        operation.reader_mut(),
        &mut image,
        &request.pages,
        &mut access,
        &request.cancel,
    )?;
    operation.check_card()?;
    Ok(Type2LockResult {
        image,
        update,
        backup: Some(backup),
        warnings: read.warnings,
    })
}

/// Program PWD/PACK and AUTH0/ACCESS after an explicit confirmation.
pub fn password(
    operation: &mut Operation<'_>,
    request: Type2Password<'_>,
) -> Result<Type2MutationResult> {
    request.cancel.check()?;
    if !request.confirm {
        return Err(NfcError::InvalidArgument(
            "changing Type 2 password configuration needs explicit confirmation".to_owned(),
        ));
    }
    check_expected_card(operation.card(), request.expected_card.as_ref())?;
    let mut access = Type2AuthSession::from_option(request.current.as_ref());
    let read = read_current(operation, &mut access, &request.cancel)?;
    let mut image = read.image;
    let backup = save_backup(&image, request.backup, "password")?;
    set_password(
        operation.reader_mut(),
        &mut image,
        &mut access,
        PasswordUpdate {
            auth0: request.auth0,
            access: request.access,
            password: request.password,
            pack: request.pack,
        },
        &request.cancel,
    )?;
    operation.check_card()?;
    Ok(Type2MutationResult {
        image,
        summary: WriteSummary::default(),
        backup,
        warnings: read.warnings,
    })
}

/// Read the tag and save a project dump without mutating it.
pub fn dump(
    operation: &mut Operation<'_>,
    backup: &BackupStore,
    request: Type2ReadRequest,
) -> Result<BackupRecord> {
    let cancel = request.cancel;
    let mut access = Type2AuthSession::from_option(request.auth.as_ref());
    let result = read_current(operation, &mut access, &cancel)?;
    operation.check_card()?;
    save_backup(&result.image, backup, "dump")
}

/// One Type 2 magic UID change, computed without touching the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type2UidPlan {
    pub card: CardInfo,
    pub image: Type2Image,
    pub current_uid: Vec<u8>,
    pub target_uid: Vec<u8>,
    pub current_pages: [[u8; PAGE_SIZE]; 3],
    pub new_pages: [[u8; PAGE_SIZE]; 3],
}

/// Execute a precomputed Type 2 UID change.
pub struct Type2UidExecution<'a> {
    pub plan: Type2UidPlan,
    pub confirm: bool,
    pub backup: &'a BackupStore,
    pub cancel: CancelToken,
}

impl<'a> Type2UidExecution<'a> {
    pub fn new(plan: Type2UidPlan, backup: &'a BackupStore) -> Self {
        Self {
            plan,
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

/// Result of a verified Type 2 UID change.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Type2UidResult {
    pub card_before: CardInfo,
    pub card_after: CardInfo,
    pub pages: [[u8; PAGE_SIZE]; 3],
    pub backup: BackupRecord,
}

fn type2_bcc0(uid: &[u8]) -> u8 {
    0x88 ^ uid[0] ^ uid[1] ^ uid[2]
}

fn type2_bcc1(uid: &[u8]) -> u8 {
    uid[3] ^ uid[4] ^ uid[5] ^ uid[6]
}

fn page_from_quad(quad: &[u8; nfc_core::PAGE_QUAD_SIZE], page: u8) -> [u8; PAGE_SIZE] {
    let index = usize::from(page) * PAGE_SIZE;
    let mut data = [0u8; PAGE_SIZE];
    data.copy_from_slice(&quad[index..index + PAGE_SIZE]);
    data
}

fn write_uid_page(
    reader: &mut dyn nfc_transport::Reader,
    page: u8,
    data: [u8; PAGE_SIZE],
) -> Result<()> {
    reader.write_page(page, data)?;
    let quad = reader.read_page_quad(0)?;
    let observed = page_from_quad(&quad, page);
    if observed != data {
        return Err(NfcError::VerificationFailed(format!(
            "page {page} read back as {observed:02X?}, expected {data:02X?}"
        )));
    }
    Ok(())
}

fn verify_uid_pages(
    reader: &mut dyn nfc_transport::Reader,
    pages: &[[u8; PAGE_SIZE]; 3],
) -> Result<()> {
    let quad = reader.read_page_quad(0)?;
    for (page, expected) in pages.iter().enumerate() {
        let observed = page_from_quad(&quad, page as u8);
        if observed != *expected {
            return Err(NfcError::VerificationFailed(format!(
                "page {page} read back as {observed:02X?}, expected {expected:02X?}"
            )));
        }
    }
    Ok(())
}

/// Read the card and build a UID plan. No page is written.
pub fn uid_preflight(
    operation: &mut Operation<'_>,
    target_uid: &[u8],
    cancel: &CancelToken,
) -> Result<Type2UidPlan> {
    cancel.check()?;
    let card = operation.card().clone();
    if card.card_type != nfc_core::CardType::Type2 {
        return Err(NfcError::Unsupported(
            "Type 2 UID writes need an NFC Forum Type 2 tag",
        ));
    }
    if target_uid.len() != 7 || card.uid.len() != 7 {
        return Err(NfcError::InvalidArgument(
            "Type 2 UID writes currently support a 7-byte UID".to_owned(),
        ));
    }
    if target_uid[0] != 0x04 {
        return Err(NfcError::InvalidArgument(
            "the UID manufacturer byte must be 04".to_owned(),
        ));
    }
    let mut access = Type2AuthSession::none();
    let read = read_current(operation, &mut access, cancel)?;
    let image = read.image;
    let current_uid = image.uid.clone();
    if current_uid.len() != 7 {
        return Err(NfcError::Unsupported(
            "the selected tag did not report a 7-byte UID",
        ));
    }
    let current_pages = [
        *image
            .page(0)
            .ok_or_else(|| NfcError::InvalidArgument("page 0 is not readable".to_owned()))?,
        *image
            .page(1)
            .ok_or_else(|| NfcError::InvalidArgument("page 1 is not readable".to_owned()))?,
        *image
            .page(2)
            .ok_or_else(|| NfcError::InvalidArgument("page 2 is not readable".to_owned()))?,
    ];
    if current_pages[0][..3] != current_uid[..3]
        || current_pages[1] != current_uid[3..7]
        || current_pages[0][3] != type2_bcc0(&current_uid)
        || current_pages[2][0] != type2_bcc1(&current_uid)
    {
        return Err(NfcError::Unsupported(
            "the UID pages are not in the standard 7-byte NTAG layout",
        ));
    }
    let target = target_uid.to_vec();
    let new_pages = [
        [target[0], target[1], target[2], type2_bcc0(&target)],
        [target[3], target[4], target[5], target[6]],
        [
            type2_bcc1(&target),
            current_pages[2][1],
            current_pages[2][2],
            current_pages[2][3],
        ],
    ];
    if new_pages == current_pages {
        return Err(NfcError::InvalidArgument(
            "the target UID is already set".to_owned(),
        ));
    }
    operation.check_card()?;
    Ok(Type2UidPlan {
        card,
        image,
        current_uid,
        target_uid: target,
        current_pages,
        new_pages,
    })
}

/// Execute a precomputed UID change, verify it, and restore the original UID
/// if any step fails.
pub fn uid_execute(
    operation: &mut Operation<'_>,
    request: Type2UidExecution<'_>,
) -> Result<Type2UidResult> {
    request.cancel.check()?;
    if !request.confirm {
        return Err(NfcError::InvalidArgument(
            "changing the Type 2 UID needs explicit confirmation".to_owned(),
        ));
    }
    let card_before = operation.card().clone();
    check_expected_card(&card_before, Some(&request.plan.card))?;
    let backup = save_backup(&request.plan.image, request.backup, "uid")?;
    let reader = operation.reader_mut();

    let attempt = (|| -> Result<()> {
        request.cancel.check()?;
        for page in [1u8, 2, 0] {
            request.cancel.check()?;
            let index = usize::from(page);
            if request.plan.new_pages[index] != request.plan.current_pages[index] {
                write_uid_page(reader, page, request.plan.new_pages[index])?;
            }
        }
        let after = reader.card_info()?;
        if after.uid != request.plan.target_uid
            || after.atqa != request.plan.card.atqa
            || after.sak != request.plan.card.sak
        {
            return Err(NfcError::VerificationFailed(
                "the card did not reselect with the target UID".to_owned(),
            ));
        }
        verify_uid_pages(reader, &request.plan.new_pages)
    })();

    if let Err(error) = attempt {
        let restore = (|| -> Result<()> {
            for page in [0u8, 1, 2] {
                let index = usize::from(page);
                if request.plan.current_pages[index] != request.plan.new_pages[index] {
                    write_uid_page(reader, page, request.plan.current_pages[index])?;
                }
            }
            let after = reader.card_info()?;
            if after.uid != request.plan.current_uid {
                return Err(NfcError::VerificationFailed(
                    "the card did not reselect with the original UID".to_owned(),
                ));
            }
            verify_uid_pages(reader, &request.plan.current_pages)
        })();
        return Err(match restore {
            Ok(()) => NfcError::VerificationFailed(format!(
                "UID write failed ({error}); the original UID was restored; backup {}",
                backup.path.display()
            )),
            Err(restore_error) => NfcError::VerificationFailed(format!(
                "UID write failed ({error}); RESTORE FAILED ({restore_error}); backup {}",
                backup.path.display()
            )),
        });
    }

    let card_after = reader.card_info()?;
    Ok(Type2UidResult {
        card_before,
        card_after,
        pages: request.plan.new_pages,
        backup,
    })
}

/// Parse a 7-byte Type 2 UID from separated or compact hex.
pub fn parse_uid_hex(text: &str) -> Result<Vec<u8>> {
    let compact = crate::classic::normalize_uid_text(text);
    if compact.len() != 14 {
        return Err(NfcError::InvalidArgument(format!(
            "a Type 2 UID is 14 hexadecimal digits, got {}",
            compact.len()
        )));
    }
    let mut uid = Vec::with_capacity(7);
    for index in 0..7 {
        let start = index * 2;
        uid.push(
            u8::from_str_radix(&compact[start..start + 2], 16).map_err(|_| {
                NfcError::InvalidArgument(format!("{text:?} is not a hexadecimal UID"))
            })?,
        );
    }
    Ok(uid)
}

impl crate::Service {
    /// Read the complete Type 2 memory with explicit partial-read warnings.
    pub fn type2_read(&mut self, request: Type2ReadRequest) -> Result<Type2ReadResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Read, cancel)?;
        read(&mut operation, request)
    }

    /// Read the Type 2 NDEF state without mutating the tag.
    pub fn type2_ndef_read(&mut self, request: Type2ReadRequest) -> Result<Type2NdefResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Read, cancel)?;
        read_ndef_state(&mut operation, request)
    }

    /// Write one page with a backup and read-back verification.
    pub fn type2_write_page(&mut self, request: Type2PageWrite<'_>) -> Result<Type2PageResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Write, cancel)?;
        write_page(&mut operation, request)
    }

    /// Write one NDEF message.
    pub fn type2_ndef_write(&mut self, request: Type2NdefWrite<'_>) -> Result<Type2MutationResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Write, cancel)?;
        write_ndef_message(&mut operation, request)
    }

    /// Format the NDEF data area.
    pub fn type2_ndef_format(&mut self, request: Type2Format<'_>) -> Result<Type2MutationResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Write, cancel)?;
        format_ndef_area(&mut operation, request)
    }

    /// Restore a Type 2 project dump.
    pub fn type2_restore(&mut self, request: Type2Restore<'_>) -> Result<Type2MutationResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Write, cancel)?;
        restore(&mut operation, request)
    }

    /// Apply one-way Type 2 lock bits.
    pub fn type2_lock(&mut self, request: Type2Lock<'_>) -> Result<Type2LockResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Write, cancel)?;
        lock(&mut operation, request)
    }

    /// Program Type 2 password configuration.
    pub fn type2_password(&mut self, request: Type2Password<'_>) -> Result<Type2MutationResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Write, cancel)?;
        password(&mut operation, request)
    }

    /// Build a Type 2 UID plan without writing the card.
    pub fn type2_uid_preflight(
        &mut self,
        target_uid: &[u8],
        cancel: CancelToken,
    ) -> Result<Type2UidPlan> {
        let mut operation =
            self.begin_operation(crate::operation::OperationKind::Read, cancel.clone())?;
        uid_preflight(&mut operation, target_uid, &cancel)
    }

    /// Change a Type 2 magic UID after confirmation and verify it.
    pub fn type2_uid_execute(&mut self, request: Type2UidExecution<'_>) -> Result<Type2UidResult> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Write, cancel)?;
        uid_execute(&mut operation, request)
    }

    /// Read the tag and save a Type 2 project dump.
    pub fn type2_dump(
        &mut self,
        backup: &BackupStore,
        request: Type2ReadRequest,
    ) -> Result<BackupRecord> {
        let cancel = request.cancel.clone();
        let mut operation = self.begin_operation(crate::operation::OperationKind::Read, cancel)?;
        dump(&mut operation, backup, request)
    }
}

fn read_current(
    operation: &mut Operation<'_>,
    access: &mut Type2AuthSession<'_>,
    cancel: &CancelToken,
) -> Result<nfc_type2::Type2ReadResult> {
    let card = operation.card().clone();
    read_tag(operation.reader_mut(), &card, access, cancel)
}

fn ensure_ndef_writable(image: &Type2Image) -> Result<()> {
    if !image.cc.writable() {
        return Err(NfcError::Unsupported(
            "the capability container write access forbids NDEF writes",
        ));
    }
    if image.ndef_area().is_none() {
        return Err(NfcError::VerificationFailed(
            "the NDEF data area is not completely readable".to_owned(),
        ));
    }
    Ok(())
}

fn save_backup(image: &Type2Image, backup: &BackupStore, action: &str) -> Result<BackupRecord> {
    let bytes = exchange::export(image)?;
    backup.save_bytes(&label(image, action), &bytes)
}

fn label(image: &Type2Image, action: &str) -> String {
    let uid = image.uid_hex().replace(' ', "");
    format!("{uid}-{}-{action}", image.layout.product.name())
}

/// A human-readable NDEF state label.
pub fn ndef_label(state: &NdefState) -> String {
    match state {
        NdefState::Unformatted => "unformatted".to_owned(),
        NdefState::Empty => "empty".to_owned(),
        NdefState::Message(message) => format!("{} record(s)", message.records().len()),
    }
}
