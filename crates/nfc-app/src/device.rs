use nfc_core::{NfcError, Result};
use nfc_transport::DeviceInfo;

/// In-memory device selection state. Driver enumeration is wired in phase 01.
#[derive(Clone, Debug, Default)]
pub struct DeviceManager {
    devices: Vec<DeviceInfo>,
    selected: Option<String>,
}

impl DeviceManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn devices(&self) -> &[DeviceInfo] {
        &self.devices
    }

    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn select(&mut self, conn_string: &str) -> Result<()> {
        if !self
            .devices
            .iter()
            .any(|device| device.conn_string == conn_string)
        {
            return Err(NfcError::InvalidArgument(format!(
                "reader {conn_string} is not in the device list"
            )));
        }
        self.selected = Some(conn_string.to_owned());
        Ok(())
    }

    pub fn replace_devices(&mut self, devices: Vec<DeviceInfo>) {
        self.devices = devices;
        if let Some(selected) = &self.selected
            && !self
                .devices
                .iter()
                .any(|device| device.conn_string == *selected)
        {
            self.selected = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecting_requires_a_known_reader() {
        let mut manager = DeviceManager::new();
        manager.replace_devices(vec![DeviceInfo::new("PN532", "pn532_uart:/dev/ttyUSB0")]);
        assert!(manager.select("pn532_uart:/dev/ttyUSB0").is_ok());
        assert_eq!(manager.selected(), Some("pn532_uart:/dev/ttyUSB0"));

        manager.replace_devices(Vec::new());
        assert_eq!(manager.selected(), None);
    }
}
