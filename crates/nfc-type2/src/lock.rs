#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LockBits {
    pub static_lock: [u8; 2],
    pub dynamic_lock: [u8; 3],
}
