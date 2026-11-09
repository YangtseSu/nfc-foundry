#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MadVersion {
    Mad1,
    Mad2,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mad {
    pub version: MadVersion,
    pub ndef_sectors: Vec<u8>,
    pub free_sectors: Vec<u8>,
}
