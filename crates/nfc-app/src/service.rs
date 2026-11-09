use nfc_transport::DeviceInfo;

use crate::DeviceManager;

/// Application facade. The first phase exposes device state; reader and card
/// workflows are added with their domain crates.
#[derive(Clone, Debug, Default)]
pub struct Service {
    devices: DeviceManager,
}

impl Service {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn devices(&self) -> &[DeviceInfo] {
        self.devices.devices()
    }

    pub fn selected_device(&self) -> Option<&str> {
        self.devices.selected()
    }

    pub fn select_device(&mut self, conn_string: &str) -> nfc_core::Result<()> {
        self.devices.select(conn_string)
    }

    pub fn replace_devices(&mut self, devices: Vec<DeviceInfo>) {
        self.devices.replace_devices(devices);
    }
}
