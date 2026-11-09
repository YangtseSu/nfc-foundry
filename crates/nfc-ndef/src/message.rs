use crate::NdefRecord;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NdefMessage {
    pub records: Vec<NdefRecord>,
}
