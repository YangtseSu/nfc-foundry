/// User-visible reader facts. The connstring is the exact selector passed to
/// the driver; no autodetection policy lives in this type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub name: String,
    pub conn_string: String,
}

impl DeviceInfo {
    pub fn new(name: impl Into<String>, conn_string: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            conn_string: conn_string.into(),
        }
    }
}

/// Explicit capability profile. Claims are only made after hardware validation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReaderCapabilities {
    pub classic_read: bool,
    pub classic_write: bool,
    pub raw_bytes: bool,
    pub raw_bits: bool,
    pub manufacturer_write: bool,
    pub type2_read: bool,
    pub type2_write: bool,
}
