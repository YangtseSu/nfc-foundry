#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub number: u8,
    pub data: [u8; 4],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Type2Info {
    pub product: String,
    pub user_pages: u16,
    pub user_bytes: u16,
}
