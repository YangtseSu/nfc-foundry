#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CapabilityContainer {
    pub major_version: u8,
    pub minor_version: u8,
    pub size: u16,
    pub access: u8,
}
