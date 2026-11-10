//! Retained Classic screen state and small presentation enums.

use std::path::PathBuf;

use nfc_app::classic::{BackupStore, KeyTypes};
use nfc_classic::DumpFormat;
use nfc_recover::auto::RecoveryStrategy;
use nfc_ui::Key;

/// Settings the shell shares with the Classic screens.
#[derive(Clone, Debug)]
pub struct ClassicSettings {
    pub key_store_path: Option<PathBuf>,
    pub backup: BackupStore,
    pub tables_dir: Option<PathBuf>,
    pub default_dump_format: DumpFormat,
    pub use_common_keys: bool,
}

impl Default for ClassicSettings {
    fn default() -> Self {
        Self {
            key_store_path: None,
            backup: BackupStore::default(),
            tables_dir: None,
            default_dump_format: DumpFormat::Project,
            use_common_keys: true,
        }
    }
}

/// Dump formats in presentation order.
pub const DUMP_FORMATS: [DumpFormat; 4] = [
    DumpFormat::Project,
    DumpFormat::Raw,
    DumpFormat::Mct,
    DumpFormat::Proxmark3,
];

pub const fn dump_format_key(format: DumpFormat) -> Key {
    match format {
        DumpFormat::Project => Key::FormatProject,
        DumpFormat::Raw => Key::FormatRaw,
        DumpFormat::Mct => Key::FormatMct,
        DumpFormat::Proxmark3 => Key::FormatProxmark,
    }
}

pub fn dump_format_index(format: DumpFormat) -> usize {
    DUMP_FORMATS
        .iter()
        .position(|candidate| *candidate == format)
        .unwrap_or(0)
}

/// NDEF message source written by the NDEF screens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NdefSource {
    #[default]
    Text,
    Uri,
    File,
}

impl NdefSource {
    pub const ALL: [Self; 3] = [Self::Text, Self::Uri, Self::File];

    pub const fn index(self) -> usize {
        match self {
            Self::Text => 0,
            Self::Uri => 1,
            Self::File => 2,
        }
    }

    pub const fn key(self) -> Key {
        match self {
            Self::Text => Key::WriteTextRecord,
            Self::Uri => Key::WriteUriRecord,
            Self::File => Key::WriteRawFile,
        }
    }
}

/// Key-type scan choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanTypes {
    #[default]
    Ab,
    A,
    B,
}

impl ScanTypes {
    pub const ALL: [Self; 3] = [Self::Ab, Self::A, Self::B];

    pub const fn key(self) -> Key {
        match self {
            Self::Ab => Key::KeyTypesAB,
            Self::A => Key::KeyTypesA,
            Self::B => Key::KeyTypesB,
        }
    }

    pub const fn key_types(self) -> KeyTypes {
        match self {
            Self::Ab => KeyTypes::ALL,
            Self::A => KeyTypes::a_only(),
            Self::B => KeyTypes::b_only(),
        }
    }
}

/// Recovery strategies in presentation order.
pub const STRATEGIES: [RecoveryStrategy; 6] = [
    RecoveryStrategy::Auto,
    RecoveryStrategy::Nested,
    RecoveryStrategy::StaticNested,
    RecoveryStrategy::HardNested,
    RecoveryStrategy::DarkSide,
    RecoveryStrategy::Fm11rf08s,
];

pub const fn strategy_key(strategy: RecoveryStrategy) -> Key {
    match strategy {
        RecoveryStrategy::Auto => Key::StrategyAuto,
        RecoveryStrategy::Dictionary => Key::StrategyAuto,
        RecoveryStrategy::Nested => Key::StrategyNested,
        RecoveryStrategy::StaticNested => Key::StrategyStaticNested,
        RecoveryStrategy::HardNested => Key::StrategyHardNested,
        RecoveryStrategy::DarkSide => Key::StrategyDarkside,
        RecoveryStrategy::Fm11rf08s => Key::StrategyFm11rf08s,
    }
}

pub const fn strategy_description_key(strategy: RecoveryStrategy) -> Key {
    match strategy {
        RecoveryStrategy::Auto => Key::StrategyAutoDesc,
        RecoveryStrategy::Dictionary => Key::StrategyAutoDesc,
        RecoveryStrategy::Nested => Key::StrategyNestedDesc,
        RecoveryStrategy::StaticNested => Key::StrategyStaticNestedDesc,
        RecoveryStrategy::HardNested => Key::StrategyHardNestedDesc,
        RecoveryStrategy::DarkSide => Key::StrategyDarksideDesc,
        RecoveryStrategy::Fm11rf08s => Key::StrategyFm11rf08sDesc,
    }
}
