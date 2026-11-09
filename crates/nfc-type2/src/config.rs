#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Configuration {
    pub auth0: u8,
    pub access: u8,
    pub password: [u8; 4],
    pub pack: [u8; 2],
}
