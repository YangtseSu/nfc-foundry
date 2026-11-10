//! Language catalogs for the workbench.
//!
//! English is the base language and the fallback. Every key has an explicit
//! English and Simplified Chinese string; the exhaustive matches make an
//! incomplete catalog a compile error, and the tests reject empty values.

use std::fmt::Display;

use serde::{Deserialize, Serialize};

macro_rules! keys {
    ($($key:ident),+ $(,)?) => {
        /// One user-visible string in the workbench.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Key {
            $($key),+
        }

        impl Key {
            /// Every catalog key, used by the parity and empty-value tests.
            pub const ALL: &'static [Self] = &[$(Self::$key),+];
        }
    };
}

keys! {
    AppName, AppSubtitle,
    NavClassic, NavType2, NavSystem,
    ScreenBlocks, ScreenClassicNdef, ScreenDiff, ScreenKeys, ScreenRecovery,
    ScreenMemory, ScreenType2Ndef, ScreenProtection,
    ScreenDiagnostics, ScreenActivity, ScreenSettings,
    SelectReader, RefreshReaders, Readers, NoReaderSelected,
    NoReadersFound, ReaderConnected, ReaderDisconnected,
    ReadCard, ReadTag, Cancel, Close, Continue, Confirm, Save, Load, Export,
    Import, Add, Remove, Copy, Clear, Refresh, Browse, Back,
    Ready, Idle, Version, English, Chinese, Language, Advanced, Backup,
    Safety, TypedConfirmation, Verified, InvalidToken, NoCard, NoDump, None,

    EyebrowClassicMemory, EyebrowClassicNdef, EyebrowClassicCompare,
    EyebrowClassicKeys, EyebrowClassicRecovery, EyebrowType2Memory,
    EyebrowType2Ndef, EyebrowType2Protection, EyebrowSystemDiagnostics,
    EyebrowSystemActivity, EyebrowSettings,

    StatusReady, StatusSelectReader, StatusReaderList, StatusReaderSelected,
    StatusReadingCard, StatusReadingTag, StatusCardRead, StatusTagRead,
    StatusCardReadWarnings, StatusTagReadWarnings, StatusReadFailed,
    StatusWriteFailed, StatusRestoreFailed, StatusFormatFailed,
    StatusSaveFailed, StatusLoadFailed, StatusKeyScanFailed,
    StatusKeyScanDone, StatusKeyAdded, StatusKeyAlreadyPresent,
    StatusKeyRemoved, StatusKeySaveFailed, StatusDumpSaved,
    StatusDumpLoaded, StatusNdefWritten, StatusNdefFormatted,
    StatusBlockWritten, StatusBlockUnchanged, StatusPageWritten,
    StatusPageUnchanged, StatusRestoreComplete, StatusLockWritten,
    StatusLocksAlreadySet, StatusPasswordWritten, StatusRecoveryRunning,
    StatusRecoveryDone, StatusRecoveryCancelled, StatusRecoveryFailed,
    StatusNoDump, StatusNoCard, StatusNoPages, StatusConfirmationRequired,
    StatusInvalidInput, StatusSettingsSaved, StatusSettingsSaveFailed,
    StatusKeysSaved, StatusReaderRefreshed,

    ErrorUnsupported, ErrorInvalidArgument, ErrorIo, ErrorCancelled,
    ErrorNoCard, ErrorNoReader, ErrorTimeout, ErrorAuthentication,
    ErrorCardChanged, ErrorVerification, ErrorNotImplemented, ErrorUnknown,

    FieldPath, FieldFormat, FieldKey, FieldSector, FieldType, FieldValue,
    FieldLanguage, FieldRecordType, FieldData, FieldPages, FieldAuth0,
    FieldAccess, FieldPassword, FieldPack, FieldKnownBlock, FieldTargetBlock,
    FieldKnownKey, FieldTargetType, FieldMaxNonces, FieldTables,
    FieldKeyStore, FieldBackupDir, FieldDefaultFormat, FieldCommonKeys,
    FieldReadOnlyFormat, FieldIncludeConfiguration,
    FieldIncludeCapabilityContainer, FieldIncludeLocks, FieldConfirmToken,
    FieldBackupPath, FieldReader,

    TypeA, TypeB, TypeAny, KeyTypeA, KeyTypeB, AllSectors, AllRoles,
    FormatProject, FormatRaw, FormatMct, FormatProxmark,
    SourceCommon, SourceManual, SourceScan, SourceFile, SourceImported,
    RoleManufacturer, RoleStaticLock, RoleCapability, RoleUserMemory,
    RoleDynamicLock, RoleConfiguration, RolePassword, RolePasswordAck,
    RoleReserved, StateChanged, StateEqual, StateUnknown, IdentityChanged,
    IdentityUnchanged,

    BlocksTitle, BlocksSubtitle, CardMemory, CardMemorySub, Readability,
    CardIdentity, KeyStoreTitle, LastBackup, FilterBlocks, AllTypes,
    DataType, Trailers, ManufacturerType, ReadLegend, UnreadableLegend,
    UnknownLegend, BlockCount, SectorCount, BlockLabel, SectorLabel,
    TypeLabel, StatusLabel, KnownLabel, DataLabel, SelectBlock,
    NoCardTitle, NoCardBody, NoDumpTitle, NoDumpBody, BlockKicker,
    InspectorHex, InspectorAscii, TrailerFields, ValueBlockTitle, ValueLabel,
    AddressLabel, AccessConditions, AccessRead, AccessWrite, AccessIncrement,
    AccessDecrement, BlockZeroReadOnly, NewBlockData, WriteBlock,
    WriteTrailer, ReadBlock, ConfirmWriteTrailerTitle, ConfirmWriteTrailerBody,
    ConfirmRestoreTitle, ConfirmRestoreBody, ConfirmFormatTitle,
    ConfirmFormatBody, ConfirmWritePageTitle, ConfirmWritePageBody,
    ConfirmWriteAdvancedTitle, ConfirmWriteAdvancedBody, DumpSaved,
    DumpLoaded, RestoreOptions, ContinueOnError, DefaultDumpFormat,
    UidLabel, AtqaLabel, SakLabel, CardTypeLabel, ReadabilityDetail,
    NdefBytesLabel, BytesUnit, PagesUnit, BlocksUnit, SectorsUnit, KeysUnit,
    RecordsUnit, KnownBytes, BackupPath, NoBackup,

    ClassicNdefTitle, ClassicNdefSubtitle, NdefMessageTitle, NdefCapacity,
    NdefUsed, NdefEmpty, NdefUnformatted, NdefInvalid, NdefReadOnly,
    NdefReadWrite, NdefNotWritable, MappingTitle, MappingVersion,
    NfcSectors, DataBlocksLabel, DataArea, AccessLabel, GpbLabel, NoMad,
    FormatCard, FormatReadOnly, FormatClassicNote, WriteNdef,
    WriteTextRecord, WriteUriRecord, WriteRawFile, RecordType, RecordText,
    RecordUri, RecordFile, RecordTnf, RecordTypeLabel, RecordPayload,
    RecordLanguage, RecordIndex, TlvInfo, ClassicNdefWriteNote,
    ExportMessage, MessageBytes, CapacityBytes, SelectRecordSource, Compare,
    FieldBefore, FieldAfter, StatusCompareDone, StatusCompareFailed,
    BusyRestoring, BusyWritingBlock, BusyFormatting, BusyWritingNdef,
    BusyScanningKeys, BusyRecovering, WatchCard, DefaultTables, BusyLabel,
    BlockStatusUnread, BlockStatusRead, BlockStatusAuthFailed,
    BlockStatusReadFailed, BlockStatusSynthetic, BlockStatusVerified,

    DiffTitle, DiffSubtitle, CompareImages, BeforeImage, AfterImage,
    Replace, ChangedBlocks, EqualBlocks, UnknownBlocks, TrailerChanges,
    DiffSummary, Before, After, LayoutLabel, OnlyDifferences, NoDiffTitle,
    NoDiffBody, LoadImage, DiffDetail,

    KeysTitle, KeysSubtitle, KeyStoreSub, SectorColumn, TypeColumn,
    KeyColumn, SourceColumn, RevealKeys, HideKeys, ScanKeys, ScanTypes,
    KeyTypesAB, KeyTypesA, KeyTypesB, AddKey, AddKeyNote, KeyEntryCount,
    KeyMasked, ImportKeys, RemoveKey, KeyImportDone, KeyScanNote,

    RecoveryTitle, RecoverySubtitle, RecoveryStrategy, RecoveryStrategySub,
    StrategyAuto, StrategyAutoDesc, StrategyNested, StrategyNestedDesc,
    StrategyStaticNested, StrategyStaticNestedDesc, StrategyHardNested,
    StrategyHardNestedDesc, StrategyDarkside, StrategyDarksideDesc,
    StrategyFm11rf08s, StrategyFm11rf08sDesc, Recommended, TargetTitle,
    TargetSub, CandidateKeys, CandidateCount, RunTitle, RunSub,
    StageAcquire, StageFilter, StageSolve, StageVerify, StartRecovery,
    CancelRecovery, VerifiedKey, VerifiedKeyDetail, RecoveryNoWrites,
    RecoveryScopeNote, LastRun, ResultLabel, RecoveryIdle, RecoveryFailed,
    RecoveryCancelled, RecoveryProgressLabel, RecoveryProfile,
    RecoveryStrategyLabel, NoCandidates, RecoveryRunning, RecoveryVerified,
    RecoveryAttempts, RecoveryNote,

    Type2MemoryTitle, Type2MemorySubtitle, TagLabel, PageReadability,
    NdefBytes, ProtectionTitle, UnlockedState, LockedPagesCount,
    CfgLockClear, TagMemory, TagMemorySub, FilterPages, UserMemoryFilter,
    ConfigurationFilter, LockFilter, WritableLegend, LockedLegend,
    PageCount, PageLabel, RoleLabel, LockLabel, SelectPage, PageKicker,
    PageNotReadable, PwdPackNote, AdvancedPageNote, LockedPageNote,
    NewPageData, WritePage, WriteAdvancedPage, PageWritable, PageAdvanced,
    SaveDump, LoadDump, RestoreTag, RestoreTitle, RestoreBody,
    RestoreIncludeOptions, IncludeConfiguration, IncludeCapabilityContainer,
    IncludeLocks, Type2NdefTitle, Type2NdefSubtitle, CapacityContainer,
    CcBytes, DataAreaLabel, FormatTag, FormatTagNote, WriteNdefType2Note,
    StaticLock, DynamicLock, LockStateTitle, LockStateSub, LockedPagesLabel,
    WritableUserPages, LockPagesTitle, LockPagesSub, ApplyLocks, LockNote,
    ConfigurationTitle, ConfigurationSub, CfgLockLabel, AuthLimLabel,
    PasswordConfigTitle, PasswordConfigSub, SetPassword, PasswordNote,
    AdvancedActionPlan, AdvancedActionPlanSub, ExactTarget, OldToNew,
    BackupLabel, VerificationLabel, AdvancedActionNote, NoLockedPages,
    ReadabilityPages, ReadabilityPagesSub,

    DiagnosticsTitle, DiagnosticsSubtitle, ApplicationSection, BuildLabel,
    PlatformLabel, GuiLabel, ReadersSection, ReaderLease, PcscdLabel,
    DataSection, HardnestedTables, KeyStoreLabel, BackupsLabel, BackupMode,
    ApplicationIdentity, DesktopEntry, ExecIcon, Locales, ChecksSection,
    CardTransport, NdefCodec, RecoveryEngine, HardwareTests, CopyReport,
    CopyReportDone, NotAvailable, ReadyLabel, ExplicitGateOnly,
    ConfigurationSection,

    ActivityTitle, ActivitySubtitle, SessionActivity, SessionActivitySub,
    ClearActivity, ActivityEmpty, SafetySub, SafetyBackup,
    SafetyAdvanced, SafetyBlockZero,

    SettingsTitle, SettingsSubtitle, KeyStorePath, BackupDirectory,
    HardnestedTablesLabel, UseCommonKeys, LanguageSetting, SaveSettings,
    SettingsNote, ResetSettings,

    ConfirmAction, ConfirmKicker, TypeToken, DialogCancel, DialogContinue,
    ActionPlan, BackupBeforeChange, ReadBackPlan,
    CardPresent, CardAbsent, ReaderMenu, AdvancedActions,
    NoCardDetected, PlaceCard, NoTagTitle, NoTagBody, CardFamilyLabel,
    FamilyClassic, FamilyType2, NoKeysTitle, NoKeysBody, ReadToLoad,
    WrittenAndVerified, UnchangedValue, NoChanges, ReadOnly, Settings,
}

/// The two workbench languages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// English, the base language and fallback.
    #[default]
    En,
    /// Simplified Chinese.
    ZhCn,
}

impl Language {
    pub const ALL: [Self; 2] = [Self::En, Self::ZhCn];

    /// Stable preference-file code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhCn => "zh-CN",
        }
    }

    /// Parse a stored preference code. Unknown codes are rejected so a typo
    /// cannot silently change the language.
    pub fn from_code(code: &str) -> Option<Self> {
        match code.trim().to_ascii_lowercase().as_str() {
            "en" | "en-us" | "en-gb" => Some(Self::En),
            "zh" | "zh-cn" | "zh-hans" | "zh-sg" => Some(Self::ZhCn),
            _ => None,
        }
    }
}

/// Resolve one locale value to a supported language.
///
/// A `zh` primary tag selects `zh-CN`; every other value resolves to English
/// because English is the base language.
pub fn locale_language(value: &str) -> Option<Language> {
    let tag = value.split(['.', '@']).next().unwrap_or_default().trim();
    if tag.is_empty() {
        return None;
    }
    let primary = tag.split(['_', '-']).next().unwrap_or_default();
    if primary.eq_ignore_ascii_case("zh") {
        Some(Language::ZhCn)
    } else {
        Some(Language::En)
    }
}

/// Detect a language from a saved preference, then `LC_ALL`, `LC_MESSAGES`,
/// and `LANG`, then English.
pub fn detect_language(saved: Option<Language>, env: impl Fn(&str) -> Option<String>) -> Language {
    if let Some(saved) = saved {
        return saved;
    }
    for variable in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env(variable)
            && let Some(language) = locale_language(&value)
        {
            return language;
        }
    }
    Language::En
}

/// Detected language from the process environment and no saved preference.
pub fn default_language() -> Language {
    detect_language(None, |variable| std::env::var(variable).ok())
}

/// The workbench translator. It is a value, not a process global, so tests can
/// select a language per GPUI context.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Translator {
    language: Language,
}

impl Translator {
    pub const fn new(language: Language) -> Self {
        Self { language }
    }

    pub const fn language(self) -> Language {
        self.language
    }

    /// Localized string for a key. An empty Chinese entry falls back to
    /// English; the exhaustive catalogs make that a safety net, not a normal
    /// path.
    pub fn text(self, key: Key) -> &'static str {
        localized(self.language, key, chinese(key))
    }

    /// Format a catalog string that contains one `{}` placeholder.
    pub fn fmt1(self, key: Key, value: impl Display) -> String {
        self.text(key).replace("{}", &value.to_string())
    }

    /// Format a catalog string that contains two `{}` placeholders.
    pub fn fmt2(self, key: Key, first: impl Display, second: impl Display) -> String {
        self.text(key)
            .replacen("{}", &first.to_string(), 1)
            .replacen("{}", &second.to_string(), 1)
    }

    /// Localized count label with singular and plural keys.
    pub fn count(self, count: usize, one: Key, many: Key) -> String {
        let unit = if count == 1 {
            self.text(one)
        } else {
            self.text(many)
        };
        format!("{count} {unit}")
    }
}

/// Select a translated value, falling back to English when the Chinese entry
/// is empty. The exhaustive catalogs make the fallback a safety net.
const fn localized(language: Language, key: Key, chinese: &'static str) -> &'static str {
    match language {
        Language::En => english(key),
        Language::ZhCn => {
            if chinese.is_empty() {
                english(key)
            } else {
                chinese
            }
        }
    }
}

const fn english(key: Key) -> &'static str {
    match key {
        Key::AppName => "NFC Foundry",
        Key::AppSubtitle => "Reader workbench",
        Key::NavClassic => "MIFARE Classic",
        Key::NavType2 => "Type 2 / NTAG",
        Key::NavSystem => "System",
        Key::ScreenBlocks => "Blocks",
        Key::ScreenClassicNdef => "Classic NDEF",
        Key::ScreenDiff => "Diff",
        Key::ScreenKeys => "Keys",
        Key::ScreenRecovery => "Recovery",
        Key::ScreenMemory => "Memory",
        Key::ScreenType2Ndef => "NDEF",
        Key::ScreenProtection => "Protection",
        Key::ScreenDiagnostics => "Diagnostics",
        Key::ScreenActivity => "Activity",
        Key::ScreenSettings => "Settings",
        Key::SelectReader => "Select a reader",
        Key::RefreshReaders => "Refresh readers",
        Key::Readers => "Readers",
        Key::NoReaderSelected => "No reader selected",
        Key::NoReadersFound => "No readers found. Connect a PN532 or ACR122U, then refresh.",
        Key::ReaderConnected => "Connected",
        Key::ReaderDisconnected => "Not connected",
        Key::ReadCard => "Read card",
        Key::ReadTag => "Read tag",
        Key::Cancel => "Cancel",
        Key::Close => "Close",
        Key::Continue => "Continue",
        Key::Confirm => "Confirm",
        Key::Save => "Save",
        Key::Load => "Load",
        Key::Export => "Export",
        Key::Import => "Import",
        Key::Add => "Add",
        Key::Remove => "Remove",
        Key::Copy => "Copy",
        Key::Clear => "Clear",
        Key::Refresh => "Refresh",
        Key::Browse => "Browse",
        Key::Back => "Back",
        Key::Ready => "Ready",
        Key::Idle => "Idle",
        Key::Version => "Version",
        Key::English => "English",
        Key::Chinese => "简体中文",
        Key::Language => "Language",
        Key::Advanced => "Advanced",
        Key::Backup => "Backup",
        Key::Safety => "Safety",
        Key::TypedConfirmation => "Typed confirmation",
        Key::Verified => "verified",
        Key::InvalidToken => "The confirmation text does not match.",
        Key::NoCard => "No card",
        Key::NoDump => "No card image",
        Key::None => "none",

        Key::EyebrowClassicMemory => "Classic / Memory",
        Key::EyebrowClassicNdef => "Classic / NDEF",
        Key::EyebrowClassicCompare => "Classic / Compare",
        Key::EyebrowClassicKeys => "Classic / Keys",
        Key::EyebrowClassicRecovery => "Classic / Recovery",
        Key::EyebrowType2Memory => "Type 2 / Memory",
        Key::EyebrowType2Ndef => "Type 2 / NDEF",
        Key::EyebrowType2Protection => "Type 2 / Protection",
        Key::EyebrowSystemDiagnostics => "System / Diagnostics",
        Key::EyebrowSystemActivity => "System / Activity",
        Key::EyebrowSettings => "System / Settings",

        Key::StatusReady => "Ready",
        Key::StatusSelectReader => "Select a reader and read a card.",
        Key::StatusReaderList => "Reader list failed",
        Key::StatusReaderSelected => "Selected reader",
        Key::StatusReadingCard => "Reading card",
        Key::StatusReadingTag => "Reading tag",
        Key::StatusCardRead => "Card read",
        Key::StatusTagRead => "Tag read",
        Key::StatusCardReadWarnings => "Card read with warnings",
        Key::StatusTagReadWarnings => "Tag read with warnings",
        Key::StatusReadFailed => "Read failed",
        Key::StatusWriteFailed => "Write failed",
        Key::StatusRestoreFailed => "Restore failed",
        Key::StatusFormatFailed => "Format failed",
        Key::StatusSaveFailed => "Couldn't save",
        Key::StatusLoadFailed => "Couldn't load",
        Key::StatusKeyScanFailed => "Key scan failed",
        Key::StatusKeyScanDone => "Key scan complete",
        Key::StatusKeyAdded => "Key added",
        Key::StatusKeyAlreadyPresent => "Key already present",
        Key::StatusKeyRemoved => "Key removed",
        Key::StatusKeySaveFailed => "Key changes could not be saved",
        Key::StatusDumpSaved => "Dump saved",
        Key::StatusDumpLoaded => "Dump loaded",
        Key::StatusNdefWritten => "NDEF written and verified",
        Key::StatusNdefFormatted => "NDEF formatted",
        Key::StatusBlockWritten => "Block written and verified",
        Key::StatusBlockUnchanged => "Block already matches",
        Key::StatusPageWritten => "Page written and verified",
        Key::StatusPageUnchanged => "Page already matches",
        Key::StatusRestoreComplete => "Restore complete",
        Key::StatusLockWritten => "Lock bits written and verified",
        Key::StatusLocksAlreadySet => "The requested pages are already locked",
        Key::StatusPasswordWritten => "Password configuration written and verified",
        Key::StatusRecoveryRunning => "Recovery running",
        Key::StatusRecoveryDone => "Recovery complete",
        Key::StatusRecoveryCancelled => "Recovery cancelled",
        Key::StatusRecoveryFailed => "Recovery failed",
        Key::StatusNoDump => "Read or load a card first.",
        Key::StatusNoCard => "Read a card first.",
        Key::StatusNoPages => "Select at least one page.",
        Key::StatusConfirmationRequired => "This action needs confirmation.",
        Key::StatusInvalidInput => "Invalid value",
        Key::StatusSettingsSaved => "Settings saved",
        Key::StatusSettingsSaveFailed => "Settings could not be saved",
        Key::StatusKeysSaved => "Keys saved",
        Key::StatusReaderRefreshed => "Reader list refreshed",

        Key::ErrorUnsupported => "Unsupported operation",
        Key::ErrorInvalidArgument => "Invalid value",
        Key::ErrorIo => "File error",
        Key::ErrorCancelled => "Cancelled",
        Key::ErrorNoCard => "No card is present",
        Key::ErrorNoReader => "No reader is selected",
        Key::ErrorTimeout => "Reader timed out",
        Key::ErrorAuthentication => "Authentication failed",
        Key::ErrorCardChanged => "The card identity changed",
        Key::ErrorVerification => "Verification failed",
        Key::ErrorNotImplemented => "Not implemented",
        Key::ErrorUnknown => "Operation failed",

        Key::FieldPath => "Path",
        Key::FieldFormat => "Format",
        Key::FieldKey => "Key",
        Key::FieldSector => "Sector",
        Key::FieldType => "Type",
        Key::FieldValue => "Value",
        Key::FieldLanguage => "Language",
        Key::FieldRecordType => "Record type",
        Key::FieldData => "Data",
        Key::FieldPages => "Pages",
        Key::FieldAuth0 => "AUTH0",
        Key::FieldAccess => "ACCESS",
        Key::FieldPassword => "PWD",
        Key::FieldPack => "PACK",
        Key::FieldKnownBlock => "Known block",
        Key::FieldTargetBlock => "Target block",
        Key::FieldKnownKey => "Known key",
        Key::FieldTargetType => "Target key type",
        Key::FieldMaxNonces => "Max nonces",
        Key::FieldTables => "Table directory",
        Key::FieldKeyStore => "Key store",
        Key::FieldBackupDir => "Backup directory",
        Key::FieldDefaultFormat => "Default dump format",
        Key::FieldCommonKeys => "Common keys",
        Key::FieldReadOnlyFormat => "Read-only after format",
        Key::FieldIncludeConfiguration => "Include configuration",
        Key::FieldIncludeCapabilityContainer => "Include capability container",
        Key::FieldIncludeLocks => "Include lock bytes",
        Key::FieldConfirmToken => "Confirmation",
        Key::FieldBackupPath => "Backup path",
        Key::FieldReader => "Reader",

        Key::TypeA => "A",
        Key::TypeB => "B",
        Key::TypeAny => "Any",
        Key::KeyTypeA => "Key A",
        Key::KeyTypeB => "Key B",
        Key::AllSectors => "All sectors",
        Key::AllRoles => "All roles",
        Key::FormatProject => "Project",
        Key::FormatRaw => "Raw",
        Key::FormatMct => "MCT",
        Key::FormatProxmark => "Proxmark3",
        Key::SourceCommon => "common",
        Key::SourceManual => "manual",
        Key::SourceScan => "scan",
        Key::SourceFile => "key file",
        Key::SourceImported => "imported",
        Key::RoleManufacturer => "Manufacturer",
        Key::RoleStaticLock => "Static lock",
        Key::RoleCapability => "Capability",
        Key::RoleUserMemory => "User memory",
        Key::RoleDynamicLock => "Dynamic lock",
        Key::RoleConfiguration => "Configuration",
        Key::RolePassword => "Password",
        Key::RolePasswordAck => "Password ack",
        Key::RoleReserved => "Reserved",
        Key::StateChanged => "changed",
        Key::StateEqual => "equal",
        Key::StateUnknown => "unknown",
        Key::IdentityChanged => "card identity changed",
        Key::IdentityUnchanged => "identity unchanged",

        Key::BlocksTitle => "Blocks",
        Key::BlocksSubtitle => "MIFARE Classic · blocks, sectors, and trailers",
        Key::CardMemory => "Card memory",
        Key::CardMemorySub => "Manufacturer, data, and trailer blocks",
        Key::Readability => "Readability",
        Key::CardIdentity => "Card identity",
        Key::KeyStoreTitle => "Key store",
        Key::LastBackup => "Last backup",
        Key::FilterBlocks => "Filter blocks",
        Key::AllTypes => "All types",
        Key::DataType => "Data",
        Key::Trailers => "Trailers",
        Key::ManufacturerType => "Manufacturer",
        Key::ReadLegend => "read",
        Key::UnreadableLegend => "unreadable",
        Key::UnknownLegend => "unknown",
        Key::BlockCount => "blocks",
        Key::SectorCount => "sectors",
        Key::BlockLabel => "Block",
        Key::SectorLabel => "Sector",
        Key::TypeLabel => "Type",
        Key::StatusLabel => "Status",
        Key::KnownLabel => "Known",
        Key::DataLabel => "Data",
        Key::SelectBlock => "Select a block.",
        Key::NoCardTitle => "No card image",
        Key::NoCardBody => "Read a MIFARE Classic card or load a dump file.",
        Key::NoDumpTitle => "No card image",
        Key::NoDumpBody => "Read a card or load a dump file.",
        Key::BlockKicker => "Block {} · sector {}",
        Key::InspectorHex => "Hex",
        Key::InspectorAscii => "ASCII",
        Key::TrailerFields => "Trailer fields",
        Key::ValueBlockTitle => "Value block",
        Key::ValueLabel => "Value",
        Key::AddressLabel => "Address",
        Key::AccessConditions => "Access conditions",
        Key::AccessRead => "read",
        Key::AccessWrite => "write",
        Key::AccessIncrement => "increment",
        Key::AccessDecrement => "decrement",
        Key::BlockZeroReadOnly => "Block 0 is read-only in this build. UID writes are not exposed.",
        Key::NewBlockData => "New block data",
        Key::WriteBlock => "Write block",
        Key::WriteTrailer => "Write trailer…",
        Key::ReadBlock => "Read block",
        Key::ConfirmWriteTrailerTitle => "Write sector trailer?",
        Key::ConfirmWriteTrailerBody => {
            "Block {} is the trailer for sector {}. Writing it can change keys, access bits, or make the sector unreadable. A project backup is written first."
        }
        Key::ConfirmRestoreTitle => "Restore this dump?",
        Key::ConfirmRestoreBody => {
            "The selected card is overwritten with the loaded image. A project backup is written before the first change."
        }
        Key::ConfirmFormatTitle => "Format Classic NDEF?",
        Key::ConfirmFormatBody => {
            "Every NFC sector is rewritten. Keys and access bits change permanently. A project backup is written first."
        }
        Key::ConfirmWritePageTitle => "Write page {}?",
        Key::ConfirmWritePageBody => {
            "Page {} is {}. The write is verified by read-back and a project backup is written first."
        }
        Key::ConfirmWriteAdvancedTitle => "Write Advanced page?",
        Key::ConfirmWriteAdvancedBody => {
            "Page {} is {}. This is an Advanced write. A project backup is written first and the page is verified by read-back."
        }
        Key::DumpSaved => "Dump saved",
        Key::DumpLoaded => "Dump loaded",
        Key::RestoreOptions => "Restore options",
        Key::ContinueOnError => "Continue after a failed block",
        Key::DefaultDumpFormat => "Default dump format",
        Key::UidLabel => "UID",
        Key::AtqaLabel => "ATQA",
        Key::SakLabel => "SAK",
        Key::CardTypeLabel => "Card type",
        Key::ReadabilityDetail => "readable",
        Key::NdefBytesLabel => "NDEF bytes",
        Key::BytesUnit => "bytes",
        Key::PagesUnit => "pages",
        Key::BlocksUnit => "blocks",
        Key::SectorsUnit => "sectors",
        Key::KeysUnit => "keys",
        Key::RecordsUnit => "records",
        Key::KnownBytes => "bytes known",
        Key::BackupPath => "Backup",
        Key::NoBackup => "No backup yet",

        Key::ClassicNdefTitle => "Classic NDEF",
        Key::ClassicNdefSubtitle => "MAD mapping, NDEF TLV, and record writes",
        Key::NdefMessageTitle => "NDEF message",
        Key::NdefCapacity => "NDEF capacity",
        Key::NdefUsed => "bytes used",
        Key::NdefEmpty => "empty",
        Key::NdefUnformatted => "unformatted",
        Key::NdefInvalid => "invalid",
        Key::NdefReadOnly => "read-only",
        Key::NdefReadWrite => "read / write",
        Key::NdefNotWritable => "not writable",
        Key::MappingTitle => "Mapping",
        Key::MappingVersion => "Mapping version",
        Key::NfcSectors => "NFC sectors",
        Key::DataBlocksLabel => "Data blocks",
        Key::DataArea => "Data area",
        Key::AccessLabel => "Access",
        Key::GpbLabel => "GPB",
        Key::NoMad => "No MAD is present. Format the card to create MAD1/MAD2 and NFC sectors.",
        Key::FormatCard => "Format card",
        Key::FormatReadOnly => "Read-only after format",
        Key::FormatClassicNote => {
            "Formatting rewrites the MAD and every NFC sector, then changes sector keys and access bits. A backup is written first."
        }
        Key::WriteNdef => "Write NDEF",
        Key::WriteTextRecord => "Text",
        Key::WriteUriRecord => "URI",
        Key::WriteRawFile => "Raw file",
        Key::RecordType => "Record type",
        Key::RecordText => "Text",
        Key::RecordUri => "URI",
        Key::RecordFile => "File",
        Key::RecordTnf => "TNF",
        Key::RecordTypeLabel => "type",
        Key::RecordPayload => "payload",
        Key::RecordLanguage => "language",
        Key::RecordIndex => "record {}",
        Key::TlvInfo => {
            "TLV parsed · terminator and padding preserved · message verified after write."
        }
        Key::ClassicNdefWriteNote => {
            "Writes the TLV, verifies every byte, and keeps a pre-write project backup."
        }
        Key::ExportMessage => "Export message",
        Key::MessageBytes => "message bytes",
        Key::CapacityBytes => "capacity",
        Key::SelectRecordSource => "Enter a value or choose a file.",

        Key::DiffTitle => "Diff",
        Key::DiffSubtitle => "Two card images · changed blocks and trailers",
        Key::CompareImages => "Compare images",
        Key::BeforeImage => "Before image",
        Key::AfterImage => "After image",
        Key::Replace => "Replace",
        Key::ChangedBlocks => "changed blocks",
        Key::EqualBlocks => "equal blocks",
        Key::UnknownBlocks => "unknown blocks",
        Key::TrailerChanges => "trailer changes",
        Key::DiffSummary => "Diff summary",
        Key::Before => "Before",
        Key::After => "After",
        Key::LayoutLabel => "Layout",
        Key::OnlyDifferences => "Only differences are listed",
        Key::NoDiffTitle => "No comparison",
        Key::NoDiffBody => "Read the card twice, or load two dump files.",
        Key::LoadImage => "Load image",
        Key::DiffDetail => {
            "Trailer changes are highlighted separately because they alter keys and access conditions."
        }

        Key::KeysTitle => "Key store",
        Key::KeysSubtitle => "Sector candidates and provenance · shared with nfctl",
        Key::KeyStoreSub => "Sector-bound and wildcard candidates · stored at 0600",
        Key::SectorColumn => "Sector",
        Key::TypeColumn => "Type",
        Key::KeyColumn => "Key",
        Key::SourceColumn => "Source",
        Key::RevealKeys => "Reveal keys",
        Key::HideKeys => "Hide keys",
        Key::ScanKeys => "Scan keys",
        Key::ScanTypes => "Key types",
        Key::KeyTypesAB => "A + B",
        Key::KeyTypesA => "Key A",
        Key::KeyTypesB => "Key B",
        Key::AddKey => "Add a key",
        Key::AddKeyNote => {
            "A key becomes trusted only after the card accepts it and a trailer read succeeds."
        }
        Key::KeyEntryCount => "entries",
        Key::KeyMasked => "•••• •••• ••••",
        Key::ImportKeys => "Import keys",
        Key::RemoveKey => "Remove key",
        Key::KeyImportDone => "Keys imported",
        Key::KeyScanNote => {
            "The key store is shared by the GUI and nfctl; it never lives inside a dump."
        }

        Key::RecoveryTitle => "Recovery",
        Key::RecoverySubtitle => "Native Crypto1 engine · authentication only · no card writes",
        Key::RecoveryStrategy => "Recovery strategy",
        Key::RecoveryStrategySub => "Native engine · authentication only · read-only",
        Key::StrategyAuto => "Auto",
        Key::StrategyAutoDesc => {
            "Classify the card, then try dictionary, nested, static-nested, or darkside."
        }
        Key::StrategyNested => "Nested",
        Key::StrategyNestedDesc => {
            "Use a known sector key to recover an unknown key on another sector."
        }
        Key::StrategyStaticNested => "Static nested",
        Key::StrategyStaticNestedDesc => "Nested path for cards with a static encrypted nonce.",
        Key::StrategyHardNested => "Hardnested",
        Key::StrategyHardNestedDesc => "Bit-sliced solver over PM3 tables. Uses more CPU and time.",
        Key::StrategyDarkside => "Darkside",
        Key::StrategyDarksideDesc => "Card-only path when no known key is available.",
        Key::StrategyFm11rf08s => "FM11RF08S",
        Key::StrategyFm11rf08sDesc => {
            "Detect the static encrypted nonce profile and explain limitations."
        }
        Key::Recommended => "recommended",
        Key::TargetTitle => "Target",
        Key::TargetSub => "Sector keys are derived from the selected blocks",
        Key::CandidateKeys => "Candidate keys",
        Key::CandidateCount => "stored keys",
        Key::RunTitle => "Run",
        Key::RunSub => "Acquire → filter → solve → verify",
        Key::StageAcquire => "Acquire encrypted nonces",
        Key::StageFilter => "Filter state pairs",
        Key::StageSolve => "Bit-sliced solver",
        Key::StageVerify => "Verify key on card",
        Key::StartRecovery => "Start recovery",
        Key::CancelRecovery => "Cancel",
        Key::VerifiedKey => "Verified key",
        Key::VerifiedKeyDetail => "Fresh authentication succeeded after recovery.",
        Key::RecoveryNoWrites => "no writes",
        Key::RecoveryScopeNote => {
            "Recovery authenticates but never writes a block. Use only cards you own."
        }
        Key::LastRun => "Last run",
        Key::ResultLabel => "Result",
        Key::RecoveryIdle => "Idle",
        Key::RecoveryFailed => "Recovery failed",
        Key::RecoveryCancelled => "Recovery cancelled",
        Key::RecoveryProgressLabel => "Progress",
        Key::RecoveryProfile => "profile",
        Key::RecoveryStrategyLabel => "strategy",
        Key::NoCandidates => "No candidate keys. Nested and hardnested recovery need a known key.",
        Key::RecoveryRunning => "Recovery running",
        Key::RecoveryVerified => "Verified",
        Key::RecoveryAttempts => "attempts",
        Key::RecoveryNote => "Recovery note",

        Key::Type2MemoryTitle => "Memory",
        Key::Type2MemorySubtitle => "NTAG · pages, roles, and one-way locks",
        Key::TagLabel => "Tag",
        Key::PageReadability => "Readability",
        Key::NdefBytes => "NDEF",
        Key::ProtectionTitle => "Protection",
        Key::UnlockedState => "Unlocked",
        Key::LockedPagesCount => "locked pages",
        Key::CfgLockClear => "CFGLCK clear",
        Key::TagMemory => "Tag memory",
        Key::TagMemorySub => "Roles and one-way locks",
        Key::FilterPages => "Filter pages",
        Key::UserMemoryFilter => "User memory",
        Key::ConfigurationFilter => "Configuration",
        Key::LockFilter => "Lock pages",
        Key::WritableLegend => "writable",
        Key::LockedLegend => "locked",
        Key::PageCount => "pages",
        Key::PageLabel => "Page",
        Key::RoleLabel => "Role",
        Key::LockLabel => "Lock",
        Key::SelectPage => "Select a page.",
        Key::PageKicker => "Page {}",
        Key::PageNotReadable => "not readable",
        Key::PwdPackNote => "PWD and PACK never read back. A write is verified with PWD_AUTH.",
        Key::AdvancedPageNote => "{} pages need an Advanced confirmation and a backup.",
        Key::LockedPageNote => "This page is locked and cannot be written.",
        Key::NewPageData => "New page data",
        Key::WritePage => "Write page",
        Key::WriteAdvancedPage => "Write page…",
        Key::PageWritable => "writable",
        Key::PageAdvanced => "advanced",
        Key::SaveDump => "Save dump",
        Key::LoadDump => "Load dump",
        Key::RestoreTag => "Restore…",
        Key::RestoreTitle => "Restore this dump?",
        Key::RestoreBody => {
            "User memory is overwritten with the loaded image. A project backup is written before the first change."
        }
        Key::RestoreIncludeOptions => "Restore include options",
        Key::IncludeConfiguration => "Configuration",
        Key::IncludeCapabilityContainer => "Capability container",
        Key::IncludeLocks => "Lock bytes",
        Key::Type2NdefTitle => "NDEF",
        Key::Type2NdefSubtitle => "Type 2 tag · capability container read/write",
        Key::CapacityContainer => "Capability container",
        Key::CcBytes => "CC bytes",
        Key::DataAreaLabel => "Data area",
        Key::FormatTag => "Format tag",
        Key::FormatTagNote => {
            "Rewrites the user-memory NDEF TLV. A backup is written before the first change."
        }
        Key::WriteNdefType2Note => {
            "Writes NLEN last, then verifies NLEN and the message by read-back."
        }
        Key::StaticLock => "Static lock",
        Key::DynamicLock => "Dynamic lock",
        Key::LockStateTitle => "Lock state",
        Key::LockStateSub => "One-way lock bytes · pages 2 and 130",
        Key::LockedPagesLabel => "Locked pages",
        Key::WritableUserPages => "Writable user pages",
        Key::LockPagesTitle => "Lock pages",
        Key::LockPagesSub => "Advanced · typed confirmation required",
        Key::ApplyLocks => "Apply locks…",
        Key::LockNote => {
            "Lock bits are one-way. A backup is written before the first lock; the action cannot be undone."
        }
        Key::ConfigurationTitle => "Configuration",
        Key::ConfigurationSub => "AUTH0, ACCESS, CFGLCK, and AUTHLIM",
        Key::CfgLockLabel => "CFGLCK",
        Key::AuthLimLabel => "AUTHLIM",
        Key::PasswordConfigTitle => "Password configuration",
        Key::PasswordConfigSub => "Advanced · PWD/PACK never read back",
        Key::SetPassword => "Set password…",
        Key::PasswordNote => {
            "Writing AUTH0/ACCESS can make the tag inaccessible without the password. A backup is written first."
        }
        Key::AdvancedActionPlan => "Advanced action plan",
        Key::AdvancedActionPlanSub => "Shown in the confirmation dialog before any write",
        Key::ExactTarget => "Exact target",
        Key::OldToNew => "Old → new",
        Key::BackupLabel => "Backup",
        Key::VerificationLabel => "Verification",
        Key::AdvancedActionNote => {
            "Every Advanced action names the exact pages, old and new values, backup path, and read-back plan before confirmation."
        }
        Key::NoLockedPages => "No locked pages",
        Key::ReadabilityPages => "pages readable",
        Key::ReadabilityPagesSub => "completely read",

        Key::DiagnosticsTitle => "Diagnostics",
        Key::DiagnosticsSubtitle => "Version, readers, resources, and runtime checks",
        Key::ApplicationSection => "Application",
        Key::BuildLabel => "Build",
        Key::PlatformLabel => "Platform",
        Key::GuiLabel => "GUI",
        Key::ReadersSection => "Readers",
        Key::ReaderLease => "Reader lease",
        Key::PcscdLabel => "pcscd",
        Key::DataSection => "Data",
        Key::HardnestedTables => "Hardnested tables",
        Key::KeyStoreLabel => "Key store",
        Key::BackupsLabel => "Backups",
        Key::BackupMode => "Backup mode",
        Key::ApplicationIdentity => "Application identity",
        Key::DesktopEntry => "Desktop entry",
        Key::ExecIcon => "Exec / Icon",
        Key::Locales => "Locales",
        Key::ChecksSection => "Checks",
        Key::CardTransport => "Card transport",
        Key::NdefCodec => "NDEF codec",
        Key::RecoveryEngine => "Recovery engine",
        Key::HardwareTests => "Hardware tests",
        Key::CopyReport => "Copy report",
        Key::CopyReportDone => "Report copied",
        Key::NotAvailable => "not available",
        Key::ReadyLabel => "ready",
        Key::ExplicitGateOnly => "explicit gate only",
        Key::ConfigurationSection => "Configuration",

        Key::ActivityTitle => "Activity",
        Key::ActivitySubtitle => "Operations in this session · backup and verification results",
        Key::SessionActivity => "Session activity",
        Key::SessionActivitySub => "Operations in this window · newest first",
        Key::ClearActivity => "Clear",
        Key::ActivityEmpty => "No operations in this session yet.",
        Key::SafetySub => "Rules applied to every mutation",
        Key::SafetyBackup => {
            "Every write creates a project backup, writes, reads back, and checks card identity."
        }
        Key::SafetyAdvanced => {
            "Lock, configuration, password, and format actions require typed confirmation."
        }
        Key::SafetyBlockZero => {
            "Block 0 is read-only in this build. UID and factory-format actions are not exposed."
        }

        Key::SettingsTitle => "Settings",
        Key::SettingsSubtitle => "Workbench paths, formats, and language",
        Key::KeyStorePath => "Key store path",
        Key::BackupDirectory => "Backup directory",
        Key::HardnestedTablesLabel => "Hardnested tables",
        Key::UseCommonKeys => "Try the built-in common keys",
        Key::LanguageSetting => "Language",
        Key::SaveSettings => "Save settings",
        Key::SettingsNote => {
            "The key store and backup directory are shared with nfctl. The language preference only affects the GUI."
        }
        Key::ResetSettings => "Reset to defaults",

        Key::ConfirmAction => "Confirm action",
        Key::ConfirmKicker => "Confirm",
        Key::TypeToken => "Type I_OWN_THIS_CARD to continue",
        Key::DialogCancel => "Cancel",
        Key::DialogContinue => "Continue",
        Key::ActionPlan => "Action plan",
        Key::BackupBeforeChange => "A project backup is written before the first change.",
        Key::ReadBackPlan => {
            "Every written byte is verified by read-back and a card identity check."
        }
        Key::CardPresent => "Card present",
        Key::CardAbsent => "No card present",
        Key::NoCardDetected => "No card detected",
        Key::PlaceCard => "Place a card on the reader to start.",
        Key::NoTagTitle => "No tag image",
        Key::NoTagBody => "Read a Type 2 tag or load a dump file.",
        Key::CardFamilyLabel => "Card",
        Key::FamilyClassic => "Classic",
        Key::FamilyType2 => "Type 2",
        Key::NoKeysTitle => "No keys stored yet",
        Key::NoKeysBody => "Scan the card or add a key on the right.",
        Key::ReadToLoad => "Press Read to load the image.",
        Key::ReaderMenu => "Reader",
        Key::AdvancedActions => "Advanced actions",
        Key::WrittenAndVerified => "written and verified",
        Key::UnchangedValue => "already matches",
        Key::NoChanges => "No changes",
        Key::ReadOnly => "read-only",
        Key::Settings => "Settings",
        Key::Compare => "Compare",
        Key::FieldBefore => "Before file",
        Key::FieldAfter => "After file",
        Key::StatusCompareDone => "Comparison complete",
        Key::StatusCompareFailed => "Comparison failed",
        Key::BusyRestoring => "Restoring",
        Key::BusyWritingBlock => "Writing block",
        Key::BusyFormatting => "Formatting",
        Key::BusyWritingNdef => "Writing NDEF",
        Key::BusyScanningKeys => "Scanning keys",
        Key::BusyRecovering => "Recovering",
        Key::WatchCard => "Watch card presence",
        Key::DefaultTables => "installed default",
        Key::BusyLabel => "busy",
        Key::BlockStatusUnread => "unread",
        Key::BlockStatusRead => "read",
        Key::BlockStatusAuthFailed => "auth failed",
        Key::BlockStatusReadFailed => "read failed",
        Key::BlockStatusSynthetic => "synthetic",
        Key::BlockStatusVerified => "verified",
    }
}

const fn chinese(key: Key) -> &'static str {
    match key {
        Key::AppName => "NFC Foundry",
        Key::AppSubtitle => "读卡工作台",
        Key::NavClassic => "MIFARE Classic",
        Key::NavType2 => "Type 2 / NTAG",
        Key::NavSystem => "系统",
        Key::ScreenBlocks => "数据块",
        Key::ScreenClassicNdef => "Classic NDEF",
        Key::ScreenDiff => "差异",
        Key::ScreenKeys => "密钥",
        Key::ScreenRecovery => "恢复",
        Key::ScreenMemory => "存储页",
        Key::ScreenType2Ndef => "NDEF",
        Key::ScreenProtection => "保护",
        Key::ScreenDiagnostics => "诊断",
        Key::ScreenActivity => "活动",
        Key::ScreenSettings => "设置",
        Key::SelectReader => "选择读卡器",
        Key::RefreshReaders => "刷新读卡器",
        Key::Readers => "读卡器",
        Key::NoReaderSelected => "未选择读卡器",
        Key::NoReadersFound => "未找到读卡器。连接 PN532 或 ACR122U 后刷新。",
        Key::ReaderConnected => "已连接",
        Key::ReaderDisconnected => "未连接",
        Key::ReadCard => "读取卡片",
        Key::ReadTag => "读取标签",
        Key::Cancel => "取消",
        Key::Close => "关闭",
        Key::Continue => "继续",
        Key::Confirm => "确认",
        Key::Save => "保存",
        Key::Load => "加载",
        Key::Export => "导出",
        Key::Import => "导入",
        Key::Add => "添加",
        Key::Remove => "移除",
        Key::Copy => "复制",
        Key::Clear => "清除",
        Key::Refresh => "刷新",
        Key::Browse => "浏览",
        Key::Back => "返回",
        Key::Ready => "就绪",
        Key::Idle => "空闲",
        Key::Version => "版本",
        Key::English => "English",
        Key::Chinese => "简体中文",
        Key::Language => "语言",
        Key::Advanced => "高级",
        Key::Backup => "备份",
        Key::Safety => "安全",
        Key::TypedConfirmation => "输入确认",
        Key::Verified => "已验证",
        Key::InvalidToken => "确认文本不匹配。",
        Key::NoCard => "无卡片",
        Key::NoDump => "无卡片镜像",
        Key::None => "无",

        Key::EyebrowClassicMemory => "Classic / 存储",
        Key::EyebrowClassicNdef => "Classic / NDEF",
        Key::EyebrowClassicCompare => "Classic / 比较",
        Key::EyebrowClassicKeys => "Classic / 密钥",
        Key::EyebrowClassicRecovery => "Classic / 恢复",
        Key::EyebrowType2Memory => "Type 2 / 存储",
        Key::EyebrowType2Ndef => "Type 2 / NDEF",
        Key::EyebrowType2Protection => "Type 2 / 保护",
        Key::EyebrowSystemDiagnostics => "系统 / 诊断",
        Key::EyebrowSystemActivity => "系统 / 活动",
        Key::EyebrowSettings => "系统 / 设置",

        Key::StatusReady => "就绪",
        Key::StatusSelectReader => "选择读卡器并读取卡片。",
        Key::StatusReaderList => "读卡器列表读取失败",
        Key::StatusReaderSelected => "已选择读卡器",
        Key::StatusReadingCard => "正在读取卡片",
        Key::StatusReadingTag => "正在读取标签",
        Key::StatusCardRead => "卡片读取完成",
        Key::StatusTagRead => "标签读取完成",
        Key::StatusCardReadWarnings => "卡片读取完成，但有警告",
        Key::StatusTagReadWarnings => "标签读取完成，但有警告",
        Key::StatusReadFailed => "读取失败",
        Key::StatusWriteFailed => "写入失败",
        Key::StatusRestoreFailed => "恢复失败",
        Key::StatusFormatFailed => "格式化失败",
        Key::StatusSaveFailed => "保存失败",
        Key::StatusLoadFailed => "加载失败",
        Key::StatusKeyScanFailed => "密钥扫描失败",
        Key::StatusKeyScanDone => "密钥扫描完成",
        Key::StatusKeyAdded => "已添加密钥",
        Key::StatusKeyAlreadyPresent => "密钥已存在",
        Key::StatusKeyRemoved => "已移除密钥",
        Key::StatusKeySaveFailed => "密钥更改未能保存",
        Key::StatusDumpSaved => "转储已保存",
        Key::StatusDumpLoaded => "转储已加载",
        Key::StatusNdefWritten => "NDEF 已写入并校验",
        Key::StatusNdefFormatted => "NDEF 已格式化",
        Key::StatusBlockWritten => "块已写入并校验",
        Key::StatusBlockUnchanged => "块内容已相同",
        Key::StatusPageWritten => "页已写入并校验",
        Key::StatusPageUnchanged => "页内容已相同",
        Key::StatusRestoreComplete => "恢复完成",
        Key::StatusLockWritten => "锁定位已写入并校验",
        Key::StatusLocksAlreadySet => "所选页已锁定",
        Key::StatusPasswordWritten => "密码配置已写入并校验",
        Key::StatusRecoveryRunning => "正在恢复",
        Key::StatusRecoveryDone => "恢复完成",
        Key::StatusRecoveryCancelled => "恢复已取消",
        Key::StatusRecoveryFailed => "恢复失败",
        Key::StatusNoDump => "请先读取或加载卡片。",
        Key::StatusNoCard => "请先读取卡片。",
        Key::StatusNoPages => "请至少选择一页。",
        Key::StatusConfirmationRequired => "此操作需要确认。",
        Key::StatusInvalidInput => "输入无效",
        Key::StatusSettingsSaved => "设置已保存",
        Key::StatusSettingsSaveFailed => "设置未能保存",
        Key::StatusKeysSaved => "密钥已保存",
        Key::StatusReaderRefreshed => "读卡器列表已刷新",

        Key::ErrorUnsupported => "不支持的操作",
        Key::ErrorInvalidArgument => "输入无效",
        Key::ErrorIo => "文件错误",
        Key::ErrorCancelled => "已取消",
        Key::ErrorNoCard => "未检测到卡片",
        Key::ErrorNoReader => "未选择读卡器",
        Key::ErrorTimeout => "读卡器超时",
        Key::ErrorAuthentication => "认证失败",
        Key::ErrorCardChanged => "卡片标识已改变",
        Key::ErrorVerification => "校验失败",
        Key::ErrorNotImplemented => "尚未实现",
        Key::ErrorUnknown => "操作失败",

        Key::FieldPath => "路径",
        Key::FieldFormat => "格式",
        Key::FieldKey => "密钥",
        Key::FieldSector => "扇区",
        Key::FieldType => "类型",
        Key::FieldValue => "值",
        Key::FieldLanguage => "语言",
        Key::FieldRecordType => "记录类型",
        Key::FieldData => "数据",
        Key::FieldPages => "页",
        Key::FieldAuth0 => "AUTH0",
        Key::FieldAccess => "ACCESS",
        Key::FieldPassword => "PWD",
        Key::FieldPack => "PACK",
        Key::FieldKnownBlock => "已知块",
        Key::FieldTargetBlock => "目标块",
        Key::FieldKnownKey => "已知密钥",
        Key::FieldTargetType => "目标密钥类型",
        Key::FieldMaxNonces => "最大随机数",
        Key::FieldTables => "表目录",
        Key::FieldKeyStore => "密钥库",
        Key::FieldBackupDir => "备份目录",
        Key::FieldDefaultFormat => "默认转储格式",
        Key::FieldCommonKeys => "常用密钥",
        Key::FieldReadOnlyFormat => "格式化后设为只读",
        Key::FieldIncludeConfiguration => "包含配置页",
        Key::FieldIncludeCapabilityContainer => "包含能力容器",
        Key::FieldIncludeLocks => "包含锁定位",
        Key::FieldConfirmToken => "确认",
        Key::FieldBackupPath => "备份路径",
        Key::FieldReader => "读卡器",

        Key::TypeA => "A",
        Key::TypeB => "B",
        Key::TypeAny => "任意",
        Key::KeyTypeA => "Key A",
        Key::KeyTypeB => "Key B",
        Key::AllSectors => "所有扇区",
        Key::AllRoles => "所有用途",
        Key::FormatProject => "项目",
        Key::FormatRaw => "原始",
        Key::FormatMct => "MCT",
        Key::FormatProxmark => "Proxmark3",
        Key::SourceCommon => "常用",
        Key::SourceManual => "手动",
        Key::SourceScan => "扫描",
        Key::SourceFile => "密钥文件",
        Key::SourceImported => "导入",
        Key::RoleManufacturer => "厂商块",
        Key::RoleStaticLock => "静态锁",
        Key::RoleCapability => "能力容器",
        Key::RoleUserMemory => "用户存储",
        Key::RoleDynamicLock => "动态锁",
        Key::RoleConfiguration => "配置",
        Key::RolePassword => "密码",
        Key::RolePasswordAck => "密码确认",
        Key::RoleReserved => "保留",
        Key::StateChanged => "已更改",
        Key::StateEqual => "相同",
        Key::StateUnknown => "未知",
        Key::IdentityChanged => "卡片标识已改变",
        Key::IdentityUnchanged => "标识未改变",

        Key::BlocksTitle => "数据块",
        Key::BlocksSubtitle => "MIFARE Classic · 数据块、扇区与尾块",
        Key::CardMemory => "卡片存储",
        Key::CardMemorySub => "厂商块、数据块与扇区尾块",
        Key::Readability => "可读性",
        Key::CardIdentity => "卡片标识",
        Key::KeyStoreTitle => "密钥库",
        Key::LastBackup => "最近备份",
        Key::FilterBlocks => "筛选数据块",
        Key::AllTypes => "所有类型",
        Key::DataType => "数据",
        Key::Trailers => "尾块",
        Key::ManufacturerType => "厂商块",
        Key::ReadLegend => "已读取",
        Key::UnreadableLegend => "不可读",
        Key::UnknownLegend => "未知",
        Key::BlockCount => "数据块",
        Key::SectorCount => "扇区",
        Key::BlockLabel => "块",
        Key::SectorLabel => "扇区",
        Key::TypeLabel => "类型",
        Key::StatusLabel => "状态",
        Key::KnownLabel => "已知",
        Key::DataLabel => "数据",
        Key::SelectBlock => "请选择一个数据块。",
        Key::NoCardTitle => "无卡片镜像",
        Key::NoCardBody => "读取 MIFARE Classic 卡片或加载转储文件。",
        Key::NoDumpTitle => "无卡片镜像",
        Key::NoDumpBody => "读取卡片或加载转储文件。",
        Key::BlockKicker => "块 {} · 扇区 {}",
        Key::InspectorHex => "十六进制",
        Key::InspectorAscii => "ASCII",
        Key::TrailerFields => "尾块字段",
        Key::ValueBlockTitle => "值块",
        Key::ValueLabel => "值",
        Key::AddressLabel => "地址",
        Key::AccessConditions => "访问条件",
        Key::AccessRead => "读取",
        Key::AccessWrite => "写入",
        Key::AccessIncrement => "增值",
        Key::AccessDecrement => "减值",
        Key::BlockZeroReadOnly => "此版本中块 0 为只读，不提供 UID 写入。",
        Key::NewBlockData => "新的块数据",
        Key::WriteBlock => "写入块",
        Key::WriteTrailer => "写入尾块…",
        Key::ReadBlock => "读取块",
        Key::ConfirmWriteTrailerTitle => "写入扇区尾块？",
        Key::ConfirmWriteTrailerBody => {
            "块 {} 是扇区 {} 的尾块。写入可能更改密钥、访问位，或使该扇区无法读取。首次更改前会写入项目备份。"
        }
        Key::ConfirmRestoreTitle => "恢复此转储？",
        Key::ConfirmRestoreBody => "所选卡片将被加载的镜像覆盖。首次更改前会写入项目备份。",
        Key::ConfirmFormatTitle => "格式化 Classic NDEF？",
        Key::ConfirmFormatBody => {
            "所有 NFC 扇区将被重写，密钥和访问位会永久改变。首次更改前会写入项目备份。"
        }
        Key::ConfirmWritePageTitle => "写入第 {} 页？",
        Key::ConfirmWritePageBody => {
            "第 {} 页是{}。写入将通过回读校验，并在首次更改前写入项目备份。"
        }
        Key::ConfirmWriteAdvancedTitle => "写入高级页？",
        Key::ConfirmWriteAdvancedBody => {
            "第 {} 页是{}。这是高级写入。首次更改前会写入项目备份，并通过回读校验。"
        }
        Key::DumpSaved => "转储已保存",
        Key::DumpLoaded => "转储已加载",
        Key::RestoreOptions => "恢复选项",
        Key::ContinueOnError => "块失败后继续",
        Key::DefaultDumpFormat => "默认转储格式",
        Key::UidLabel => "UID",
        Key::AtqaLabel => "ATQA",
        Key::SakLabel => "SAK",
        Key::CardTypeLabel => "卡片类型",
        Key::ReadabilityDetail => "可读",
        Key::NdefBytesLabel => "NDEF 字节",
        Key::BytesUnit => "字节",
        Key::PagesUnit => "页",
        Key::BlocksUnit => "块",
        Key::SectorsUnit => "扇区",
        Key::KeysUnit => "密钥",
        Key::RecordsUnit => "记录",
        Key::KnownBytes => "已知字节",
        Key::BackupPath => "备份",
        Key::NoBackup => "尚无备份",

        Key::ClassicNdefTitle => "Classic NDEF",
        Key::ClassicNdefSubtitle => "MAD 映射、NDEF TLV 与记录写入",
        Key::NdefMessageTitle => "NDEF 消息",
        Key::NdefCapacity => "NDEF 容量",
        Key::NdefUsed => "已用字节",
        Key::NdefEmpty => "空",
        Key::NdefUnformatted => "未格式化",
        Key::NdefInvalid => "无效",
        Key::NdefReadOnly => "只读",
        Key::NdefReadWrite => "可读 / 可写",
        Key::NdefNotWritable => "不可写",
        Key::MappingTitle => "映射",
        Key::MappingVersion => "映射版本",
        Key::NfcSectors => "NFC 扇区",
        Key::DataBlocksLabel => "数据块",
        Key::DataArea => "数据区",
        Key::AccessLabel => "访问权限",
        Key::GpbLabel => "GPB",
        Key::NoMad => "未找到 MAD。格式化卡片可创建 MAD1/MAD2 和 NFC 扇区。",
        Key::FormatCard => "格式化卡片",
        Key::FormatReadOnly => "格式化后只读",
        Key::FormatClassicNote => {
            "格式化会重写 MAD 和所有 NFC 扇区，并更改扇区密钥与访问位。首次更改前会写入备份。"
        }
        Key::WriteNdef => "写入 NDEF",
        Key::WriteTextRecord => "文本",
        Key::WriteUriRecord => "URI",
        Key::WriteRawFile => "原始文件",
        Key::RecordType => "记录类型",
        Key::RecordText => "文本",
        Key::RecordUri => "URI",
        Key::RecordFile => "文件",
        Key::RecordTnf => "TNF",
        Key::RecordTypeLabel => "类型",
        Key::RecordPayload => "负载",
        Key::RecordLanguage => "语言",
        Key::RecordIndex => "记录 {}",
        Key::TlvInfo => "TLV 已解析 · 终止符与填充已保留 · 写入后消息已校验。",
        Key::ClassicNdefWriteNote => "写入 TLV、逐字节校验，并在写入前保留项目备份。",
        Key::ExportMessage => "导出消息",
        Key::MessageBytes => "消息字节",
        Key::CapacityBytes => "容量",
        Key::SelectRecordSource => "输入值或选择文件。",

        Key::DiffTitle => "差异",
        Key::DiffSubtitle => "两个卡片镜像 · 更改的数据块与尾块",
        Key::CompareImages => "比较镜像",
        Key::BeforeImage => "更改前镜像",
        Key::AfterImage => "更改后镜像",
        Key::Replace => "替换",
        Key::ChangedBlocks => "更改的块",
        Key::EqualBlocks => "相同的块",
        Key::UnknownBlocks => "未知的块",
        Key::TrailerChanges => "尾块更改",
        Key::DiffSummary => "差异摘要",
        Key::Before => "更改前",
        Key::After => "更改后",
        Key::LayoutLabel => "布局",
        Key::OnlyDifferences => "仅列出差异",
        Key::NoDiffTitle => "无比较结果",
        Key::NoDiffBody => "读取两次卡片，或加载两个转储文件。",
        Key::LoadImage => "加载镜像",
        Key::DiffDetail => "尾块更改会单独标注，因为它们会改变密钥和访问条件。",

        Key::KeysTitle => "密钥库",
        Key::KeysSubtitle => "扇区候选与来源 · 与 nfctl 共享",
        Key::KeyStoreSub => "扇区绑定与通配候选 · 以 0600 存储",
        Key::SectorColumn => "扇区",
        Key::TypeColumn => "类型",
        Key::KeyColumn => "密钥",
        Key::SourceColumn => "来源",
        Key::RevealKeys => "显示密钥",
        Key::HideKeys => "隐藏密钥",
        Key::ScanKeys => "扫描密钥",
        Key::ScanTypes => "密钥类型",
        Key::KeyTypesAB => "A + B",
        Key::KeyTypesA => "Key A",
        Key::KeyTypesB => "Key B",
        Key::AddKey => "添加密钥",
        Key::AddKeyNote => "只有卡片接受密钥且尾块读取成功后，密钥才会被视为可信。",
        Key::KeyEntryCount => "条目",
        Key::KeyMasked => "•••• •••• ••••",
        Key::ImportKeys => "导入密钥",
        Key::RemoveKey => "移除密钥",
        Key::KeyImportDone => "密钥已导入",
        Key::KeyScanNote => "密钥库由 GUI 和 nfctl 共享，绝不写入转储文件。",

        Key::RecoveryTitle => "恢复",
        Key::RecoverySubtitle => "原生 Crypto1 引擎 · 仅认证 · 不写入卡片",
        Key::RecoveryStrategy => "恢复策略",
        Key::RecoveryStrategySub => "原生引擎 · 仅认证 · 只读",
        Key::StrategyAuto => "自动",
        Key::StrategyAutoDesc => "识别卡片后，依次尝试字典、嵌套、静态嵌套或暗侧攻击。",
        Key::StrategyNested => "嵌套",
        Key::StrategyNestedDesc => "使用已知扇区密钥恢复另一扇区的未知密钥。",
        Key::StrategyStaticNested => "静态嵌套",
        Key::StrategyStaticNestedDesc => "适用于静态加密随机数卡片的嵌套路径。",
        Key::StrategyHardNested => "硬嵌套",
        Key::StrategyHardNestedDesc => "基于 PM3 表的位切片求解器，消耗更多 CPU 和时间。",
        Key::StrategyDarkside => "暗侧",
        Key::StrategyDarksideDesc => "没有已知密钥时使用的纯卡片路径。",
        Key::StrategyFm11rf08s => "FM11RF08S",
        Key::StrategyFm11rf08sDesc => "检测静态加密随机数特征并说明限制。",
        Key::Recommended => "推荐",
        Key::TargetTitle => "目标",
        Key::TargetSub => "扇区密钥由所选数据块推导",
        Key::CandidateKeys => "候选密钥",
        Key::CandidateCount => "已存密钥",
        Key::RunTitle => "运行",
        Key::RunSub => "采集 → 过滤 → 求解 → 校验",
        Key::StageAcquire => "采集加密随机数",
        Key::StageFilter => "过滤状态对",
        Key::StageSolve => "位切片求解",
        Key::StageVerify => "在卡片上校验密钥",
        Key::StartRecovery => "开始恢复",
        Key::CancelRecovery => "取消",
        Key::VerifiedKey => "已验证密钥",
        Key::VerifiedKeyDetail => "恢复后已通过全新认证。",
        Key::RecoveryNoWrites => "不写入",
        Key::RecoveryScopeNote => "恢复只会认证，绝不写入数据块。仅可用于你拥有的卡片。",
        Key::LastRun => "上次运行",
        Key::ResultLabel => "结果",
        Key::RecoveryIdle => "空闲",
        Key::RecoveryFailed => "恢复失败",
        Key::RecoveryCancelled => "恢复已取消",
        Key::RecoveryProgressLabel => "进度",
        Key::RecoveryProfile => "特征",
        Key::RecoveryStrategyLabel => "策略",
        Key::NoCandidates => "没有候选密钥。嵌套和硬嵌套恢复需要已知密钥。",
        Key::RecoveryRunning => "正在恢复",
        Key::RecoveryVerified => "已验证",
        Key::RecoveryAttempts => "尝试次数",
        Key::RecoveryNote => "恢复说明",

        Key::Type2MemoryTitle => "存储页",
        Key::Type2MemorySubtitle => "NTAG · 页、用途与一次性锁定",
        Key::TagLabel => "标签",
        Key::PageReadability => "可读性",
        Key::NdefBytes => "NDEF",
        Key::ProtectionTitle => "保护",
        Key::UnlockedState => "未锁定",
        Key::LockedPagesCount => "已锁定页",
        Key::CfgLockClear => "CFGLCK 未置位",
        Key::TagMemory => "标签存储",
        Key::TagMemorySub => "页用途与一次性锁定",
        Key::FilterPages => "筛选页",
        Key::UserMemoryFilter => "用户存储",
        Key::ConfigurationFilter => "配置",
        Key::LockFilter => "锁定位",
        Key::WritableLegend => "可写",
        Key::LockedLegend => "已锁定",
        Key::PageCount => "页",
        Key::PageLabel => "页",
        Key::RoleLabel => "用途",
        Key::LockLabel => "锁定",
        Key::SelectPage => "请选择一页。",
        Key::PageKicker => "页 {}",
        Key::PageNotReadable => "不可读",
        Key::PwdPackNote => "PWD 和 PACK 无法回读。写入将通过 PWD_AUTH 校验。",
        Key::AdvancedPageNote => "{} 页需要高级确认和备份。",
        Key::LockedPageNote => "此页已锁定，无法写入。",
        Key::NewPageData => "新的页数据",
        Key::WritePage => "写入页",
        Key::WriteAdvancedPage => "写入页…",
        Key::PageWritable => "可写",
        Key::PageAdvanced => "高级",
        Key::SaveDump => "保存转储",
        Key::LoadDump => "加载转储",
        Key::RestoreTag => "恢复…",
        Key::RestoreTitle => "恢复此转储？",
        Key::RestoreBody => "用户存储将被加载的镜像覆盖。首次更改前会写入项目备份。",
        Key::RestoreIncludeOptions => "恢复包含选项",
        Key::IncludeConfiguration => "配置页",
        Key::IncludeCapabilityContainer => "能力容器",
        Key::IncludeLocks => "锁定位",
        Key::Type2NdefTitle => "NDEF",
        Key::Type2NdefSubtitle => "Type 2 标签 · 能力容器可读 / 可写",
        Key::CapacityContainer => "能力容器",
        Key::CcBytes => "CC 字节",
        Key::DataAreaLabel => "数据区",
        Key::FormatTag => "格式化标签",
        Key::FormatTagNote => "重写用户存储中的 NDEF TLV。首次更改前会写入备份。",
        Key::WriteNdefType2Note => "最后写入 NLEN，然后通过回读校验 NLEN 和消息。",
        Key::StaticLock => "静态锁",
        Key::DynamicLock => "动态锁",
        Key::LockStateTitle => "锁定状态",
        Key::LockStateSub => "一次性锁定位 · 第 2 页和第 130 页",
        Key::LockedPagesLabel => "已锁定页",
        Key::WritableUserPages => "可写用户页",
        Key::LockPagesTitle => "锁定页",
        Key::LockPagesSub => "高级 · 需要输入确认",
        Key::ApplyLocks => "应用锁定…",
        Key::LockNote => "锁定位是一次性的。首次锁定前会写入备份，操作无法撤销。",
        Key::ConfigurationTitle => "配置",
        Key::ConfigurationSub => "AUTH0、ACCESS、CFGLCK 与 AUTHLIM",
        Key::CfgLockLabel => "CFGLCK",
        Key::AuthLimLabel => "AUTHLIM",
        Key::PasswordConfigTitle => "密码配置",
        Key::PasswordConfigSub => "高级 · PWD/PACK 无法回读",
        Key::SetPassword => "设置密码…",
        Key::PasswordNote => {
            "写入 AUTH0/ACCESS 可能导致标签在没有密码时无法访问。首次更改前会写入备份。"
        }
        Key::AdvancedActionPlan => "高级操作计划",
        Key::AdvancedActionPlanSub => "在任何写入前显示于确认对话框",
        Key::ExactTarget => "确切目标",
        Key::OldToNew => "旧值 → 新值",
        Key::BackupLabel => "备份",
        Key::VerificationLabel => "校验",
        Key::AdvancedActionNote => {
            "每个高级操作都会在确认前说明确切页、新旧值、备份路径和回读计划。"
        }
        Key::NoLockedPages => "没有已锁定的页",
        Key::ReadabilityPages => "页可读",
        Key::ReadabilityPagesSub => "完整读取",

        Key::DiagnosticsTitle => "诊断",
        Key::DiagnosticsSubtitle => "版本、读卡器、资源与运行时检查",
        Key::ApplicationSection => "应用",
        Key::BuildLabel => "构建",
        Key::PlatformLabel => "平台",
        Key::GuiLabel => "GUI",
        Key::ReadersSection => "读卡器",
        Key::ReaderLease => "读卡器租约",
        Key::PcscdLabel => "pcscd",
        Key::DataSection => "数据",
        Key::HardnestedTables => "硬嵌套表",
        Key::KeyStoreLabel => "密钥库",
        Key::BackupsLabel => "备份",
        Key::BackupMode => "备份权限",
        Key::ApplicationIdentity => "应用标识",
        Key::DesktopEntry => "桌面入口",
        Key::ExecIcon => "Exec / Icon",
        Key::Locales => "语言",
        Key::ChecksSection => "检查",
        Key::CardTransport => "卡片传输",
        Key::NdefCodec => "NDEF 编解码",
        Key::RecoveryEngine => "恢复引擎",
        Key::HardwareTests => "硬件测试",
        Key::CopyReport => "复制报告",
        Key::CopyReportDone => "报告已复制",
        Key::NotAvailable => "不可用",
        Key::ReadyLabel => "就绪",
        Key::ExplicitGateOnly => "仅显式启用",
        Key::ConfigurationSection => "配置",

        Key::ActivityTitle => "活动",
        Key::ActivitySubtitle => "本次会话的操作 · 备份与校验结果",
        Key::SessionActivity => "会话活动",
        Key::SessionActivitySub => "此窗口中的操作 · 最新在前",
        Key::ClearActivity => "清除",
        Key::ActivityEmpty => "本次会话尚无操作。",
        Key::SafetySub => "每次更改都适用的规则",
        Key::SafetyBackup => "每次写入都会创建项目备份、写入、回读并检查卡片标识。",
        Key::SafetyAdvanced => "锁定、配置、密码和格式化操作需要输入确认。",
        Key::SafetyBlockZero => "此版本中块 0 为只读，不提供 UID 和出厂格式化操作。",

        Key::SettingsTitle => "设置",
        Key::SettingsSubtitle => "工作台路径、格式与语言",
        Key::KeyStorePath => "密钥库路径",
        Key::BackupDirectory => "备份目录",
        Key::HardnestedTablesLabel => "硬嵌套表",
        Key::UseCommonKeys => "尝试内置常用密钥",
        Key::LanguageSetting => "语言",
        Key::SaveSettings => "保存设置",
        Key::SettingsNote => "密钥库和备份目录与 nfctl 共享。语言偏好仅影响 GUI。",
        Key::ResetSettings => "恢复默认",

        Key::ConfirmAction => "确认操作",
        Key::ConfirmKicker => "确认",
        Key::TypeToken => "输入 I_OWN_THIS_CARD 以继续",
        Key::DialogCancel => "取消",
        Key::DialogContinue => "继续",
        Key::ActionPlan => "操作计划",
        Key::BackupBeforeChange => "首次更改前会写入项目备份。",
        Key::ReadBackPlan => "每个写入字节都会通过回读和卡片标识检查校验。",
        Key::CardPresent => "卡片在位",
        Key::CardAbsent => "无卡片",
        Key::NoCardDetected => "未检测到卡片",
        Key::PlaceCard => "将卡片放到读卡器上即可开始。",
        Key::NoTagTitle => "无标签镜像",
        Key::NoTagBody => "读取 Type 2 标签或加载转储文件。",
        Key::CardFamilyLabel => "卡片",
        Key::FamilyClassic => "Classic",
        Key::FamilyType2 => "Type 2",
        Key::NoKeysTitle => "尚未保存密钥",
        Key::NoKeysBody => "扫描卡片，或在右侧添加密钥。",
        Key::ReadToLoad => "点击“读取”加载镜像。",
        Key::ReaderMenu => "读卡器",
        Key::AdvancedActions => "高级操作",
        Key::WrittenAndVerified => "已写入并校验",
        Key::UnchangedValue => "内容已相同",
        Key::NoChanges => "无更改",
        Key::ReadOnly => "只读",
        Key::Settings => "设置",
        Key::Compare => "比较",
        Key::FieldBefore => "更改前文件",
        Key::FieldAfter => "更改后文件",
        Key::StatusCompareDone => "比较完成",
        Key::StatusCompareFailed => "比较失败",
        Key::BusyRestoring => "正在恢复",
        Key::BusyWritingBlock => "正在写入块",
        Key::BusyFormatting => "正在格式化",
        Key::BusyWritingNdef => "正在写入 NDEF",
        Key::BusyScanningKeys => "正在扫描密钥",
        Key::BusyRecovering => "正在恢复密钥",
        Key::WatchCard => "监视卡片在位",
        Key::DefaultTables => "已安装默认路径",
        Key::BusyLabel => "忙碌",
        Key::BlockStatusUnread => "未读取",
        Key::BlockStatusRead => "已读取",
        Key::BlockStatusAuthFailed => "认证失败",
        Key::BlockStatusReadFailed => "读取失败",
        Key::BlockStatusSynthetic => "合成",
        Key::BlockStatusVerified => "已验证",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalogs_cover_the_same_keys_without_empty_values() {
        let mut seen = HashSet::new();
        for key in Key::ALL {
            assert!(seen.insert(*key), "duplicate key {key:?}");
            let en = english(*key);
            let zh = chinese(*key);
            assert!(!en.trim().is_empty(), "empty English value for {key:?}");
            assert!(!zh.trim().is_empty(), "empty Chinese value for {key:?}");
        }
    }

    #[test]
    fn english_is_the_base_and_chinese_switches() {
        let en = Translator::new(Language::En);
        let zh = Translator::new(Language::ZhCn);
        assert_eq!(en.text(Key::ScreenBlocks), "Blocks");
        assert_eq!(zh.text(Key::ScreenBlocks), "数据块");
        assert_eq!(zh.text(Key::AppName), "NFC Foundry");
    }

    #[test]
    fn empty_chinese_values_fall_back_to_english() {
        let translated = match "" {
            "" => english(Key::ReadCard),
            text => text,
        };
        assert_eq!(translated, "Read card");
        assert_eq!(
            Translator::new(Language::En).text(Key::ReadCard),
            "Read card"
        );
    }

    #[test]
    fn counts_select_the_singular_and_plural_keys() {
        let translator = Translator::new(Language::En);
        assert_eq!(
            translator.count(1, Key::BytesUnit, Key::BytesUnit),
            "1 bytes"
        );
        assert_eq!(
            translator.count(2, Key::BlockLabel, Key::BlocksUnit),
            "2 blocks"
        );
    }

    #[test]
    fn locale_detection_prefers_saved_then_lc_all_then_lang() {
        let env = |name: &str| match name {
            "LC_ALL" => Some("zh_CN.UTF-8".to_owned()),
            "LANG" => Some("en_US.UTF-8".to_owned()),
            _ => None,
        };
        assert_eq!(detect_language(Some(Language::En), env), Language::En);
        assert_eq!(detect_language(None, env), Language::ZhCn);
        assert_eq!(
            detect_language(None, |name| match name {
                "LANG" => Some("en_GB.UTF-8".to_owned()),
                _ => None,
            }),
            Language::En
        );
        assert_eq!(detect_language(None, |_| None), Language::En);
    }

    #[test]
    fn locale_values_resolve_to_supported_languages() {
        assert_eq!(locale_language("zh_CN.UTF-8"), Some(Language::ZhCn));
        assert_eq!(locale_language("zh-Hans-CN"), Some(Language::ZhCn));
        assert_eq!(locale_language("de_DE.UTF-8"), Some(Language::En));
        assert_eq!(locale_language("C.UTF-8"), Some(Language::En));
        assert_eq!(locale_language(""), None);
    }

    #[test]
    fn preference_codes_round_trip() {
        for language in Language::ALL {
            assert_eq!(Language::from_code(language.code()), Some(language));
        }
        assert_eq!(Language::from_code("fr"), None);
    }
}
