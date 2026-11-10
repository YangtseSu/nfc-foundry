use gpui_kit::SharedString;

/// Tabs in the Type 2 workspace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceTab {
    #[default]
    Memory,
    Ndef,
    Protection,
}

impl WorkspaceTab {
    pub const fn index(self) -> usize {
        match self {
            Self::Memory => 0,
            Self::Ndef => 1,
            Self::Protection => 2,
        }
    }

    pub const fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Ndef,
            2 => Self::Protection,
            _ => Self::Memory,
        }
    }
}

/// Reader summary shown in the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    pub name: String,
    pub conn_string: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusKind {
    #[default]
    Info,
    Success,
    Warning,
    Error,
}

/// One status message in the workspace footer.
#[derive(Clone, Debug)]
pub struct StatusMessage {
    pub text: SharedString,
    pub kind: StatusKind,
}

impl StatusMessage {
    pub fn info(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Info,
        }
    }

    pub fn success(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Success,
        }
    }

    pub fn warning(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Warning,
        }
    }

    pub fn error(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            kind: StatusKind::Error,
        }
    }
}
