//! Scripted reader used by application and workflow tests.

use std::collections::HashMap;

use nfc_core::{BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, Result};

use crate::{DeviceInfo, Reader, TrailerWriter};

#[derive(Debug)]
pub struct MockReader {
    info: DeviceInfo,
    card: Option<CardInfo>,
    blocks: HashMap<u8, [u8; BLOCK_SIZE]>,
    raw_mode: bool,
}

impl MockReader {
    pub fn new(info: DeviceInfo, card: Option<CardInfo>) -> Self {
        Self {
            info,
            card,
            blocks: HashMap::new(),
            raw_mode: false,
        }
    }

    pub fn set_card(&mut self, card: Option<CardInfo>) {
        self.card = card;
    }

    pub fn set_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) {
        self.blocks.insert(block, data);
    }

    pub fn is_raw_mode(&self) -> bool {
        self.raw_mode
    }
}

impl Reader for MockReader {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn open(&mut self, conn_string: &str) -> Result<()> {
        self.info.conn_string = conn_string.to_owned();
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        self.card.clone().ok_or(NfcError::NoCard)
    }

    fn authenticate(&mut self, _block: u8, _key_type: KeyType, _key: Key) -> Result<()> {
        Ok(())
    }

    fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        self.blocks
            .get(&block)
            .copied()
            .ok_or(NfcError::InvalidArgument(format!(
                "mock block {block} is not loaded"
            )))
    }

    fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.blocks.insert(block, data);
        Ok(())
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        Ok(Vec::new())
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        Ok((Vec::new(), 0))
    }

    fn set_raw_mode(&mut self, enabled: bool) -> Result<()> {
        self.raw_mode = enabled;
        Ok(())
    }
}

impl TrailerWriter for MockReader {
    fn write_sector_trailer(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.blocks.insert(block, data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_reader_round_trips_blocks_and_raw_mode() {
        let mut reader = MockReader::new(
            DeviceInfo::new("mock", "mock:test"),
            Some(nfc_core::CardInfo::new(
                vec![1, 2, 3, 4],
                [0, 4],
                8,
                nfc_core::CardType::MifareClassic1K,
            )),
        );
        let data = [0xAA; BLOCK_SIZE];
        reader.set_block(1, data);
        assert_eq!(reader.read_block(1).expect("block"), data);
        assert!(!reader.is_raw_mode());
        reader.set_raw_mode(true).expect("raw mode");
        assert!(reader.is_raw_mode());
    }
}
