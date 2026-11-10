//! Retained Type 2 screen state and presentation enums.

use nfc_app::classic::BackupStore;
use nfc_type2::PageRole;
use nfc_ui::Key;

/// Settings the shell shares with the Type 2 screens.
#[derive(Clone, Debug, Default)]
pub struct Type2Settings {
    pub backup: BackupStore,
}

/// NDEF source written by the Type 2 NDEF screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NdefSource {
    #[default]
    Text,
    Uri,
    File,
}

impl NdefSource {
    pub const ALL: [Self; 3] = [Self::Text, Self::Uri, Self::File];

    pub const fn key(self) -> Key {
        match self {
            Self::Text => Key::WriteTextRecord,
            Self::Uri => Key::WriteUriRecord,
            Self::File => Key::WriteRawFile,
        }
    }
}

/// Page table filter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageFilter {
    #[default]
    All,
    User,
    Configuration,
    Lock,
}

impl PageFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::User, Self::Configuration, Self::Lock];

    pub const fn key(self) -> Key {
        match self {
            Self::All => Key::AllRoles,
            Self::User => Key::UserMemoryFilter,
            Self::Configuration => Key::ConfigurationFilter,
            Self::Lock => Key::LockFilter,
        }
    }

    pub const fn matches(self, role: PageRole) -> bool {
        match self {
            Self::All => true,
            Self::User => matches!(role, PageRole::UserMemory),
            Self::Configuration => matches!(
                role,
                PageRole::Configuration | PageRole::Password | PageRole::PasswordAck
            ),
            Self::Lock => matches!(role, PageRole::StaticLock | PageRole::DynamicLock),
        }
    }
}

/// Localized role label.
pub const fn role_key(role: PageRole) -> Key {
    match role {
        PageRole::Manufacturer => Key::RoleManufacturer,
        PageRole::StaticLock => Key::RoleStaticLock,
        PageRole::CapabilityContainer => Key::RoleCapability,
        PageRole::UserMemory => Key::RoleUserMemory,
        PageRole::DynamicLock => Key::RoleDynamicLock,
        PageRole::Configuration => Key::RoleConfiguration,
        PageRole::Password => Key::RolePassword,
        PageRole::PasswordAck => Key::RolePasswordAck,
        PageRole::Reserved => Key::RoleReserved,
    }
}
