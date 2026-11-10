//! Scripted Classic NDEF workflow tests. No hardware is touched.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nfc_app::Service;
use nfc_app::classic::{
    BackupStore, ClassicNdefFormat, ClassicNdefRead, ClassicNdefWrite, KeyStore,
};
use nfc_classic::{AccessBits, Layout, bcc_for_uid};
use nfc_classic_ndef::{
    MAD1_GPB, MAD2_GPB, NFC_GPB_RW, NFC_KEY_A, NFC_RW_ACCESS_BITS, NdefState, crc8,
};
use nfc_core::{BLOCK_SIZE, CardInfo, CardType, Key, KeyType, NfcError, Result};
use nfc_ndef::{NdefMessage, TextRecord};
use nfc_transport::{DeviceInfo, Reader, Transport};

const UID: [u8; 4] = [0x04, 0xA2, 0x24, 0x8A];

#[derive(Clone)]
struct TestHandle(Arc<Mutex<TestState>>);

struct TestState {
    card: Option<CardInfo>,
    blocks: BTreeMap<u8, [u8; BLOCK_SIZE]>,
    current: Option<(usize, KeyType)>,
    fail_read: Option<u8>,
    write_log: Vec<u8>,
}

impl TestHandle {
    fn new(card: CardInfo) -> Self {
        let mut blocks = BTreeMap::new();
        let mut block_zero = [0u8; BLOCK_SIZE];
        block_zero[0..4].copy_from_slice(&UID);
        block_zero[4] = bcc_for_uid(&UID).expect("bcc");
        block_zero[5] = 0x08;
        block_zero[6..8].copy_from_slice(&[0x00, 0x04]);
        blocks.insert(0, block_zero);
        for block in 1..64u8 {
            if block % 4 == 3 {
                let mut data = [0xFF; BLOCK_SIZE];
                data[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
                data[9] = 0x69;
                blocks.insert(block, data);
            } else {
                blocks.insert(block, [0u8; BLOCK_SIZE]);
            }
        }
        Self(Arc::new(Mutex::new(TestState {
            card: Some(card),
            blocks,
            current: None,
            fail_read: None,
            write_log: Vec::new(),
        })))
    }

    fn lock(&self) -> MutexGuard<'_, TestState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn block(&self, block: u8) -> Option<[u8; BLOCK_SIZE]> {
        self.lock().blocks.get(&block).copied()
    }

    fn set_block(&self, block: u8, data: [u8; BLOCK_SIZE]) {
        self.lock().blocks.insert(block, data);
    }

    fn fail_read(&self, block: u8) {
        self.lock().fail_read = Some(block);
    }

    fn write_log(&self) -> Vec<u8> {
        self.lock().write_log.clone()
    }
}

struct TestTransport(TestHandle);

impl Transport for TestTransport {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        Ok(vec![DeviceInfo::new("Test Classic", "test:classic")])
    }

    fn open(&self, _conn_string: &str) -> Result<Box<dyn Reader>> {
        Ok(Box::new(TestReader {
            info: DeviceInfo::new("Test Classic", "test:classic"),
            handle: self.0.clone(),
        }))
    }
}

struct TestReader {
    info: DeviceInfo,
    handle: TestHandle,
}

impl Reader for TestReader {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn open(&mut self, _conn_string: &str) -> Result<()> {
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        self.handle.lock().card.clone().ok_or(NfcError::NoCard)
    }

    fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()> {
        let mut state = self.handle.lock();
        let card = state.card.clone().ok_or(NfcError::NoCard)?;
        let layout = layout_for(&card)?;
        let sector = layout.sector_for_block(usize::from(block))?;
        let trailer_block = layout.trailer_block(sector)?;
        let trailer = state
            .blocks
            .get(&(trailer_block as u8))
            .copied()
            .ok_or(NfcError::AuthenticationFailed)?;
        let expected = match key_type {
            KeyType::A => &trailer[..6],
            KeyType::B => &trailer[10..],
        };
        if key.as_bytes() != expected {
            return Err(NfcError::AuthenticationFailed);
        }
        state.current = Some((sector, key_type));
        Ok(())
    }

    fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        let state = self.handle.lock();
        if state.fail_read == Some(block) {
            return Err(NfcError::Io(format!("test read of block {block} failed")));
        }
        state
            .blocks
            .get(&block)
            .copied()
            .ok_or_else(|| NfcError::Io(format!("test block {block} is not loaded")))
    }

    fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        let mut state = self.handle.lock();
        let card = state.card.clone().ok_or(NfcError::NoCard)?;
        let layout = layout_for(&card)?;
        let sector = layout.sector_for_block(usize::from(block))?;
        let trailer_block = layout.trailer_block(sector)?;
        let trailer = state
            .blocks
            .get(&(trailer_block as u8))
            .copied()
            .ok_or(NfcError::AuthenticationFailed)?;
        let Some((current_sector, key_type)) = state.current else {
            return Err(NfcError::AuthenticationFailed);
        };
        if current_sector != sector {
            return Err(NfcError::AuthenticationFailed);
        }
        let bits = AccessBits::decode([trailer[6], trailer[7], trailer[8]])?;
        let allowed = if usize::from(block) == trailer_block {
            bits.groups[3].full_trailer_write_keys()
        } else {
            let group = layout.access_group_for_block(usize::from(block))?;
            bits.groups[group].data_write_keys()
        };
        let permitted = match key_type {
            KeyType::A => allowed.allows_a(),
            KeyType::B => allowed.allows_b(),
        };
        if !permitted {
            return Err(NfcError::Unsupported("test write denied by access bits"));
        }
        state.write_log.push(block);
        state.blocks.insert(block, data);
        Ok(())
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        Err(NfcError::NotImplemented("test transceive"))
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        Err(NfcError::NotImplemented("test transceive bits"))
    }

    fn set_raw_mode(&mut self, _enabled: bool) -> Result<()> {
        Ok(())
    }
}

fn service(handle: &TestHandle) -> Service {
    let mut service = Service::with_transport(Arc::new(TestTransport(handle.clone())));
    service.refresh_devices().expect("refresh");
    service.select_device("test:classic").expect("select");
    service
}

fn layout_for(card: &CardInfo) -> Result<Layout> {
    match card.card_type {
        CardType::MifareClassic1K => Ok(Layout::Classic1K),
        CardType::MifareClassic4K => Ok(Layout::Classic4K),
        _ => Err(NfcError::Unsupported("not a Classic card")),
    }
}

fn card(layout: Layout) -> CardInfo {
    CardInfo::new(
        UID.to_vec(),
        [0x00, 0x04],
        0x08,
        match layout {
            Layout::Classic1K => CardType::MifareClassic1K,
            Layout::Classic4K => CardType::MifareClassic4K,
        },
    )
}

fn text(message: &str) -> NdefMessage {
    NdefMessage::single(
        TextRecord::new(message, "en")
            .expect("text")
            .to_record()
            .expect("record"),
    )
    .expect("message")
}

#[test]
fn format_then_write_and_read_a_classic_1k() {
    let handle = TestHandle::new(card(Layout::Classic1K));
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let formatted = service
        .classic_ndef_format(
            ClassicNdefFormat::new(&keys, &backup)
                .confirm(true)
                .include_common_keys(true),
        )
        .expect("format");
    assert_eq!(formatted.info.ndef.state, NdefState::Empty);
    let layout = formatted.info.layout.expect("layout");
    assert_eq!(layout.sectors, (1..=15).collect::<Vec<_>>());
    assert_eq!(
        &handle.block(3).expect("trailer")[..6],
        &[0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5]
    );
    assert_eq!(&handle.block(3).expect("trailer")[9], &MAD1_GPB);
    assert_eq!(&handle.block(7).expect("trailer")[..6], &NFC_KEY_A);
    assert_eq!(
        &handle.block(7).expect("trailer")[6..9],
        &NFC_RW_ACCESS_BITS
    );
    assert_eq!(handle.block(7).expect("trailer")[9], NFC_GPB_RW);
    assert_eq!(
        &handle.block(4).expect("first data block")[..3],
        &[0x03, 0x00, 0xFE]
    );
    assert_eq!(
        handle.block(8).expect("sector 2 data block"),
        [0u8; BLOCK_SIZE]
    );
    assert_eq!(
        handle.block(9).expect("sector 2 data block"),
        [0u8; BLOCK_SIZE]
    );
    assert!(formatted.backup.path.exists());

    let message = text("classic ndef");
    let write = service
        .classic_ndef_write(ClassicNdefWrite::new(message.clone(), &keys, &backup))
        .expect("write");
    assert_eq!(write.info.ndef.state, NdefState::Message(message));
    assert!(write.backup.path.exists());
    assert!(write.written_blocks >= 1);

    let read = service
        .classic_ndef_read(ClassicNdefRead::new(&keys))
        .expect("read");
    assert_eq!(
        read.info.ndef.state,
        NdefState::Message(text("classic ndef"))
    );
    assert_eq!(read.info.ndef.capacity, 715);
    assert!(read.warnings.is_empty(), "{:?}", read.warnings);
}

#[test]
fn format_read_only_prevents_ndef_writes() {
    let handle = TestHandle::new(card(Layout::Classic1K));
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let formatted = service
        .classic_ndef_format(
            ClassicNdefFormat::new(&keys, &backup)
                .confirm(true)
                .read_only(true),
        )
        .expect("format");
    assert!(formatted.info.layout.expect("layout").read_only);
    assert_eq!(
        &handle.block(7).expect("trailer")[6..9],
        &[0x07, 0x8F, 0x0F]
    );

    let error = service
        .classic_ndef_write(ClassicNdefWrite::new(text("no"), &keys, &backup))
        .expect_err("read-only write");
    assert!(error.to_string().contains("read-only"), "{error}");
}

#[test]
fn write_requires_a_formatted_card() {
    let handle = TestHandle::new(card(Layout::Classic1K));
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let error = service
        .classic_ndef_write(ClassicNdefWrite::new(text("x"), &keys, &backup))
        .expect_err("unformatted");
    assert!(error.to_string().contains("format"), "{error}");
    assert!(handle.write_log().is_empty());
}

#[test]
fn format_requires_confirmation_and_a_complete_read() {
    let handle = TestHandle::new(card(Layout::Classic1K));
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let error = service
        .classic_ndef_format(ClassicNdefFormat::new(&keys, &backup))
        .expect_err("confirmation");
    assert!(
        error.to_string().contains("confirmation is required"),
        "{error}"
    );
    assert!(handle.write_log().is_empty());

    handle.fail_read(5);
    let error = service
        .classic_ndef_format(ClassicNdefFormat::new(&keys, &backup).confirm(true))
        .expect_err("partial read");
    assert!(error.to_string().contains("complete"), "{error}");
    assert!(handle.write_log().is_empty());
}

#[test]
fn a_bad_mad_crc_is_rejected() {
    let handle = TestHandle::new(card(Layout::Classic1K));
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    service
        .classic_ndef_format(ClassicNdefFormat::new(&keys, &backup).confirm(true))
        .expect("format");
    let mut block1 = handle.block(1).expect("block 1");
    block1[0] ^= 1;
    handle.set_block(1, block1);

    let error = service
        .classic_ndef_read(ClassicNdefRead::new(&keys))
        .expect_err("CRC");
    assert!(error.to_string().contains("CRC"), "{error}");
}

#[test]
fn a_long_message_round_trips_across_sectors() {
    let handle = TestHandle::new(card(Layout::Classic1K));
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    service
        .classic_ndef_format(ClassicNdefFormat::new(&keys, &backup).confirm(true))
        .expect("format");

    let message = text(&"y".repeat(400));
    let write = service
        .classic_ndef_write(ClassicNdefWrite::new(message.clone(), &keys, &backup))
        .expect("write");
    assert!(write.written_blocks > 10);
    assert_eq!(write.info.ndef.state, NdefState::Message(message));
}

#[test]
fn format_4k_uses_mad2() {
    let handle = TestHandle(Arc::new(Mutex::new(TestState {
        card: Some(card(Layout::Classic4K)),
        blocks: BTreeMap::new(),
        current: None,
        fail_read: None,
        write_log: Vec::new(),
    })));
    {
        let mut state = handle.lock();
        let mut block_zero = [0u8; BLOCK_SIZE];
        block_zero[0..4].copy_from_slice(&UID);
        block_zero[4] = bcc_for_uid(&UID).expect("bcc");
        block_zero[5] = 0x08;
        block_zero[6..8].copy_from_slice(&[0x00, 0x04]);
        state.blocks.insert(0, block_zero);
        for sector in 0..40usize {
            let first = if sector < 32 {
                sector * 4
            } else {
                128 + (sector - 32) * 16
            };
            let count = if sector < 32 { 4 } else { 16 };
            for block in first..first + count {
                if block == first + count - 1 {
                    let mut data = [0xFF; BLOCK_SIZE];
                    data[6..9].copy_from_slice(&[0xFF, 0x07, 0x80]);
                    data[9] = 0x69;
                    state.blocks.insert(block as u8, data);
                } else {
                    state.blocks.insert(block as u8, [0u8; BLOCK_SIZE]);
                }
            }
        }
    }
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let formatted = service
        .classic_ndef_format(ClassicNdefFormat::new(&keys, &backup).confirm(true))
        .expect("format 4K");
    let layout = formatted.info.layout.expect("layout");
    assert_eq!(layout.sectors.len(), 38);
    assert_eq!(&handle.block(3).expect("trailer")[9], &MAD2_GPB);
    assert_eq!(handle.block(0x43).expect("MAD2 trailer")[9], 0);
    assert_eq!(layout.data_blocks.len(), 210);
    assert_eq!(formatted.info.ndef.capacity, 3355);
}

#[test]
fn mad_crc_detects_a_gap() {
    let handle = TestHandle::new(card(Layout::Classic1K));
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    service
        .classic_ndef_format(ClassicNdefFormat::new(&keys, &backup).confirm(true))
        .expect("format");
    let mut block1 = handle.block(1).expect("block 1");
    let block2 = handle.block(2).expect("block 2");
    block1[4..6].copy_from_slice(&[0x00, 0x00]);
    let mut crc_input = vec![block1[1]];
    crc_input.extend_from_slice(&block1[2..]);
    crc_input.extend_from_slice(&block2);
    block1[0] = crc8(&crc_input);
    handle.set_block(1, block1);
    let error = service
        .classic_ndef_read(ClassicNdefRead::new(&keys))
        .expect_err("gap");
    assert!(error.to_string().contains("not contiguous"), "{error}");
}

#[test]
fn format_preserves_the_current_key_b() {
    let handle = TestHandle::new(card(Layout::Classic1K));
    let mut trailer = handle.block(7).expect("sector 1 trailer");
    trailer[10..].copy_from_slice(&[0x11; 6]);
    handle.set_block(7, trailer);
    let mut service = service(&handle);
    let keys = KeyStore::new();
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    service
        .classic_ndef_format(ClassicNdefFormat::new(&keys, &backup).confirm(true))
        .expect("format");
    assert_eq!(&handle.block(7).expect("trailer")[10..], &[0x11; 6]);
}
