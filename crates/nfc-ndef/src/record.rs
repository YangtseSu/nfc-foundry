#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tnf {
    #[default]
    Empty,
    WellKnown,
    Media,
    AbsoluteUri,
    External,
    Unknown,
    Unchanged,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NdefRecord {
    pub tnf: Tnf,
    pub record_type: Vec<u8>,
    pub id: Vec<u8>,
    pub payload: Vec<u8>,
}
