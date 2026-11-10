//! State the shell and workspace crates exchange.
//!
//! The shell owns the session entity; workspaces update it after an operation.
//! It contains no reader or card handle, only presentation state.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nfc_core::{CardInfo, CardType, NfcError};

use crate::i18n::{Key, Translator};

/// Severity of a status message or activity entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusKind {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

/// One localized summary plus an unlocalized technical detail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusMessage {
    pub kind: StatusKind,
    pub summary: Key,
    pub detail: Option<String>,
}

impl StatusMessage {
    pub fn new(kind: StatusKind, summary: Key, detail: Option<String>) -> Self {
        Self {
            kind,
            summary,
            detail,
        }
    }

    pub fn info(summary: Key) -> Self {
        Self::new(StatusKind::Info, summary, None)
    }

    pub fn success(summary: Key) -> Self {
        Self::new(StatusKind::Success, summary, None)
    }

    pub fn warning(summary: Key, detail: impl Into<String>) -> Self {
        Self::new(StatusKind::Warning, summary, Some(detail.into()))
    }

    pub fn error(summary: Key, detail: impl Into<String>) -> Self {
        Self::new(StatusKind::Error, summary, Some(detail.into()))
    }

    /// Map an application error to a localized summary and keep its English
    /// detail for diagnostics.
    pub fn from_error(error: &NfcError) -> Self {
        Self::error(error_key(error), error.to_string())
    }
}

/// Localized summary key for an application error.
pub fn error_key(error: &NfcError) -> Key {
    match error {
        NfcError::Unsupported(_) => Key::ErrorUnsupported,
        NfcError::InvalidArgument(_) => Key::ErrorInvalidArgument,
        NfcError::Io(_) => Key::ErrorIo,
        NfcError::Cancelled => Key::ErrorCancelled,
        NfcError::NoCard => Key::ErrorNoCard,
        NfcError::NoReader => Key::ErrorNoReader,
        NfcError::Timeout(_) => Key::ErrorTimeout,
        NfcError::AuthenticationFailed => Key::ErrorAuthentication,
        NfcError::CardChanged => Key::ErrorCardChanged,
        NfcError::VerificationFailed(_) => Key::ErrorVerification,
        NfcError::NotImplemented(_) => Key::ErrorNotImplemented,
    }
}

/// In-flight operation label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Busy {
    pub label: Key,
}

/// Reader summary shown by the shell; driver types stay below `nfc-app`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderSummary {
    pub name: String,
    pub conn_string: String,
}

/// Card identity for the top bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardIdentity {
    pub card_type: String,
    pub family: CardFamily,
    pub uid: String,
    pub atqa: String,
    pub sak: String,
    pub detail: String,
}

impl CardIdentity {
    pub fn from_card(card: &CardInfo, detail: impl Into<String>) -> Self {
        Self {
            card_type: card_type_name(card.card_type).to_owned(),
            family: CardFamily::from_card_type(card.card_type),
            uid: card.uid_hex(),
            atqa: format!("{:02X} {:02X}", card.atqa[0], card.atqa[1]),
            sak: format!("{:02X}", card.sak),
            detail: detail.into(),
        }
    }
}

/// Card protocol family that selects the workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardFamily {
    Classic,
    Type2,
    Iso14443,
    Unknown,
}

impl CardFamily {
    pub const fn from_card_type(card_type: CardType) -> Self {
        match card_type {
            CardType::MifareClassic1K | CardType::MifareClassic4K => Self::Classic,
            CardType::Type2 => Self::Type2,
            CardType::Iso14443 => Self::Iso14443,
            CardType::Unknown => Self::Unknown,
        }
    }
}

/// Protocol-family display name. Protocol names stay unlocalized.
pub fn card_type_name(card_type: CardType) -> &'static str {
    match card_type {
        CardType::Unknown => "Unknown",
        CardType::MifareClassic1K => "MIFARE Classic 1K",
        CardType::MifareClassic4K => "MIFARE Classic 4K",
        CardType::Type2 => "NFC Forum Type 2",
        CardType::Iso14443 => "ISO/IEC 14443-4",
    }
}

/// One session activity entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivityEntry {
    pub time: String,
    pub kind: StatusKind,
    pub title: Key,
    pub detail: String,
}

impl ActivityEntry {
    pub fn new(kind: StatusKind, title: Key, detail: impl Into<String>) -> Self {
        Self {
            time: timestamp(),
            kind,
            title,
            detail: detail.into(),
        }
    }
}

/// Recovery stage shown by the recovery screen and status bar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecoveryStage {
    #[default]
    Idle,
    Acquire,
    Filter,
    Solve,
    Verify,
    Complete,
}

impl RecoveryStage {
    /// Localized stage label.
    pub const fn key(self) -> Key {
        match self {
            Self::Idle => Key::RecoveryIdle,
            Self::Acquire => Key::StageAcquire,
            Self::Filter => Key::StageFilter,
            Self::Solve => Key::StageSolve,
            Self::Verify => Key::StageVerify,
            Self::Complete => Key::StatusRecoveryDone,
        }
    }
}

/// Recovery progress retained across navigation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryView {
    pub stage: RecoveryStage,
    pub percent: u8,
    pub detail: String,
    pub verified: Vec<String>,
    pub failed: bool,
    pub cancelled: bool,
}

impl RecoveryView {
    pub fn running(stage: RecoveryStage, percent: u8, detail: impl Into<String>) -> Self {
        Self {
            stage,
            percent,
            detail: detail.into(),
            ..Self::default()
        }
    }
}

/// Workbench session state shared by the shell and the workspace crates.
#[derive(Clone, Debug)]
pub struct Session {
    pub translator: Translator,
    pub status: StatusMessage,
    pub busy: Option<Busy>,
    pub classic_card: Option<CardIdentity>,
    pub type2_card: Option<CardIdentity>,
    /// Card currently detected on the reader, before any workspace image is
    /// loaded. Drives the family-specific navigation and the Read action.
    pub present_card: Option<CardIdentity>,
    pub present_family: Option<CardFamily>,
    pub devices: Vec<ReaderSummary>,
    pub selected_device: Option<String>,
    pub last_backup: Option<PathBuf>,
    pub activity: Vec<ActivityEntry>,
    pub recovery: Option<RecoveryView>,
    pub watching: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self {
            translator: Translator::default(),
            status: StatusMessage::info(Key::StatusReady),
            busy: None,
            classic_card: None,
            type2_card: None,
            present_card: None,
            present_family: None,
            devices: Vec::new(),
            selected_device: None,
            last_backup: None,
            activity: Vec::new(),
            recovery: None,
            watching: false,
        }
    }
}

impl Session {
    pub fn new(translator: Translator) -> Self {
        Self {
            translator,
            ..Self::default()
        }
    }

    pub fn text(&self, key: Key) -> &'static str {
        self.translator.text(key)
    }

    pub fn set_language(&mut self, translator: Translator) {
        self.translator = translator;
    }

    pub fn set_status(&mut self, message: StatusMessage) {
        self.status = message;
    }

    pub fn set_status_key(&mut self, kind: StatusKind, summary: Key) {
        self.status = StatusMessage::new(kind, summary, None);
    }

    pub fn set_error(&mut self, error: &NfcError) {
        self.status = StatusMessage::from_error(error);
    }

    pub fn begin(&mut self, label: Key) {
        self.busy = Some(Busy { label });
    }

    pub fn finish(&mut self) {
        self.busy = None;
    }

    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    pub fn set_devices(&mut self, devices: Vec<ReaderSummary>, selected: Option<String>) {
        self.devices = devices;
        self.selected_device = selected;
    }

    pub fn selected_reader(&self) -> Option<&ReaderSummary> {
        let selected = self.selected_device.as_deref()?;
        self.devices
            .iter()
            .find(|device| device.conn_string == selected)
    }

    pub fn set_last_backup(&mut self, path: Option<PathBuf>) {
        self.last_backup = path;
    }

    pub fn record(&mut self, entry: ActivityEntry) {
        self.activity.insert(0, entry);
        self.activity.truncate(100);
    }

    pub fn clear_activity(&mut self) {
        self.activity.clear();
    }

    pub fn set_recovery(&mut self, recovery: Option<RecoveryView>) {
        self.recovery = recovery;
    }

    /// Record the card found by a presence scan.
    pub fn set_present_card(&mut self, card: Option<CardIdentity>) {
        let family = card.as_ref().map(|card| card.family);
        self.present_card = card;
        self.present_family = family;
    }

    pub fn reset_cards(&mut self) {
        self.classic_card = None;
        self.type2_card = None;
        self.present_card = None;
        self.present_family = None;
        self.recovery = None;
    }
}

/// UTC wall-clock time with seconds. Activity is session-local, so a time
/// without a date is enough and avoids a date/time dependency.
pub fn timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!(
        "{:02}:{:02}:{:02}Z",
        (seconds / 3600) % 24,
        (seconds / 60) % 60,
        seconds % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_keep_the_english_detail() {
        let status = StatusMessage::from_error(&NfcError::Timeout("reader did not answer".into()));
        assert_eq!(status.summary, Key::ErrorTimeout);
        assert_eq!(status.kind, StatusKind::Error);
        assert!(status.detail.expect("detail").contains("reader"));
    }

    #[test]
    fn activity_is_newest_first_and_bounded() {
        let mut session = Session::default();
        for index in 0..120 {
            session.record(ActivityEntry::new(
                StatusKind::Info,
                Key::ReadCard,
                index.to_string(),
            ));
        }
        assert_eq!(session.activity.len(), 100);
        assert_eq!(session.activity[0].detail, "119");
    }

    #[test]
    fn card_identity_uses_protocol_names_and_hex() {
        let card = CardInfo::new(
            vec![0x6A, 0x31, 0x5E, 0x28],
            [0x00, 0x04],
            0x08,
            CardType::MifareClassic1K,
        );
        let identity = CardIdentity::from_card(&card, "64/64 blocks");
        assert_eq!(identity.card_type, "MIFARE Classic 1K");
        assert_eq!(identity.uid, "6A 31 5E 28");
        assert_eq!(identity.atqa, "00 04");
        assert_eq!(identity.sak, "08");
    }

    #[test]
    fn busy_and_status_transitions_are_explicit() {
        let mut session = Session::default();
        assert!(!session.is_busy());
        session.begin(Key::StatusReadingCard);
        assert!(session.is_busy());
        session.finish();
        assert!(!session.is_busy());
        session.set_status_key(StatusKind::Success, Key::StatusCardRead);
        assert_eq!(session.status.summary, Key::StatusCardRead);
    }
}
