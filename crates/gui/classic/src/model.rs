use gpui_kit::SharedString;

/// Tabs in the Classic workspace.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceTab {
    #[default]
    Blocks,
    Diff,
    Keys,
}

impl WorkspaceTab {
    pub const ALL: [Self; 3] = [Self::Blocks, Self::Diff, Self::Keys];

    pub const fn index(self) -> usize {
        match self {
            Self::Blocks => 0,
            Self::Diff => 1,
            Self::Keys => 2,
        }
    }

    pub const fn from_index(index: usize) -> Self {
        match index {
            1 => Self::Diff,
            2 => Self::Keys,
            _ => Self::Blocks,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Blocks => "Blocks",
            Self::Diff => "Diff",
            Self::Keys => "Keys",
        }
    }
}

/// Reader summary shown in the workspace; driver types stay below `nfc-app`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceSummary {
    pub name: String,
    pub conn_string: String,
}

/// Severity of the workspace status line.
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
