use nfc_core::BLOCK_SIZE;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlockStatus {
    #[default]
    Unread,
    Read,
    AuthFailed,
    ReadFailed,
    Synthetic,
    Verified,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub number: usize,
    pub sector: usize,
    pub data: [u8; BLOCK_SIZE],
    pub known_mask: u16,
    pub status: BlockStatus,
    pub error: Option<String>,
}

impl Block {
    pub fn complete(&self) -> bool {
        self.known_mask == u16::MAX
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dump {
    pub layout: super::Layout,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Readability {
    pub readable_blocks: usize,
    pub total_blocks: usize,
    pub readable_sectors: usize,
    pub total_sectors: usize,
    pub skipped_sectors: Vec<usize>,
}
