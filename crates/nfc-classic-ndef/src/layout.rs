#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClassicNdefLayout {
    pub sectors: Vec<u8>,
    pub capacity: usize,
    pub read_only: bool,
}
