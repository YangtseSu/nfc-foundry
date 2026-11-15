//! Scripted T-Union card for hardware-free tests and `nfctl --mock`.
//!
//! The fixture uses synthetic card numbers, terminals, and balances. It copies
//! the file layout and command sequence of the reference reader, not any live
//! card data.

use nfc_core::{BLOCK_SIZE, CardInfo, CardType, NfcError, Result};
use nfc_transport::{ApduCapabilities, ApduResponse, DeviceInfo, Reader, Transport};

use crate::apdu;
use crate::tunion::{TUNION_AID, TUNION_CASH_AID};

/// Synthetic mock card state.
#[derive(Clone, Debug)]
pub struct MockTUnionCard {
    pub tunion_present: bool,
    pub meta: Vec<u8>,
    pub balance: [u8; 4],
    pub transactions: Vec<Vec<u8>>,
    pub travels: Vec<Vec<u8>>,
    /// Return `6D00` for this transaction record instead of the stored data.
    pub fail_transaction: Option<u8>,
    /// Return `6D00` for this travel record instead of the stored data.
    pub fail_travel: Option<u8>,
}

impl Default for MockTUnionCard {
    fn default() -> Self {
        Self {
            tunion_present: true,
            meta: synthetic_meta(),
            balance: 5000u32.to_be_bytes(),
            transactions: vec![
                transaction(
                    1,
                    500,
                    9,
                    *b"A1\0\0\0\x01",
                    [0x20, 0x26, 0x01, 0x02, 0x03, 0x04, 0x05],
                ),
                transaction(
                    2,
                    200,
                    2,
                    *b"A1\0\0\0\x02",
                    [0x20, 0x26, 0x01, 0x03, 0x04, 0x05, 0x06],
                ),
            ],
            travels: vec![
                travel(
                    4,
                    *b"A1\0\0\0\0\0\x01",
                    1,
                    *b"\0\x01\0\x0C\0\0\0",
                    200,
                    5000,
                    [0x20, 0x26, 0x01, 0x02, 0x03, 0x04, 0x05],
                    *b"\0\x10",
                    *b"\x10\x08\x30\x10\xFF\xFF\xFF\xFF",
                ),
                travel(
                    6,
                    *b"A1\0\0\0\0\0\x02",
                    2,
                    *b"\0\x02\0\x27\0\0\0",
                    200,
                    4800,
                    [0x20, 0x26, 0x01, 0x03, 0x04, 0x05, 0x06],
                    *b"\0\x10",
                    *b"\x01\x66\x33\x10\xFF\xFF\xFF\xFF",
                ),
            ],
            fail_transaction: None,
            fail_travel: None,
        }
    }
}

impl MockTUnionCard {
    /// PPSE directory as returned by SELECT `2PAY.SYS.DDF01`.
    pub fn ppse(&self) -> Vec<u8> {
        let cash = wrap(
            &[0x61],
            &[
                wrap(&[0x4F], &TUNION_CASH_AID),
                wrap(&[0x50], b"MOT_T_CASH"),
                wrap(&[0x87], &[0x01]),
            ]
            .concat(),
        );
        let transit = wrap(
            &[0x61],
            &[
                wrap(&[0x4F], &TUNION_AID),
                wrap(&[0x50], b"MOT_T_EP"),
                wrap(&[0x87], &[0x02]),
            ]
            .concat(),
        );
        let directory = wrap(&[0xBF, 0x0C], &[cash, transit].concat());
        wrap(
            &[0x6F],
            &[wrap(&[0x84], b"2PAY.SYS.DDF01"), wrap(&[0xA5], &directory)].concat(),
        )
    }

    /// Application FCI built from the current metadata fixture.
    pub fn fci(&self) -> Vec<u8> {
        let proprietary = wrap(
            &[0xA5],
            &[
                wrap(&[0x9F, 0x08], &[0x02]),
                wrap(&[0x9F, 0x0C], &self.meta),
            ]
            .concat(),
        );
        wrap(&[0x6F], &[wrap(&[0x84], &TUNION_AID), proprietary].concat())
    }
}

/// One scripted reader. Each open gets an independent copy of the fixture.
pub struct MockTUnionReader {
    card: MockTUnionCard,
    device: DeviceInfo,
}

impl MockTUnionReader {
    pub fn new(card: MockTUnionCard) -> Self {
        Self {
            card,
            device: DeviceInfo::new("Mock T-Union", "mock:tunion"),
        }
    }
}

impl Reader for MockTUnionReader {
    fn device(&self) -> &DeviceInfo {
        &self.device
    }

    fn open(&mut self, _conn_string: &str) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        Ok(CardInfo::new(
            vec![0x04, 0xA1, 0xB2, 0xC3],
            [0x00, 0x08],
            0x28,
            CardType::Iso14443,
        ))
    }

    fn authenticate(
        &mut self,
        _block: u8,
        _key_type: nfc_core::KeyType,
        _key: nfc_core::Key,
    ) -> Result<()> {
        Err(NfcError::Unsupported(
            "the mock T-Union card has no MIFARE authentication",
        ))
    }

    fn read_block(&mut self, _block: u8) -> Result<[u8; BLOCK_SIZE]> {
        Err(NfcError::Unsupported(
            "the mock T-Union card has no MIFARE blocks",
        ))
    }

    fn write_block(&mut self, _block: u8, _data: [u8; BLOCK_SIZE]) -> Result<()> {
        Err(NfcError::Unsupported(
            "the mock T-Union card has no MIFARE blocks",
        ))
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        Err(NfcError::Unsupported(
            "the mock T-Union card only accepts ISO 7816 APDUs",
        ))
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        Err(NfcError::Unsupported(
            "the mock T-Union card only accepts ISO 7816 APDUs",
        ))
    }

    fn set_raw_mode(&mut self, _enabled: bool) -> Result<()> {
        Ok(())
    }

    fn apdu_capabilities(&self) -> ApduCapabilities {
        ApduCapabilities {
            supported: true,
            max_command: 247,
        }
    }

    fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse> {
        if apdu == crate::apdu::select_apdu(b"2PAY.SYS.DDF01")? {
            return Ok(ApduResponse::new(self.card.ppse(), [0x90, 0x00]));
        }
        if apdu == crate::apdu::select_apdu(&TUNION_AID)? {
            if !self.card.tunion_present {
                return Ok(ApduResponse::new(Vec::new(), [0x6A, 0x82]));
            }
            return Ok(ApduResponse::new(self.card.fci(), [0x90, 0x00]));
        }
        if apdu == apdu::balance_apdu() {
            return Ok(ApduResponse::new(self.card.balance.to_vec(), [0x90, 0x00]));
        }
        for (record, data) in self.card.transactions.iter().enumerate() {
            let number = (record + 1) as u8;
            if apdu == apdu::read_record_apdu(number, crate::tunion::TRANSACTION_FILE) {
                if self.card.fail_transaction == Some(number) {
                    return Ok(ApduResponse::new(Vec::new(), [0x6D, 0x00]));
                }
                return Ok(ApduResponse::new(data.clone(), [0x90, 0x00]));
            }
        }
        for (record, data) in self.card.travels.iter().enumerate() {
            let number = (record + 1) as u8;
            if apdu == apdu::read_record_apdu(number, crate::tunion::TRAVEL_FILE) {
                if self.card.fail_travel == Some(number) {
                    return Ok(ApduResponse::new(Vec::new(), [0x6D, 0x00]));
                }
                return Ok(ApduResponse::new(data.clone(), [0x90, 0x00]));
            }
        }
        if apdu.len() == 5
            && apdu[0] == 0x00
            && apdu[1] == 0xB2
            && apdu[3] == 0xC4
            && apdu[2] > self.card.transactions.len() as u8
        {
            return Ok(ApduResponse::new(Vec::new(), [0x6A, 0x83]));
        }
        if apdu.len() == 5
            && apdu[0] == 0x00
            && apdu[1] == 0xB2
            && apdu[3] == 0xF4
            && apdu[2] > self.card.travels.len() as u8
        {
            return Ok(ApduResponse::new(Vec::new(), [0x6A, 0x83]));
        }
        Ok(ApduResponse::new(Vec::new(), [0x6D, 0x00]))
    }
}

/// Transport that opens one scripted T-Union reader.
#[derive(Clone, Debug)]
pub struct MockTUnionTransport {
    card: MockTUnionCard,
}

impl MockTUnionTransport {
    pub fn new() -> Self {
        Self {
            card: MockTUnionCard::default(),
        }
    }

    pub fn with_card(card: MockTUnionCard) -> Self {
        Self { card }
    }
}

impl Default for MockTUnionTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for MockTUnionTransport {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        Ok(vec![DeviceInfo::new("Mock T-Union", "mock:tunion")])
    }

    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>> {
        if conn_string != "mock:tunion" {
            return Err(NfcError::Unsupported(
                "the mock T-Union transport only opens mock:tunion",
            ));
        }
        Ok(Box::new(MockTUnionReader::new(self.card.clone())))
    }
}

fn synthetic_meta() -> Vec<u8> {
    vec![
        0x01, 0x00, 0x00, 0x10, 0xFF, 0xFF, 0xFF, 0xFF, 0x02, 0x01, 0x03, 0x10, 0x50, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x20, 0x22, 0x01, 0x02, 0x20, 0x72, 0x01, 0x02, 0x50, 0x00,
    ]
}

fn transaction(sequence: u16, amount: u32, kind: u8, terminal: [u8; 6], date: [u8; 7]) -> Vec<u8> {
    let mut record = vec![0u8; 23];
    record[0..2].copy_from_slice(&sequence.to_be_bytes());
    record[5..9].copy_from_slice(&amount.to_be_bytes());
    record[9] = kind;
    record[10..16].copy_from_slice(&terminal);
    record[16..23].copy_from_slice(&date);
    record
}

#[allow(clippy::too_many_arguments)]
fn travel(
    kind: u8,
    terminal: [u8; 8],
    sub_type: u8,
    station: [u8; 7],
    amount: u32,
    balance: u32,
    date: [u8; 7],
    city: [u8; 2],
    institution: [u8; 8],
) -> Vec<u8> {
    let mut record = vec![0u8; 48];
    record[0] = kind;
    record[1..9].copy_from_slice(&terminal);
    record[9] = sub_type;
    record[10..17].copy_from_slice(&station);
    record[17..21].copy_from_slice(&amount.to_be_bytes());
    record[21..25].copy_from_slice(&balance.to_be_bytes());
    record[25..32].copy_from_slice(&date);
    record[32..34].copy_from_slice(&city);
    record[34..42].copy_from_slice(&institution);
    record
}

/// Wrap a tag and value using the shortest definite length.
fn wrap(tag: &[u8], value: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(tag.len() + 3 + value.len());
    output.extend_from_slice(tag);
    if value.len() < 0x80 {
        output.push(value.len() as u8);
    } else if value.len() <= 0xFF {
        output.push(0x81);
        output.push(value.len() as u8);
    } else {
        output.push(0x82);
        output.extend_from_slice(&(value.len() as u16).to_be_bytes());
    }
    output.extend_from_slice(value);
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunion;
    use crate::tunion::hex_upper;

    #[test]
    fn mock_transport_enumerates_and_opens() {
        let transport = MockTUnionTransport::new();
        let devices = transport.enumerate().expect("enumerate");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].conn_string, "mock:tunion");
        let reader = transport.open("mock:tunion").expect("open");
        assert!(reader.apdu_capabilities().supported);
    }

    #[test]
    fn unknown_apdu_returns_unsupported_instruction() {
        let mut reader = MockTUnionReader::new(MockTUnionCard::default());
        let response = reader
            .transceive_apdu(&[0x00, 0xA4, 0x04, 0x00])
            .expect("APDU");
        assert_eq!(response.sw, [0x6D, 0x00]);
    }

    #[test]
    fn records_can_fail_with_a_protocol_error() {
        let card = MockTUnionCard {
            fail_travel: Some(1),
            ..MockTUnionCard::default()
        };
        let mut reader = MockTUnionReader::new(card);
        let response = reader
            .transceive_apdu(&apdu::read_record_apdu(1, tunion::TRAVEL_FILE))
            .expect("APDU");
        assert_eq!(response.sw, [0x6D, 0x00]);
    }

    #[test]
    fn fixture_uses_synthetic_identity() {
        let card = MockTUnionCard::default();
        let meta = hex_upper(&card.meta);
        assert!(!meta.contains("5070266901736393"));
        assert_eq!(card.balance, 5000u32.to_be_bytes());
    }
}
