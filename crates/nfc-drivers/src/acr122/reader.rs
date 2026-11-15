//! Reader wrapper over the shared ACR122 protocol.
//!
//! Both transports expose the same card behavior; only the link differs. A
//! concrete wrapper supplies the link factory to [`Acr122Reader::open`].

use std::fmt;
use std::thread;
use std::time::Duration;

use nfc_core::{
    BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, PAGE_QUAD_SIZE, PAGE_SIZE, Result,
    TYPE2_VERSION_SIZE,
};
use nfc_transport::{ApduCapabilities, ApduResponse, DeviceInfo, ParityFrame, ReaderCapabilities};

use super::link::AcrLink;
use super::protocol::{Acr122Config, Acr122Protocol};

pub(crate) use crate::pn53x::FirmwareVersion;

/// Shared ACR122 reader state: device facts, retry policy, and the open
/// protocol.
pub(crate) struct Acr122Reader {
    info: DeviceInfo,
    config: Acr122Config,
    capabilities: ReaderCapabilities,
    apdu: ApduCapabilities,
    protocol: Option<Acr122Protocol>,
}

impl fmt::Debug for Acr122Reader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Acr122Reader")
            .field("info", &self.info)
            .field("open", &self.protocol.is_some())
            .finish()
    }
}

impl Acr122Reader {
    pub(crate) fn new(
        info: DeviceInfo,
        capabilities: ReaderCapabilities,
        apdu: ApduCapabilities,
    ) -> Self {
        Self {
            info,
            config: Acr122Config::default(),
            capabilities,
            apdu,
            protocol: None,
        }
    }

    pub(crate) fn device(&self) -> &DeviceInfo {
        &self.info
    }

    pub(crate) fn firmware(&self) -> Option<FirmwareVersion> {
        self.protocol.as_ref().map(Acr122Protocol::firmware)
    }

    pub(crate) fn open(
        &mut self,
        conn_string: &str,
        connect: impl Fn(&str) -> Result<Box<dyn AcrLink>>,
    ) -> Result<()> {
        if self.protocol.is_some() {
            self.close()?;
        }
        let mut last_error = None;
        for attempt in 0..self.config.attempts.max(1) {
            let link = connect(conn_string)?;
            match Acr122Protocol::open(link, self.config.clone()) {
                Ok(protocol) => {
                    self.protocol = Some(protocol);
                    self.info.conn_string = conn_string.to_owned();
                    return Ok(());
                }
                Err(error @ (NfcError::Timeout(_) | NfcError::Io(_)))
                    if attempt + 1 < self.config.attempts.max(1) =>
                {
                    last_error = Some(error);
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or(NfcError::Unsupported(
            "the ACR122U did not respond as a PN532",
        )))
    }

    pub(crate) fn close(&mut self) -> Result<()> {
        if let Some(mut protocol) = self.protocol.take() {
            protocol.shutdown();
        }
        Ok(())
    }

    pub(crate) fn release_card(&mut self) -> Result<()> {
        self.protocol()?.release_card()
    }

    pub(crate) fn cycle_rf_field(&mut self) -> Result<()> {
        self.protocol()?.cycle_rf_field()
    }

    fn protocol(&mut self) -> Result<&mut Acr122Protocol> {
        self.protocol
            .as_mut()
            .ok_or_else(|| NfcError::Io("the ACR122U reader is not open".to_owned()))
    }

    pub(crate) fn firmware_label(&self) -> Option<String> {
        self.protocol
            .as_ref()
            .map(|protocol| protocol.firmware().label())
    }

    pub(crate) fn capabilities(&self) -> ReaderCapabilities {
        self.capabilities
    }

    pub(crate) fn card_info(&mut self) -> Result<CardInfo> {
        self.protocol()?.select_card()
    }

    pub(crate) fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()> {
        self.protocol()?.authenticate(block, key_type, key)
    }

    pub(crate) fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        self.protocol()?.read_block(block)
    }

    pub(crate) fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.write_block(block, data)
    }

    pub(crate) fn read_page_quad(&mut self, start_page: u8) -> Result<[u8; PAGE_QUAD_SIZE]> {
        self.protocol()?.read_page_quad(start_page)
    }

    pub(crate) fn write_page(&mut self, page: u8, data: [u8; PAGE_SIZE]) -> Result<()> {
        self.protocol()?.write_page(page, data)
    }

    pub(crate) fn password_auth(&mut self, password: [u8; PAGE_SIZE]) -> Result<[u8; 2]> {
        self.protocol()?.password_auth(password)
    }

    pub(crate) fn type2_version(&mut self) -> Result<[u8; TYPE2_VERSION_SIZE]> {
        self.protocol()?.type2_version()
    }

    pub(crate) fn apdu_capabilities(&self) -> ApduCapabilities {
        self.apdu
    }

    pub(crate) fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse> {
        self.protocol()?.transceive_apdu(apdu)
    }

    pub(crate) fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>> {
        self.protocol()?.transceive_bytes(tx)
    }

    pub(crate) fn transceive_bits(
        &mut self,
        tx: &[u8],
        tx_bits: usize,
    ) -> Result<(Vec<u8>, usize)> {
        self.protocol()?.transceive_bits(tx, tx_bits)
    }

    pub(crate) fn set_raw_mode(&mut self, enabled: bool) -> Result<()> {
        self.protocol()?.set_raw_mode(enabled)
    }

    pub(crate) fn set_recovery_mode(&mut self, enabled: bool) -> Result<()> {
        self.protocol()?.set_recovery_mode(enabled)
    }

    pub(crate) fn transceive_parity(&mut self, data: &[u8], parity: &[u8]) -> Result<ParityFrame> {
        self.protocol()?.transceive_parity(data, parity)
    }

    pub(crate) fn transceive_parity_bits(
        &mut self,
        data: &[u8],
        data_bits: usize,
        parity: &[u8],
    ) -> Result<ParityFrame> {
        self.protocol()?
            .transceive_parity_bits(data, data_bits, parity)
    }

    pub(crate) fn transceive_parity_expect(
        &mut self,
        data: &[u8],
        parity: &[u8],
        expected_bits: usize,
    ) -> Result<ParityFrame> {
        self.protocol()?
            .transceive_parity_expect(data, parity, expected_bits)
    }

    pub(crate) fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.write_manufacturer_block(data)
    }

    pub(crate) fn probe_gen1a(&mut self) -> Result<bool> {
        self.protocol()?.probe_gen1a()
    }

    pub(crate) fn seal_manufacturer_card(&mut self, config: [u8; BLOCK_SIZE]) -> Result<()> {
        self.protocol()?.seal_manufacturer_card(config)
    }
}

/// Forward `Reader` and `ManufacturerWriter` to the `core` field of a concrete
/// ACR122 reader. Each wrapper supplies its own `open` and emergency methods.
macro_rules! forward_acr122_reader {
    ($reader:ty, $connect:expr) => {
        impl nfc_transport::Reader for $reader {
            fn device(&self) -> &nfc_transport::DeviceInfo {
                self.core.device()
            }

            fn open(&mut self, conn_string: &str) -> nfc_core::Result<()> {
                self.core.open(conn_string, $connect)
            }

            fn firmware(&self) -> Option<String> {
                self.core.firmware_label()
            }

            fn capabilities(&self) -> nfc_transport::ReaderCapabilities {
                self.core.capabilities()
            }

            fn close(&mut self) -> nfc_core::Result<()> {
                self.core.close()
            }

            fn card_info(&mut self) -> nfc_core::Result<nfc_core::CardInfo> {
                self.core.card_info()
            }

            fn authenticate(
                &mut self,
                block: u8,
                key_type: nfc_core::KeyType,
                key: nfc_core::Key,
            ) -> nfc_core::Result<()> {
                self.core.authenticate(block, key_type, key)
            }

            fn read_block(&mut self, block: u8) -> nfc_core::Result<[u8; nfc_core::BLOCK_SIZE]> {
                self.core.read_block(block)
            }

            fn write_block(
                &mut self,
                block: u8,
                data: [u8; nfc_core::BLOCK_SIZE],
            ) -> nfc_core::Result<()> {
                self.core.write_block(block, data)
            }

            fn read_page_quad(
                &mut self,
                start_page: u8,
            ) -> nfc_core::Result<[u8; nfc_core::PAGE_QUAD_SIZE]> {
                self.core.read_page_quad(start_page)
            }

            fn write_page(
                &mut self,
                page: u8,
                data: [u8; nfc_core::PAGE_SIZE],
            ) -> nfc_core::Result<()> {
                self.core.write_page(page, data)
            }

            fn password_auth(
                &mut self,
                password: [u8; nfc_core::PAGE_SIZE],
            ) -> nfc_core::Result<[u8; 2]> {
                self.core.password_auth(password)
            }

            fn type2_version(&mut self) -> nfc_core::Result<[u8; nfc_core::TYPE2_VERSION_SIZE]> {
                self.core.type2_version()
            }

            fn apdu_capabilities(&self) -> nfc_transport::ApduCapabilities {
                self.core.apdu_capabilities()
            }

            fn transceive_apdu(
                &mut self,
                apdu: &[u8],
            ) -> nfc_core::Result<nfc_transport::ApduResponse> {
                self.core.transceive_apdu(apdu)
            }

            fn transceive_bytes(&mut self, tx: &[u8]) -> nfc_core::Result<Vec<u8>> {
                self.core.transceive_bytes(tx)
            }

            fn transceive_bits(
                &mut self,
                tx: &[u8],
                tx_bits: usize,
            ) -> nfc_core::Result<(Vec<u8>, usize)> {
                self.core.transceive_bits(tx, tx_bits)
            }

            fn set_raw_mode(&mut self, enabled: bool) -> nfc_core::Result<()> {
                self.core.set_raw_mode(enabled)
            }

            fn manufacturer_writer(
                &mut self,
            ) -> Option<&mut dyn nfc_transport::ManufacturerWriter> {
                Some(self)
            }

            fn manufacturer_sealer(
                &mut self,
            ) -> Option<&mut dyn nfc_transport::ManufacturerSealer> {
                Some(self)
            }

            fn set_recovery_mode(&mut self, enabled: bool) -> nfc_core::Result<()> {
                self.core.set_recovery_mode(enabled)
            }

            fn transceive_parity(
                &mut self,
                data: &[u8],
                parity: &[u8],
            ) -> nfc_core::Result<nfc_transport::ParityFrame> {
                self.core.transceive_parity(data, parity)
            }

            fn transceive_parity_bits(
                &mut self,
                data: &[u8],
                data_bits: usize,
                parity: &[u8],
            ) -> nfc_core::Result<nfc_transport::ParityFrame> {
                self.core.transceive_parity_bits(data, data_bits, parity)
            }

            fn transceive_parity_expect(
                &mut self,
                data: &[u8],
                parity: &[u8],
                expected_bits: usize,
            ) -> nfc_core::Result<nfc_transport::ParityFrame> {
                self.core
                    .transceive_parity_expect(data, parity, expected_bits)
            }
        }

        impl nfc_transport::ManufacturerWriter for $reader {
            fn write_manufacturer_block(
                &mut self,
                data: [u8; nfc_core::BLOCK_SIZE],
            ) -> nfc_core::Result<()> {
                self.core.write_manufacturer_block(data)
            }
        }

        impl nfc_transport::ManufacturerSealer for $reader {
            fn probe_gen1a(&mut self) -> nfc_core::Result<bool> {
                self.core.probe_gen1a()
            }

            fn seal_manufacturer_card(
                &mut self,
                config: [u8; nfc_core::BLOCK_SIZE],
            ) -> nfc_core::Result<()> {
                self.core.seal_manufacturer_card(config)
            }
        }
    };
}

pub(crate) use forward_acr122_reader;
