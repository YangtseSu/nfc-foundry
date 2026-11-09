/// Crypto1/LFSR state. The implementation is ported in phase 06.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Crypto1State {
    pub odd: u32,
    pub even: u32,
}
