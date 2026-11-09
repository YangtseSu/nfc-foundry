#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateId {
    Text,
    Uri,
    SmartPoster,
    VCard,
    Wifi,
    BluetoothLe,
    Dpp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateField {
    pub id: &'static str,
    pub label: &'static str,
    pub required: bool,
}
