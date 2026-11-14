//! Transport boundary shared by the ACR122U USB and PC/SC links.

use std::time::Duration;

use nfc_core::Result;

/// One complete pseudo-APDU response before status-word handling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApduResponse {
    pub data: Vec<u8>,
    pub sw: [u8; 2],
}

/// Operations the ACR122 protocol layer needs from a link.
pub(crate) trait AcrLink: Send {
    fn power_on(&mut self) -> Result<()>;
    fn xfr_block(&mut self, apdu: &[u8], timeout: Duration) -> Result<ApduResponse>;
    fn shutdown(&mut self);
}
