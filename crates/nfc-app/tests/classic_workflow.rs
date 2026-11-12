//! Scripted Classic workflow tests. No hardware is touched.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nfc_app::classic::{
    ClassicBlockWrite, ClassicKeyscan, ClassicRead, ClassicRestore, ClassicUidExecution,
    ClassicUidPreflight, ClassicWriteExecution, ClassicWritePreflight, KeyTypes, RestoreBlockState,
    RestoreSelection, RestoreUidState, UidWriteBackend, WorkbenchMergeReport,
};
use nfc_app::{BackupStore, KeyEntry, KeyKind, KeySource, KeyStore, Service};
use nfc_classic::{AccessBits, AccessCondition, Dump, DumpFormat, Layout, bcc_for_uid, exchange};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, CardType, Key, KeyType, NfcError, Result};
use nfc_transport::{DeviceInfo, ManufacturerWriter, Reader, ReaderCapabilities, Transport};

const UID: [u8; 4] = [0x04, 0xA2, 0x24, 0x8A];
const KEY_A: Key = Key::new([0xFF; 6]);
const KEY_B: Key = Key::new([0xA0; 6]);
const KEY_NFC: Key = Key::new([0xD3, 0xF7, 0xD3, 0xF7, 0xD3, 0xF7]);
const DENIED: Key = Key::new([0x99; 6]);

fn card() -> CardInfo {
    CardInfo::new(UID.to_vec(), [0x00, 0x04], 0x08, CardType::MifareClassic1K)
}

fn card_with_uid(uid: u8) -> CardInfo {
    CardInfo::new(vec![uid; 4], [0x00, 0x04], 0x08, CardType::MifareClassic1K)
}

fn trailer(access: [u8; 3]) -> [u8; BLOCK_SIZE] {
    let mut data = [0xFF; BLOCK_SIZE];
    data[6..9].copy_from_slice(&access);
    data[9] = 0x69;
    data
}

/// The bytes a MIFARE trailer read returns: Key A is always masked and Key B
/// is masked when the access bits hide it.
fn trailer_as_read(mut data: [u8; BLOCK_SIZE]) -> [u8; BLOCK_SIZE] {
    data[..6].fill(0);
    if let Ok(access) = AccessBits::decode([data[6], data[7], data[8]])
        && !access.groups[3].key_b_readable()
    {
        data[10..].fill(0);
    }
    data
}

fn default_image() -> BTreeMap<u8, [u8; BLOCK_SIZE]> {
    let mut blocks = BTreeMap::new();
    let mut block_zero = [0u8; BLOCK_SIZE];
    block_zero[0..4].copy_from_slice(&UID);
    block_zero[4] = bcc_for_uid(&UID).expect("bcc");
    block_zero[5] = 0x08;
    block_zero[6..8].copy_from_slice(&[0x00, 0x04]);
    blocks.insert(0, block_zero);
    for block in 1..64u8 {
        if block % 4 == 3 {
            blocks.insert(block, trailer([0xFF, 0x07, 0x80]));
        } else {
            blocks.insert(block, [block; BLOCK_SIZE]);
        }
    }
    blocks
}

#[derive(Default)]
struct TestState {
    card: Option<CardInfo>,
    blocks: BTreeMap<u8, [u8; BLOCK_SIZE]>,
    /// If a sector is present, only these key/type pairs authenticate.
    sector_rules: HashMap<u8, Vec<(KeyType, Key)>>,
    ignore_write: Option<u8>,
    fail_write: Option<u8>,
    /// Apply the write and then report an I/O error, as when the ACK is lost.
    write_applied_error: Option<u8>,
    /// Remove the card after a successful write, before the verification read.
    vanish_after_write: bool,
    /// Selection succeeds but every MIFARE authentication times out, as on a
    /// phone-emulated UID-only card.
    mifare_unsupported: bool,
    /// A denied key times out instead of returning a NAK, as some phone
    /// emulations do. A MIFARE read still answers, proving MIFARE support.
    auth_timeout_on_denied: bool,
    /// One block read times out while the card stays present.
    read_timeout_block: Option<u8>,
    /// Advertise the protected Gen1A manufacturer writer.
    manufacturer_write: bool,
    /// Fail the protected writer without touching the card.
    manufacturer_write_fails: bool,
    /// Accept an ordinary write to block 0.
    block_zero_writable: bool,
    /// Refuse an ordinary block-0 write unless the last authentication was B.
    block_zero_requires_b: bool,
    write_log: Vec<u8>,
    manufacturer_write_log: Vec<[u8; BLOCK_SIZE]>,
    auth_log: Vec<(u8, KeyType, Key)>,
    cancel_after_auths: Option<(usize, CancelToken)>,
}

impl TestState {
    fn with_card(card: CardInfo) -> Self {
        Self {
            card: Some(card),
            blocks: default_image(),
            block_zero_writable: true,
            ..Self::default()
        }
    }

    fn apply_write(&mut self, block: u8, data: [u8; BLOCK_SIZE]) {
        if block == 0
            && let Some(card) = self.card.as_mut()
        {
            card.uid = data[0..4].to_vec();
        }
        self.blocks.insert(block, data);
    }

    fn set_trailer_access(&mut self, sector: u8, access: [u8; 3]) {
        let block = sector * 4 + 3;
        self.blocks.insert(block, trailer(access));
    }

    fn accept_only(&mut self, sector: u8, keys: &[(KeyType, Key)]) {
        self.sector_rules.insert(sector, keys.to_vec());
    }
}

#[derive(Clone)]
struct TestHandle(Arc<Mutex<TestState>>);

impl TestHandle {
    fn new(state: TestState) -> Self {
        Self(Arc::new(Mutex::new(state)))
    }

    fn lock(&self) -> MutexGuard<'_, TestState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

struct TestTransport {
    handle: TestHandle,
    info: DeviceInfo,
}

impl TestTransport {
    fn new(handle: TestHandle) -> Self {
        Self {
            handle,
            info: DeviceInfo::new("Test Classic", "test:classic"),
        }
    }
}

impl Transport for TestTransport {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        Ok(vec![self.info.clone()])
    }

    fn open(&self, _conn_string: &str) -> Result<Box<dyn Reader>> {
        Ok(Box::new(TestReader {
            info: self.info.clone(),
            handle: self.handle.clone(),
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

    fn capabilities(&self) -> ReaderCapabilities {
        ReaderCapabilities {
            classic_read: true,
            classic_write: true,
            raw_bytes: true,
            raw_bits: true,
            manufacturer_write: true,
            type2_read: true,
            type2_write: true,
            recovery_parity: true,
        }
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
        if state.card.is_none() {
            return Err(NfcError::NoCard);
        }
        if state.mifare_unsupported {
            return Err(NfcError::Timeout(
                "the card did not answer MIFARE authentication".to_owned(),
            ));
        }
        state.auth_log.push((block, key_type, key));
        let sector = block / 4;
        if let Some(rules) = state.sector_rules.get(&sector)
            && !rules.contains(&(key_type, key))
        {
            if state.auth_timeout_on_denied {
                return Err(NfcError::Timeout(
                    "the card did not answer MIFARE authentication".to_owned(),
                ));
            }
            return Err(NfcError::AuthenticationFailed);
        }
        if let Some((threshold, token)) = &state.cancel_after_auths
            && state.auth_log.len() >= *threshold
        {
            token.cancel();
        }
        Ok(())
    }

    fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        let state = self.handle.lock();
        if state.card.is_none() {
            return Err(NfcError::NoCard);
        }
        if state.mifare_unsupported {
            return Err(NfcError::Timeout(
                "the card did not answer a MIFARE read".to_owned(),
            ));
        }
        if state.read_timeout_block == Some(block) {
            return Err(NfcError::Timeout(format!(
                "the card did not answer a read of block {block}"
            )));
        }
        let mut data = state
            .blocks
            .get(&block)
            .copied()
            .ok_or_else(|| NfcError::Io(format!("test block {block} is not loaded")))?;
        if block % 4 == 3 {
            data = trailer_as_read(data);
        }
        Ok(data)
    }

    fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        let mut state = self.handle.lock();
        if state.card.is_none() {
            return Err(NfcError::NoCard);
        }
        state.write_log.push(block);
        if block == 0 && !state.block_zero_writable {
            return Err(NfcError::Io(
                "test card refused the manufacturer block write".to_owned(),
            ));
        }
        if block == 0
            && state.block_zero_requires_b
            && !state
                .auth_log
                .last()
                .is_some_and(|(_, key_type, _)| *key_type == KeyType::B)
        {
            return Err(NfcError::Io(
                "test card requires Key B for manufacturer block 0".to_owned(),
            ));
        }
        if state.fail_write == Some(block) {
            return Err(NfcError::Io(format!("test write of block {block} failed")));
        }
        if state.ignore_write == Some(block) {
            return Ok(());
        }
        state.apply_write(block, data);
        if state.write_applied_error == Some(block) {
            return Err(NfcError::Io(
                "test write acknowledgement was lost".to_owned(),
            ));
        }
        if state.vanish_after_write {
            state.card = None;
        }
        Ok(())
    }

    fn manufacturer_writer(&mut self) -> Option<&mut dyn ManufacturerWriter> {
        let available = self.handle.lock().manufacturer_write;
        if available { Some(self) } else { None }
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

impl ManufacturerWriter for TestReader {
    fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        let mut state = self.handle.lock();
        if state.card.is_none() {
            return Err(NfcError::NoCard);
        }
        if !state.manufacturer_write {
            return Err(NfcError::Unsupported(
                "the test reader has no Gen1A manufacturer writer",
            ));
        }
        if state.manufacturer_write_fails {
            return Err(NfcError::Unsupported(
                "the test card did not answer the Gen1A unlock",
            ));
        }
        state.manufacturer_write_log.push(data);
        if state.ignore_write == Some(0) {
            return Ok(());
        }
        state.apply_write(0, data);
        if state.write_applied_error == Some(0) {
            return Err(NfcError::Io(
                "test write acknowledgement was lost".to_owned(),
            ));
        }
        if state.vanish_after_write {
            state.card = None;
        }
        Ok(())
    }
}

fn service(handle: &TestHandle) -> Service {
    let mut service = Service::with_transport(Arc::new(TestTransport::new(handle.clone())));
    service.refresh_devices().expect("refresh");
    service
        .select_device("test:classic")
        .expect("select test reader");
    service
}

fn key_store(keys: &[(KeyType, Key)]) -> KeyStore {
    let mut store = KeyStore::new();
    for (key_type, key) in keys {
        store
            .add(KeyEntry::new(
                None,
                match key_type {
                    KeyType::A => KeyKind::A,
                    KeyType::B => KeyKind::B,
                },
                *key,
                KeySource::Manual,
            ))
            .expect("add key");
    }
    store
}

fn read_dump(service: &mut Service, keys: &KeyStore) -> Dump {
    service
        .classic_read(ClassicRead::new(keys))
        .expect("read dump")
        .dump
}

fn ordinary_access() -> [u8; 3] {
    [0xFF, 0x07, 0x80]
}

fn read_only_second_block_access(access: [u8; 3]) -> [u8; 3] {
    let mut bits = AccessBits::decode(access).expect("valid access");
    bits.groups[1] = AccessCondition::new(false, false, true);
    bits.encode()
}

#[test]
fn dump_reads_block_zero_after_sector_auth_and_records_unreadable_sectors() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle
        .lock()
        .accept_only(1, &[(KeyType::A, DENIED), (KeyType::B, DENIED)]);
    let mut service = service(&handle);
    let store = KeyStore::new();

    let result = service
        .classic_read(ClassicRead::new(&store))
        .expect("partial dump is still a result");
    assert_eq!(result.dump.layout, Layout::Classic1K);
    assert_eq!(result.dump.card.as_ref(), Some(&card()));
    assert!(result.dump.blocks[0].complete());
    assert_eq!(result.dump.blocks[0].data[0..4], UID);
    assert_eq!(result.keys.len(), 15, "sector 1 was denied");
    assert_eq!(result.dump.metadata.keys.len(), 15);
    assert!(result.dump.metadata.started_at.is_some());
    assert!(
        result
            .keys
            .iter()
            .all(|key| key.source == KeySource::Common)
    );
    assert!(!result.keys.iter().any(|key| key.sector == 1));
    for block in 4..8 {
        assert!(
            !result.dump.blocks[block].complete(),
            "sector 1 block {block}"
        );
        assert_eq!(
            result.dump.blocks[block].status,
            nfc_classic::BlockStatus::AuthFailed
        );
    }
    assert!(result.dump.blocks[8].complete());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("sector 1")),
        "{:?}",
        result.warnings
    );
    assert!(
        handle
            .lock()
            .auth_log
            .iter()
            .any(|(block, _, _)| *block == 4)
    );
}

#[test]
fn dump_leaves_block_zero_unknown_when_sector_zero_does_not_authenticate() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle
        .lock()
        .accept_only(0, &[(KeyType::A, DENIED), (KeyType::B, DENIED)]);
    let mut service = service(&handle);
    let store = KeyStore::new();

    let result = service
        .classic_read(ClassicRead::new(&store))
        .expect("partial dump is still a result");
    assert!(!result.dump.blocks[0].complete());
    assert_eq!(
        result.dump.blocks[0].status,
        nfc_classic::BlockStatus::AuthFailed
    );
    assert!(result.dump.blocks[4].complete());
}

#[test]
fn uid_only_emulation_reads_identity_without_data() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().mifare_unsupported = true;
    let mut service = service(&handle);
    let store = KeyStore::new();

    let result = service
        .classic_read(ClassicRead::new(&store).allow_identity_only(true))
        .expect("identity-only read");
    assert!(result.identity_only);
    assert_eq!(result.dump.card.as_ref(), Some(&card()));
    assert_eq!(result.dump.readability().readable_blocks, 0);
    assert!(result.keys.is_empty());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("did not answer MIFARE authentication")),
        "{:?}",
        result.warnings
    );
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("selection identity")),
        "{:?}",
        result.warnings
    );
}

#[test]
fn uid_only_emulation_is_an_error_without_the_identity_fallback() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().mifare_unsupported = true;
    let mut service = service(&handle);
    let store = KeyStore::new();

    let result = service.classic_read(ClassicRead::new(&store));
    assert!(matches!(result, Err(NfcError::Timeout(_))));
}

#[test]
fn phone_emulation_continues_after_a_timed_out_wrong_key() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.accept_only(0, &[(KeyType::A, KEY_A)]);
        state.auth_timeout_on_denied = true;
    }
    let mut service = service(&handle);
    // The stored key is tried first and times out; the common-key fallback
    // then authenticates. The unauthenticated READ probe must prove that the
    // card speaks MIFARE so the read does not stop as identity-only.
    let store = key_store(&[(KeyType::A, DENIED)]);
    let result = service
        .classic_read(
            ClassicRead::new(&store)
                .include_common_keys(true)
                .allow_identity_only(true),
        )
        .expect("phone emulation read");
    assert!(!result.identity_only);
    assert!(result.dump.blocks[0].complete());
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("answered a MIFARE read")),
        "{:?}",
        result.warnings
    );
}

#[test]
fn dump_reuses_the_last_authenticated_key() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        for sector in 0..16 {
            state.accept_only(sector, &[(KeyType::A, KEY_NFC)]);
        }
    }
    let mut service = service(&handle);

    let result = service
        .classic_read(ClassicRead::new(&KeyStore::new()).include_common_keys(true))
        .expect("read");
    assert!(result.dump.complete());
    let auths = handle.lock().auth_log.len();
    assert!(
        auths <= 20,
        "expected the preferred key to be reused across sectors, got {auths} auths"
    );
}

#[test]
fn dump_does_not_brute_force_the_full_common_dictionary() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let late = Key::from_hex("AABBCCDDEEFF").expect("key");
    {
        let mut state = handle.lock();
        for sector in 0..16 {
            state.accept_only(sector, &[(KeyType::A, late)]);
        }
    }
    let mut service = service(&handle);

    let result = service.classic_read(ClassicRead::new(&KeyStore::new()).include_common_keys(true));
    assert!(matches!(result, Err(NfcError::AuthenticationFailed)));
    let auths = handle.lock().auth_log.len();
    assert!(
        auths <= 16 * 8,
        "a read must not scan the full dictionary, got {auths} auths"
    );
}

#[test]
fn keyscan_continues_after_a_timed_out_wrong_key() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.accept_only(0, &[(KeyType::A, KEY_A)]);
        state.auth_timeout_on_denied = true;
    }
    let mut service = service(&handle);

    let result = service
        .classic_keyscan(
            ClassicKeyscan::new(&KeyStore::new(), vec![DENIED, KEY_A])
                .include_common_keys(false)
                .key_types(KeyTypes::a_only()),
        )
        .expect("key scan on a phone emulation");
    assert_eq!(result.sectors[0].key_a, Some(KEY_A));
}

#[test]
fn read_timeout_marks_one_block_and_keeps_the_rest() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().read_timeout_block = Some(4);
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);

    let result = service
        .classic_read(ClassicRead::new(&store).include_common_keys(false))
        .expect("partial read after a block timeout");
    assert_eq!(
        result.dump.blocks[4].status,
        nfc_classic::BlockStatus::ReadFailed
    );
    assert!(result.dump.blocks[0].complete());
    assert!(result.dump.blocks[5].complete());
    assert!(!result.identity_only);
}

#[test]
fn single_block_write_rejects_block_zero_and_unconfirmed_trailers() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let block_zero = service.classic_write_block(ClassicBlockWrite::new(
        0,
        [0x11; BLOCK_SIZE],
        &store,
        &backup,
    ));
    assert!(matches!(block_zero, Err(NfcError::Unsupported(_))));
    let trailer = service.classic_write_block(ClassicBlockWrite::new(
        7,
        trailer(ordinary_access()),
        &store,
        &backup,
    ));
    assert!(matches!(trailer, Err(NfcError::InvalidArgument(_))));
    assert!(handle.lock().write_log.is_empty());
}

#[test]
fn single_block_write_rejects_access_conditions_that_forbid_the_write() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle
        .lock()
        .set_trailer_access(0, read_only_second_block_access(ordinary_access()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let result = service.classic_write_block(ClassicBlockWrite::new(
        1,
        [0x55; BLOCK_SIZE],
        &store,
        &backup,
    ));
    assert!(matches!(result, Err(NfcError::Unsupported(_))));
    assert!(handle.lock().write_log.is_empty());
}

#[test]
fn single_block_write_backs_up_before_writing_and_verifies_read_back() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let original = handle.lock().blocks[&4];

    let result = service
        .classic_write_block(ClassicBlockWrite::new(
            4,
            [0xA5; BLOCK_SIZE],
            &store,
            &backup,
        ))
        .expect("write");
    assert!(result.changed);
    assert_eq!(handle.lock().blocks[&4], [0xA5; BLOCK_SIZE]);
    assert_eq!(handle.lock().write_log, vec![4]);
    let record = result.backup.expect("backup");
    let backup_dump =
        nfc_classic::exchange::load(std::fs::read(&record.path).expect("backup file").as_slice())
            .expect("load backup")
            .dump;
    assert_eq!(backup_dump.blocks[4].data, original);

    let unchanged = service
        .classic_write_block(ClassicBlockWrite::new(
            4,
            [0xA5; BLOCK_SIZE],
            &store,
            &backup,
        ))
        .expect("unchanged write");
    assert!(!unchanged.changed);
    assert!(unchanged.backup.is_none());
    assert_eq!(handle.lock().write_log, vec![4]);
}

#[test]
fn single_block_write_refuses_a_card_swap_before_backup() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let source = read_dump(&mut service, &store);
    let expected = source.card.clone().expect("source identity");

    handle.lock().card = Some(card_with_uid(0x55));
    let result = service.classic_write_block(
        ClassicBlockWrite::new(4, [0xA5; BLOCK_SIZE], &store, &backup).expected_card(expected),
    );
    assert!(matches!(result, Err(NfcError::CardChanged)));
    assert!(handle.lock().write_log.is_empty());
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("backup dir")
            .count(),
        0
    );
}

#[test]
fn single_block_write_detects_an_ignored_write() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().ignore_write = Some(5);
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let original = handle.lock().blocks[&5];

    let result = service.classic_write_block(ClassicBlockWrite::new(
        5,
        [0x77; BLOCK_SIZE],
        &store,
        &backup,
    ));
    assert!(matches!(result, Err(NfcError::VerificationFailed(_))));
    assert_eq!(handle.lock().blocks[&5], original);
}

#[test]
fn restore_writes_data_blocks_before_the_trailer_and_backs_up() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x41; BLOCK_SIZE]);
    source.blocks[5].set_read([0x42; BLOCK_SIZE]);
    source.blocks[6].set_read([0x43; BLOCK_SIZE]);
    let mut new_trailer = trailer([0xFF, 0x07, 0x80]);
    new_trailer[10..16].copy_from_slice(&[0xBB; 6]);
    source.blocks[7].set_read(new_trailer);
    let original_trailer = handle.lock().blocks[&7];

    let result = service
        .classic_restore(ClassicRestore::new(source, &store, &backup).confirmed(true))
        .expect("restore");
    assert_eq!(result.written_blocks, 4);
    assert_eq!(result.trailer_writes, 1);
    assert!(result.failures.is_empty());
    assert_eq!(result.blocks.len(), 64);
    assert_eq!(result.blocks[0].state, RestoreBlockState::Unchanged);
    for block in 4..=7 {
        assert_eq!(result.blocks[block].state, RestoreBlockState::Written);
    }
    assert_eq!(result.blocks[8].state, RestoreBlockState::Unchanged);
    assert_eq!(handle.lock().write_log, vec![4, 5, 6, 7]);
    assert_eq!(handle.lock().blocks[&7], new_trailer);
    let record = result.backup.expect("backup");
    let backup_dump =
        nfc_classic::exchange::load(std::fs::read(&record.path).expect("backup file").as_slice())
            .expect("load backup")
            .dump;
    assert_eq!(backup_dump.blocks[4].data, [4; BLOCK_SIZE]);
    assert_eq!(
        backup_dump.blocks[7].data,
        trailer_as_read(original_trailer)
    );
}

#[test]
fn restore_stops_at_the_first_failure_by_default() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().fail_write = Some(5);
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    for block in 4..7 {
        source.blocks[block].set_read([block as u8 + 0x40; BLOCK_SIZE]);
    }
    let mut new_trailer = trailer([0xFF, 0x07, 0x80]);
    new_trailer[10..16].copy_from_slice(&[0xBB; 6]);
    source.blocks[7].set_read(new_trailer);
    let original_trailer = handle.lock().blocks[&7];

    let result =
        service.classic_restore(ClassicRestore::new(source, &store, &backup).confirmed(true));
    assert!(matches!(result, Err(NfcError::VerificationFailed(_))));
    assert_eq!(handle.lock().write_log, vec![4, 5]);
    assert_eq!(handle.lock().blocks[&4], [0x44; BLOCK_SIZE]);
    assert_eq!(handle.lock().blocks[&5], [5; BLOCK_SIZE]);
    assert_eq!(handle.lock().blocks[&7], original_trailer);
}

#[test]
fn restore_honors_the_explicit_continue_policy() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().fail_write = Some(5);
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    for block in 4..7 {
        source.blocks[block].set_read([block as u8 + 0x40; BLOCK_SIZE]);
    }
    let mut new_trailer = trailer([0xFF, 0x07, 0x80]);
    new_trailer[10..16].copy_from_slice(&[0xBB; 6]);
    source.blocks[7].set_read(new_trailer);

    let result = service
        .classic_restore(
            ClassicRestore::new(source, &store, &backup)
                .confirmed(true)
                .continue_on_failure(true),
        )
        .expect("continue restore");
    assert_eq!(result.written_blocks, 3);
    assert_eq!(result.failures.len(), 1);
    assert!(result.failures[0].contains("block 5"));
    assert_eq!(result.blocks[5].state, RestoreBlockState::Failed);
    assert_eq!(result.blocks[4].state, RestoreBlockState::Written);
    assert_eq!(result.blocks[7].state, RestoreBlockState::Written);
    assert_eq!(handle.lock().write_log, vec![4, 5, 6, 7]);
    assert_eq!(handle.lock().blocks[&4], [0x44; BLOCK_SIZE]);
    assert_eq!(handle.lock().blocks[&5], [5; BLOCK_SIZE]);
    assert_eq!(handle.lock().blocks[&6], [0x46; BLOCK_SIZE]);
}

#[test]
fn restore_rejects_identity_mismatch_and_block_zero_changes_before_writing() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let mut different_card = read_dump(&mut service, &store);
    different_card.card = Some(card_with_uid(0x77));
    let result = service
        .classic_restore(ClassicRestore::new(different_card, &store, &backup).confirmed(true));
    assert!(matches!(result, Err(NfcError::CardChanged)));
    assert!(handle.lock().write_log.is_empty());
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("backup dir")
            .count(),
        0
    );

    let mut different_block_zero = read_dump(&mut service, &store);
    different_block_zero.card = None;
    different_block_zero.blocks[0].data[8] = 0xAA;
    let result = service.classic_restore(
        ClassicRestore::new(different_block_zero, &store, &backup).confirmed(true),
    );
    assert!(matches!(result, Err(NfcError::VerificationFailed(_))));
    assert!(handle.lock().write_log.is_empty());
}

#[test]
fn restore_requires_explicit_confirmation() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let source = read_dump(&mut service, &store);

    let result = service.classic_restore(ClassicRestore::new(source, &store, &backup));
    assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
    assert!(handle.lock().write_log.is_empty());
}

#[test]
fn keyscan_finds_a_and_b_keys_and_counts_attempts() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.accept_only(0, &[(KeyType::A, KEY_A), (KeyType::B, KEY_B)]);
        state.accept_only(1, &[(KeyType::A, KEY_A)]);
        for sector in 2..16 {
            state.accept_only(sector, &[(KeyType::A, DENIED), (KeyType::B, DENIED)]);
        }
    }
    let mut service = service(&handle);
    let store = KeyStore::new();
    let result = service
        .classic_keyscan(
            ClassicKeyscan::new(&store, vec![KEY_A, KEY_B])
                .include_common_keys(false)
                .key_types(KeyTypes::ALL),
        )
        .expect("keyscan");
    assert_eq!(result.sectors[0].key_a, Some(KEY_A));
    assert_eq!(result.sectors[0].key_b, Some(KEY_B));
    assert_eq!(result.sectors[1].key_a, Some(KEY_A));
    assert_eq!(result.sectors[1].key_b, None);
    assert_eq!(result.verified, 3);
    // Sector 0 takes three attempts, sector 1 takes three, and the remaining
    // fourteen sectors take four each.
    assert_eq!(result.attempts, 3 + 3 + 14 * 4);
    assert_eq!(result.store.len(), 3);
}

#[test]
fn keyscan_returns_card_scoped_verified_keys() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.accept_only(0, &[(KeyType::A, KEY_A)]);
        state.accept_only(1, &[(KeyType::B, KEY_B)]);
        for sector in 2..16 {
            state.accept_only(sector, &[(KeyType::A, DENIED)]);
        }
    }
    let mut service = service(&handle);
    let store = KeyStore::new();
    let result = service
        .classic_keyscan(
            ClassicKeyscan::new(&store, vec![KEY_A, KEY_B])
                .include_common_keys(false)
                .key_types(KeyTypes::ALL),
        )
        .expect("keyscan");
    assert_eq!(result.card, card());
    assert_eq!(result.verified_keys.len(), 2);
    assert!(result.verified_keys.iter().all(|entry| {
        entry.source == KeySource::KeyScan
            && ((entry.sector == 0 && entry.key == KEY_A)
                || (entry.sector == 1 && entry.key == KEY_B))
    }));

    let mut captured = store;
    captured
        .record_verified(&result.card, &result.verified_keys)
        .expect("record");
    assert_eq!(captured.verified_count(&card()), 2);
    assert_eq!(
        captured.candidates_for_card(&card(), 0)[0],
        (KeyType::A, KEY_A, KeySource::KeyScan)
    );
}

#[test]
fn keyscan_reports_progress_after_each_sector() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.accept_only(0, &[(KeyType::A, KEY_A)]);
        for sector in 1..16 {
            state.accept_only(sector, &[(KeyType::A, DENIED)]);
        }
    }
    let mut service = service(&handle);
    let store = KeyStore::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);
    let result = service
        .classic_keyscan(
            ClassicKeyscan::new(&store, vec![KEY_A, DENIED])
                .include_common_keys(false)
                .progress(Some(Arc::new(move |progress| {
                    sink.lock().expect("progress lock").push(progress);
                }))),
        )
        .expect("keyscan");
    let events = events.lock().expect("progress lock");
    assert_eq!(events.len(), 16);
    assert_eq!(events[0].sector, 0);
    assert_eq!(events[0].sector_count, 16);
    assert_eq!(events[15].sector, 15);
    assert_eq!(events[15].attempts, result.attempts);
    assert_eq!(events[15].verified, result.verified);
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].attempts <= pair[1].attempts)
    );
}

#[test]
fn keyscan_stops_when_the_token_is_cancelled() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let token = CancelToken::new();
    handle.lock().cancel_after_auths = Some((2, token.clone()));
    let mut service = service(&handle);
    let store = KeyStore::new();
    let result = service
        .classic_keyscan(
            ClassicKeyscan::new(&store, vec![DENIED])
                .include_common_keys(false)
                .cancel(token),
        )
        .expect("partial result");
    assert!(result.cancelled);
    assert!(!result.verified_keys.is_empty(), "confirmed keys are kept");
    assert_eq!(handle.lock().auth_log.len(), 2);
}

#[test]
fn read_block_authenticates_the_sector_for_block_zero_too() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.accept_only(0, &[(KeyType::B, KEY_B)]);
        state.accept_only(1, &[(KeyType::B, KEY_B)]);
    }
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::B, KEY_B)]);

    let zero = service
        .classic_read_block(0, &store, true, CancelToken::new())
        .expect("block zero");
    assert_eq!(zero.data[0..4], UID);
    assert_eq!(zero.key.key_type, KeyType::B);
    assert_eq!(zero.key.key, KEY_B);
    assert!(handle.lock().auth_log.contains(&(0, KeyType::B, KEY_B)));
    let block_four = service
        .classic_read_block(4, &store, false, CancelToken::new())
        .expect("block four");
    assert_eq!(block_four.data, [4; BLOCK_SIZE]);
    assert_eq!(block_four.key.sector, 1);
    assert!(handle.lock().auth_log.contains(&(4, KeyType::B, KEY_B)));
}

#[test]
fn selective_restore_writes_only_the_requested_blocks() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    for block in 4..=6 {
        source.blocks[block].set_read([block as u8 + 0x40; BLOCK_SIZE]);
    }

    let result = service
        .classic_restore(
            ClassicRestore::new(source, &store, &backup)
                .confirmed(true)
                .selection(RestoreSelection::Blocks(vec![4, 6])),
        )
        .expect("selective restore");
    assert_eq!(result.written_blocks, 2);
    assert_eq!(result.not_selected_blocks, 62);
    assert_eq!(handle.lock().write_log, vec![4, 6]);
    assert_eq!(handle.lock().blocks[&4], [0x44; BLOCK_SIZE]);
    assert_eq!(handle.lock().blocks[&5], [5; BLOCK_SIZE]);
    assert_eq!(result.blocks[5].state, RestoreBlockState::NotSelected);
    assert_eq!(result.blocks[0].state, RestoreBlockState::NotSelected);
}

#[test]
fn changes_only_uses_the_baseline_and_refuses_an_empty_plan() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let baseline = read_dump(&mut service, &store);
    let mut source = baseline.clone();
    source.blocks[4].set_read([0x99; BLOCK_SIZE]);

    let result = service
        .classic_restore(
            ClassicRestore::new(source.clone(), &store, &backup)
                .confirmed(true)
                .selection(RestoreSelection::ChangesOnly)
                .baseline(baseline.clone()),
        )
        .expect("changes-only restore");
    assert_eq!(result.written_blocks, 1);
    assert_eq!(handle.lock().write_log, vec![4]);

    let result = service.classic_restore(
        ClassicRestore::new(baseline.clone(), &store, &backup)
            .confirmed(true)
            .selection(RestoreSelection::ChangesOnly)
            .baseline(baseline),
    );
    assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
    assert_eq!(handle.lock().write_log, vec![4]);
}

#[test]
fn dry_run_reports_the_plan_without_a_backup_or_write() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0xAB; BLOCK_SIZE]);

    let result = service
        .classic_restore(
            ClassicRestore::new(source, &store, &backup)
                .dry_run(true)
                .selection(RestoreSelection::Blocks(vec![4])),
        )
        .expect("dry run");
    assert!(result.dry_run);
    assert_eq!(result.planned_blocks, 1);
    assert_eq!(result.written_blocks, 0);
    assert!(result.backup.is_none());
    assert_eq!(result.blocks[4].state, RestoreBlockState::Planned);
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("dry run"))
    );
    assert!(handle.lock().write_log.is_empty());
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("backup dir")
            .count(),
        0
    );
}

#[test]
fn preflight_token_rechecks_the_source_and_card_and_is_single_use() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x5A; BLOCK_SIZE]);

    let plan = service
        .classic_write_preflight(
            ClassicWritePreflight::new(source.clone(), &store)
                .selection(RestoreSelection::Blocks(vec![4])),
        )
        .expect("preflight");
    assert_eq!(plan.blocks.len(), 1);
    assert_eq!(plan.blocks[0].block, 4);
    assert_eq!(plan.blocks[0].new, [0x5A; BLOCK_SIZE]);
    assert!(plan.source_fingerprint != 0);

    let mut changed_source = source.clone();
    changed_source.blocks[5].set_read([0x77; BLOCK_SIZE]);
    let result = service.classic_write_execute(
        ClassicWriteExecution::new(plan.token, changed_source, &store, &backup).confirmed(true),
    );
    assert!(matches!(result, Err(NfcError::CardChanged)));
    assert!(handle.lock().write_log.is_empty());

    let plan = service
        .classic_write_preflight(
            ClassicWritePreflight::new(source.clone(), &store)
                .selection(RestoreSelection::Blocks(vec![4])),
        )
        .expect("second preflight");
    handle.lock().blocks.insert(4, [0x11; BLOCK_SIZE]);
    let result = service.classic_write_execute(
        ClassicWriteExecution::new(plan.token, source.clone(), &store, &backup).confirmed(true),
    );
    assert!(matches!(result, Err(NfcError::CardChanged)));
    assert!(handle.lock().write_log.is_empty());

    let plan = service
        .classic_write_preflight(
            ClassicWritePreflight::new(source.clone(), &store)
                .selection(RestoreSelection::Blocks(vec![4])),
        )
        .expect("third preflight");
    let token = plan.token;
    let result = service
        .classic_write_execute(
            ClassicWriteExecution::new(token, source.clone(), &store, &backup).confirmed(true),
        )
        .expect("planned write");
    assert_eq!(result.written_blocks, 1);
    assert_eq!(handle.lock().blocks[&4], [0x5A; BLOCK_SIZE]);
    let reused = service.classic_write_execute(
        ClassicWriteExecution::new(token, source, &store, &backup).confirmed(true),
    );
    assert!(matches!(reused, Err(NfcError::InvalidArgument(_))));
}

#[test]
fn workbench_merge_refuses_a_different_card() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let source = read_dump(&mut service, &store);
    service.workbench_load(source.clone()).expect("load");
    let mut other = Dump::new(source.layout);
    other.card = Some(card_with_uid(0x77));
    other.blocks[4].set_read([0x10; BLOCK_SIZE]);
    assert!(matches!(
        service.workbench_merge(&other),
        Err(NfcError::CardChanged)
    ));
}

#[test]
fn workbench_service_tracks_edits_and_merges_reads() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let source = read_dump(&mut service, &store);

    let snapshot = service.workbench_load(source.clone()).expect("load");
    assert!(snapshot.is_saved());
    let snapshot = service.workbench_edit(4, [0xC3; BLOCK_SIZE]).expect("edit");
    assert_eq!(snapshot.dirty_blocks, vec![4]);
    assert_eq!(snapshot.dump.blocks[4].data, [0xC3; BLOCK_SIZE]);

    let report = service
        .workbench_merge_block(5, [0x55; BLOCK_SIZE])
        .expect("merge");
    assert_eq!(report.refreshed, vec![5]);
    let snapshot = service.workbench_snapshot().expect("snapshot");
    assert_eq!(snapshot.dump.blocks[5].data, [0x55; BLOCK_SIZE]);

    let report = service
        .workbench_merge_block(4, [0xC3; BLOCK_SIZE])
        .expect("reconcile");
    assert_eq!(report.reconciled, vec![4]);
    assert!(service.workbench_snapshot().expect("snapshot").is_saved());

    service.workbench_edit(6, [0x66; BLOCK_SIZE]).expect("edit");
    let snapshot = service.workbench_undo().expect("undo");
    assert!(snapshot.is_saved());
    service.workbench_edit(6, [0x66; BLOCK_SIZE]).expect("edit");
    let snapshot = service.workbench_revert().expect("revert");
    assert!(snapshot.is_saved());
    assert_eq!(snapshot.dump.blocks[6].data, source.blocks[6].data);
    assert!(service.workbench_validate().expect("validate").is_valid());
    let exported = service
        .workbench_export(DumpFormat::Project)
        .expect("export workbench");
    let reloaded = exchange::load(&exported).expect("reload export").dump;
    assert_eq!(
        reloaded,
        service.workbench_snapshot().expect("snapshot").dump
    );
    assert!(matches!(
        service.workbench_edit(0, [0; BLOCK_SIZE]),
        Err(NfcError::Unsupported(_))
    ));
    let _: WorkbenchMergeReport = report;
}

#[test]
fn preflight_refuses_a_card_change_outside_the_planned_blocks() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x3C; BLOCK_SIZE]);

    let plan = service
        .classic_write_preflight(
            ClassicWritePreflight::new(source.clone(), &store)
                .selection(RestoreSelection::Blocks(vec![4])),
        )
        .expect("preflight");
    handle.lock().blocks.insert(5, [0x12; BLOCK_SIZE]);
    let result = service.classic_write_execute(
        ClassicWriteExecution::new(plan.token, source, &store, &backup).confirmed(true),
    );
    assert!(matches!(result, Err(NfcError::CardChanged)));
    assert!(handle.lock().write_log.is_empty());
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("backup dir")
            .count(),
        0
    );
}

const TARGET_UID: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

fn uid_preflight(
    service: &mut Service,
    store: &KeyStore,
    target: [u8; 4],
) -> Result<nfc_app::classic::ClassicUidPlan> {
    service.classic_uid_preflight(
        ClassicUidPreflight::new(target, store)
            .include_common_keys(false)
            .advanced(true),
    )
}

fn uid_execute(
    service: &mut Service,
    store: &KeyStore,
    backup: &BackupStore,
    plan: &nfc_app::classic::ClassicUidPlan,
) -> Result<nfc_app::classic::ClassicUidResult> {
    service.classic_uid_execute(
        ClassicUidExecution::new(plan.token, store, backup)
            .include_common_keys(false)
            .advanced(true)
            .confirmed(true),
    )
}

#[test]
fn uid_write_changes_block_zero_and_reselects_the_new_uid() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let original = handle.lock().blocks[&0];
    let original_trailer = handle.lock().blocks[&3];

    let plan = uid_preflight(&mut service, &store, TARGET_UID).expect("preflight");
    assert!(plan.changed);
    assert_eq!(plan.backends, vec![UidWriteBackend::Standard]);
    assert_eq!(plan.current_uid, UID);
    assert_eq!(plan.old_block0, original);
    assert_eq!(plan.new_block0[0..4], TARGET_UID);
    assert_eq!(
        plan.new_block0[4],
        bcc_for_uid(&TARGET_UID).expect("target bcc")
    );
    assert_eq!(plan.new_block0[5..], original[5..]);

    let result = uid_execute(&mut service, &store, &backup, &plan).expect("uid write");
    assert!(result.changed);
    assert_eq!(result.backend, Some(UidWriteBackend::Standard));
    assert_eq!(result.card_after.uid, TARGET_UID.to_vec());
    assert_eq!(result.old_block0, original);
    assert_eq!(result.new_block0, plan.new_block0);
    assert_eq!(handle.lock().blocks[&0], plan.new_block0);
    assert_eq!(handle.lock().card.as_ref().expect("card").uid, TARGET_UID);
    assert_eq!(handle.lock().write_log, vec![0]);
    assert!(handle.lock().manufacturer_write_log.is_empty());

    let record = result.backup.expect("backup");
    assert_eq!(record.old_block0, original);
    assert_eq!(record.new_block0, plan.new_block0);
    assert_eq!(record.old_uid, UID);
    assert_eq!(record.target_uid, TARGET_UID);
    let backup_dump = exchange::load(
        std::fs::read(&record.backup.path)
            .expect("backup file")
            .as_slice(),
    )
    .expect("load backup")
    .dump;
    assert_eq!(backup_dump.blocks[0].data, original);
    assert_eq!(
        backup_dump.blocks[3].data,
        trailer_as_read(original_trailer)
    );

    let read = service
        .classic_read_block(0, &store, false, CancelToken::new())
        .expect("read the new block 0");
    assert_eq!(read.card.uid, TARGET_UID.to_vec());
    assert_eq!(read.data, plan.new_block0);

    assert!(matches!(
        uid_execute(&mut service, &store, &backup, &plan),
        Err(NfcError::InvalidArgument(_))
    ));
}

#[test]
fn uid_write_authenticates_with_the_key_type_the_access_bits_allow() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.block_zero_requires_b = true;
        let mut access = AccessBits::decode(ordinary_access()).expect("access");
        access.groups[0] = AccessCondition::new(false, true, true);
        state.set_trailer_access(0, access.encode());
    }
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A), (KeyType::B, KEY_B)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let plan = uid_preflight(&mut service, &store, TARGET_UID).expect("preflight");
    assert_eq!(plan.backends, vec![UidWriteBackend::Standard]);
    assert_eq!(plan.sector0_key.0, KeyType::A);
    assert_eq!(plan.write_key, Some((KeyType::B, KEY_B)));
    assert!(plan.write_keys.allows(KeyType::B));

    let result = uid_execute(&mut service, &store, &backup, &plan).expect("standard write");
    assert_eq!(result.backend, Some(UidWriteBackend::Standard));
    assert_eq!(handle.lock().blocks[&0], plan.new_block0);
    assert!(
        handle
            .lock()
            .auth_log
            .iter()
            .any(|(block, key_type, key)| *block == 0 && *key_type == KeyType::B && *key == KEY_B)
    );
}

#[test]
fn uid_write_uses_gen1a_when_access_bits_deny_a_standard_write() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.manufacturer_write = true;
        state.block_zero_writable = false;
        let mut access = AccessBits::decode(ordinary_access()).expect("access");
        access.groups[0] = AccessCondition::new(true, true, true);
        state.set_trailer_access(0, access.encode());
    }
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let plan = uid_preflight(&mut service, &store, TARGET_UID).expect("preflight");
    assert_eq!(plan.backends, vec![UidWriteBackend::Gen1a]);
    assert!(!plan.write_keys.any());
    assert!(
        plan.warnings
            .iter()
            .any(|warning| warning.contains("access bits")),
        "{:?}",
        plan.warnings
    );

    let result = uid_execute(&mut service, &store, &backup, &plan).expect("Gen1A write");
    assert_eq!(result.backend, Some(UidWriteBackend::Gen1a));
    assert_eq!(result.card_after.uid, TARGET_UID.to_vec());
    assert_eq!(handle.lock().manufacturer_write_log, vec![plan.new_block0]);
    assert!(handle.lock().write_log.is_empty());
    assert_eq!(handle.lock().blocks[&0], plan.new_block0);
}

#[test]
fn uid_write_prefers_gen1a_and_falls_back_to_the_standard_path() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.manufacturer_write = true;
        state.manufacturer_write_fails = true;
    }
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let plan = uid_preflight(&mut service, &store, TARGET_UID).expect("preflight");
    assert_eq!(
        plan.backends,
        vec![UidWriteBackend::Gen1a, UidWriteBackend::Standard]
    );
    let result = uid_execute(&mut service, &store, &backup, &plan).expect("fallback write");
    assert_eq!(result.backend, Some(UidWriteBackend::Standard));
    assert_eq!(handle.lock().write_log, vec![0]);
    assert!(handle.lock().manufacturer_write_log.is_empty());
}

#[test]
fn uid_write_of_the_current_uid_needs_no_backup_or_write() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());

    let plan = uid_preflight(&mut service, &store, UID).expect("preflight");
    assert!(!plan.changed);
    assert_eq!(plan.old_block0, plan.new_block0);
    let result = uid_execute(&mut service, &store, &backup, &plan).expect("unchanged");
    assert!(!result.changed);
    assert!(result.backend.is_none());
    assert!(result.backup.is_none());
    assert!(handle.lock().write_log.is_empty());
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("backup dir")
            .count(),
        0
    );
}

#[test]
fn uid_write_requires_advanced_mode_and_a_sector_zero_key() {
    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        state.accept_only(0, &[(KeyType::A, KEY_A)]);
    }
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, DENIED)]);
    let directory = tempfile::tempdir().expect("temp dir");

    let plain = service.classic_uid_preflight(
        ClassicUidPreflight::new(TARGET_UID, &store).include_common_keys(false),
    );
    assert!(matches!(plain, Err(NfcError::InvalidArgument(_))));

    let result = service.classic_uid_preflight(
        ClassicUidPreflight::new(TARGET_UID, &store)
            .include_common_keys(false)
            .advanced(true),
    );
    assert!(matches!(result, Err(NfcError::AuthenticationFailed)));
    assert!(handle.lock().write_log.is_empty());
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("backup dir")
            .count(),
        0
    );
}

#[test]
fn uid_write_refuses_broken_access_bits_and_a_broken_block_zero() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().set_trailer_access(0, [0x00, 0x00, 0x00]);
    let mut first = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let result = uid_preflight(&mut first, &store, TARGET_UID);
    assert!(
        matches!(result, Err(NfcError::InvalidArgument(_))),
        "{result:?}"
    );

    let handle = TestHandle::new(TestState::with_card(card()));
    {
        let mut state = handle.lock();
        let mut block = state.blocks[&0];
        block[4] ^= 0xFF;
        state.blocks.insert(0, block);
    }
    let mut second = service(&handle);
    let result = uid_preflight(&mut second, &store, TARGET_UID);
    assert!(
        matches!(result, Err(NfcError::VerificationFailed(_))),
        "{result:?}"
    );
    assert!(handle.lock().write_log.is_empty());
}

#[test]
fn uid_write_detects_an_ignored_write_and_an_uncertain_acknowledgement() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().ignore_write = Some(0);
    let mut first = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let plan = uid_preflight(&mut first, &store, TARGET_UID).expect("preflight");
    let result = uid_execute(&mut first, &store, &backup, &plan);
    assert!(
        matches!(result, Err(NfcError::VerificationFailed(_))),
        "{result:?}"
    );
    assert_eq!(handle.lock().blocks[&0][0..4], UID);

    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().write_applied_error = Some(0);
    let mut second = service(&handle);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let plan = uid_preflight(&mut second, &store, TARGET_UID).expect("preflight");
    let result = uid_execute(&mut second, &store, &backup, &plan).expect("lost ack");
    assert!(result.changed);
    assert!(
        result
            .warnings
            .iter()
            .any(|warning| warning.contains("acknowledgement was lost")),
        "{:?}",
        result.warnings
    );
    assert_eq!(handle.lock().blocks[&0], plan.new_block0);
}

#[test]
fn uid_write_never_claims_success_when_the_card_disappears() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().vanish_after_write = true;
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let plan = uid_preflight(&mut service, &store, TARGET_UID).expect("preflight");
    let result = uid_execute(&mut service, &store, &backup, &plan);
    match result {
        Err(NfcError::VerificationFailed(message)) => {
            assert!(message.contains("unknown"), "{message}");
            assert!(message.contains("Backup"), "{message}");
        }
        other => panic!("unexpected result: {other:?}"),
    }
    assert!(handle.lock().card.is_none());
    assert_eq!(handle.lock().blocks[&0], plan.new_block0);
}

#[test]
fn uid_write_refuses_a_card_change_and_a_cancelled_operation() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut first = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let plan = uid_preflight(&mut first, &store, TARGET_UID).expect("preflight");
    handle.lock().blocks.insert(1, [0x5A; BLOCK_SIZE]);
    let result = uid_execute(&mut first, &store, &backup, &plan);
    assert!(matches!(result, Err(NfcError::CardChanged)));
    assert!(handle.lock().write_log.is_empty());

    let handle = TestHandle::new(TestState::with_card(card()));
    let mut second = service(&handle);
    let plan = uid_preflight(&mut second, &store, TARGET_UID).expect("preflight");
    let cancel = CancelToken::new();
    cancel.cancel();
    let result = second.classic_uid_execute(
        ClassicUidExecution::new(plan.token, &store, &backup)
            .include_common_keys(false)
            .advanced(true)
            .confirmed(true)
            .cancel(cancel),
    );
    assert!(matches!(result, Err(NfcError::Cancelled)));
    assert!(handle.lock().write_log.is_empty());
}

#[test]
fn uid_write_on_an_immutable_card_changes_nothing() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().block_zero_writable = false;
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let plan = uid_preflight(&mut service, &store, TARGET_UID).expect("preflight");
    let result = uid_execute(&mut service, &store, &backup, &plan);
    match result {
        Err(NfcError::VerificationFailed(message)) => {
            assert!(message.contains("not changed"), "{message}");
        }
        other => panic!("unexpected result: {other:?}"),
    }
    assert_eq!(handle.lock().blocks[&0][0..4], UID);
    assert_eq!(handle.lock().card.as_ref().expect("card").uid, UID);
}

fn restore_uid_preflight(
    service: &mut Service,
    store: &KeyStore,
    dump: &Dump,
    target: [u8; 4],
) -> Result<nfc_app::classic::ClassicWritePlan> {
    service.classic_write_preflight(
        ClassicWritePreflight::new(dump.clone(), store)
            .include_common_keys(false)
            .uid_target(target)
            .advanced(true),
    )
}

fn restore_uid_execute(
    service: &mut Service,
    store: &KeyStore,
    backup: &BackupStore,
    token: nfc_app::classic::WriteToken,
    dump: Dump,
    advanced: bool,
) -> Result<nfc_app::classic::ClassicRestoreResult> {
    service.classic_write_execute(
        ClassicWriteExecution::new(token, dump, store, backup)
            .include_common_keys(false)
            .advanced(advanced)
            .confirmed(true),
    )
}

#[test]
fn restore_with_uid_restores_the_dump_then_changes_and_verifies_the_uid() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x41; BLOCK_SIZE]);
    let original_block0 = handle.lock().blocks[&0];

    let plan = restore_uid_preflight(&mut service, &store, &source, TARGET_UID)
        .expect("restore UID preflight");
    let uid = plan.uid.as_ref().expect("UID plan");
    assert_eq!(uid.target_uid, TARGET_UID);
    assert_eq!(uid.old_block0, original_block0);
    assert_eq!(uid.current_uid, UID);
    assert_eq!(uid.backends, vec![UidWriteBackend::Standard]);
    assert_eq!(plan.blocks.len(), 1);
    assert_eq!(plan.blocks[0].block, 4);

    let result = restore_uid_execute(
        &mut service,
        &store,
        &backup,
        plan.token,
        source.clone(),
        true,
    )
    .expect("restore and UID write");
    assert!(result.failures.is_empty());
    assert_eq!(result.written_blocks, 1);
    assert_eq!(handle.lock().write_log, vec![4, 0]);
    let uid = result.uid.expect("UID result");
    assert_eq!(uid.state, RestoreUidState::Applied);
    assert_eq!(uid.backend, Some(UidWriteBackend::Standard));
    assert_eq!(uid.old_block0, original_block0);
    assert_eq!(uid.target_uid, TARGET_UID);
    assert_eq!(
        uid.card_after.as_ref().expect("card after").uid,
        TARGET_UID.to_vec()
    );
    let uid_backup = uid.backup.expect("UID backup");
    assert!(uid_backup.backup.path.exists());
    assert_eq!(handle.lock().blocks[&4], [0x41; BLOCK_SIZE]);
    assert_eq!(&handle.lock().blocks[&0][0..4], TARGET_UID.as_slice());

    let read = service
        .classic_read(ClassicRead::new(&store).include_common_keys(false))
        .expect("read after restore");
    assert_eq!(read.dump.card.expect("card").uid, TARGET_UID.to_vec());
    assert_eq!(read.dump.blocks[4].data, [0x41; BLOCK_SIZE]);
    assert_eq!(read.dump.blocks[0].data, uid.new_block0);
}

#[test]
fn restore_with_uid_uses_the_restored_trailer_key_for_the_uid_write() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A), (KeyType::B, KEY_B)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x44; BLOCK_SIZE]);
    let mut access = AccessBits::decode(ordinary_access()).expect("access");
    access.groups[0] = AccessCondition::new(false, true, true);
    let mut new_trailer = trailer(access.encode());
    new_trailer[..6].fill(0);
    new_trailer[10..16].copy_from_slice(KEY_B.as_bytes());
    source.blocks[3].set_read(new_trailer);

    let plan = restore_uid_preflight(&mut service, &store, &source, TARGET_UID)
        .expect("restore UID preflight");
    let uid = plan.uid.as_ref().expect("UID plan");
    assert_eq!(uid.sector0_key, (KeyType::B, KEY_B));
    assert_eq!(uid.write_key, Some((KeyType::B, KEY_B)));
    assert_eq!(uid.old_trailer, trailer_as_read(new_trailer));
    assert!(uid.write_keys.b);

    let result = restore_uid_execute(&mut service, &store, &backup, plan.token, source, true)
        .expect("restore and UID write");
    assert!(result.failures.is_empty());
    let uid = result.uid.expect("UID result");
    assert_eq!(uid.state, RestoreUidState::Applied);
    let mut stored_trailer = new_trailer;
    stored_trailer[..6].copy_from_slice(KEY_A.as_bytes());
    assert_eq!(handle.lock().blocks[&3], stored_trailer);
    assert_eq!(&handle.lock().blocks[&0][0..4], TARGET_UID.as_slice());
    assert!(
        handle
            .lock()
            .auth_log
            .iter()
            .any(|(block, key_type, key)| *block == 0 && *key_type == KeyType::B && *key == KEY_B)
    );
}

#[test]
fn restore_with_uid_refuses_post_restore_access_that_denies_a_uid_write() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x55; BLOCK_SIZE]);
    let mut access = AccessBits::decode(ordinary_access()).expect("access");
    access.groups[0] = AccessCondition::new(true, true, true);
    source.blocks[3].set_read(trailer(access.encode()));

    let result = restore_uid_preflight(&mut service, &store, &source, TARGET_UID);
    assert!(
        matches!(result, Err(NfcError::Unsupported(_))),
        "{result:?}"
    );
    assert!(handle.lock().write_log.is_empty());
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("backup dir")
            .count(),
        0
    );
}

#[test]
fn restore_with_uid_requires_advanced_mode_a_full_restore_and_confirmation() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x66; BLOCK_SIZE]);

    let result = service.classic_write_preflight(
        ClassicWritePreflight::new(source.clone(), &store)
            .include_common_keys(false)
            .uid_target(TARGET_UID),
    );
    assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
    assert!(handle.lock().write_log.is_empty());

    let result = service.classic_write_preflight(
        ClassicWritePreflight::new(source.clone(), &store)
            .include_common_keys(false)
            .uid_target(TARGET_UID)
            .advanced(true)
            .selection(RestoreSelection::Blocks(vec![4])),
    );
    assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
    assert!(handle.lock().write_log.is_empty());

    let plan = restore_uid_preflight(&mut service, &store, &source, TARGET_UID)
        .expect("restore UID preflight");
    let result = service.classic_write_execute(
        ClassicWriteExecution::new(plan.token, source.clone(), &store, &backup)
            .include_common_keys(false)
            .advanced(false)
            .confirmed(true),
    );
    assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
    assert!(handle.lock().write_log.is_empty());

    let plan = restore_uid_preflight(&mut service, &store, &source, TARGET_UID)
        .expect("restore UID preflight");
    let result = service.classic_write_execute(
        ClassicWriteExecution::new(plan.token, source, &store, &backup)
            .include_common_keys(false)
            .advanced(true)
            .confirmed(false),
    );
    assert!(matches!(result, Err(NfcError::InvalidArgument(_))));
    assert!(handle.lock().write_log.is_empty());
}

#[test]
fn restore_with_uid_skips_the_uid_when_a_dump_block_fails() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().fail_write = Some(5);
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x47; BLOCK_SIZE]);
    source.blocks[5].set_read([0x48; BLOCK_SIZE]);

    let plan = restore_uid_preflight(&mut service, &store, &source, TARGET_UID)
        .expect("restore UID preflight");
    let result = restore_uid_execute(&mut service, &store, &backup, plan.token, source, true)
        .expect("restore reports partial failure");
    assert_eq!(result.failures.len(), 1);
    assert!(result.failures[0].contains("block 5"));
    assert_eq!(result.written_blocks, 1);
    let uid = result.uid.expect("UID result");
    assert_eq!(uid.state, RestoreUidState::NotAttempted);
    assert!(
        uid.detail
            .as_deref()
            .is_some_and(|detail| detail.contains("failure")),
        "{:?}",
        uid.detail
    );
    assert_eq!(handle.lock().write_log, vec![4, 5]);
    assert_eq!(handle.lock().card.as_ref().expect("card").uid, UID);
}

#[test]
fn restore_with_uid_distinguishes_a_uid_failure_from_a_dump_failure() {
    let handle = TestHandle::new(TestState::with_card(card()));
    handle.lock().block_zero_writable = false;
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let mut source = read_dump(&mut service, &store);
    source.blocks[4].set_read([0x49; BLOCK_SIZE]);

    let plan = restore_uid_preflight(&mut service, &store, &source, TARGET_UID)
        .expect("restore UID preflight");
    let result = restore_uid_execute(&mut service, &store, &backup, plan.token, source, true)
        .expect("dump restore succeeds");
    assert!(result.failures.is_empty());
    assert_eq!(result.written_blocks, 1);
    assert_eq!(handle.lock().blocks[&4], [0x49; BLOCK_SIZE]);
    let uid = result.uid.expect("UID result");
    assert_eq!(uid.state, RestoreUidState::Failed);
    assert!(
        uid.detail
            .as_deref()
            .is_some_and(|detail| detail.contains("not changed")),
        "{:?}",
        uid.detail
    );
    assert_eq!(handle.lock().write_log, vec![4, 0]);
    assert_eq!(handle.lock().card.as_ref().expect("card").uid, UID);
}

#[test]
fn single_block_trailer_write_preserves_a_masked_key_a() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let original = handle.lock().blocks[&7];
    let mut changed = original;
    changed[..6].fill(0);
    changed[9] ^= 0xFF;

    let result = service
        .classic_write_block(
            ClassicBlockWrite::new(7, changed, &store, &backup)
                .include_common_keys(false)
                .confirm_trailer(true),
        )
        .expect("write the trailer");
    assert!(result.changed);
    let stored = handle.lock().blocks[&7];
    assert_eq!(&stored[..6], KEY_A.as_bytes());
    assert_eq!(stored[9], original[9] ^ 0xFF);
    let read = service
        .classic_read_block(7, &store, false, CancelToken::new())
        .expect("read the trailer")
        .data;
    assert_eq!(read[6..10], stored[6..10]);

    service
        .classic_write_block(
            ClassicBlockWrite::new(7, original, &store, &backup)
                .include_common_keys(false)
                .confirm_trailer(true),
        )
        .expect("restore the trailer");
    assert_eq!(handle.lock().blocks[&7], original);
}

#[test]
fn restore_preserves_a_masked_trailer_key() {
    let handle = TestHandle::new(TestState::with_card(card()));
    let mut service = service(&handle);
    let store = key_store(&[(KeyType::A, KEY_A)]);
    let directory = tempfile::tempdir().expect("temp dir");
    let backup = BackupStore::new(directory.path());
    let original = handle.lock().blocks[&7];
    let mut source = read_dump(&mut service, &store);
    assert!(source.blocks[7].data[..6].iter().all(|byte| *byte == 0));
    source.blocks[7].data[9] ^= 0xFF;

    let result = service
        .classic_restore(ClassicRestore::new(source, &store, &backup).confirmed(true))
        .expect("restore the trailer");
    assert!(result.failures.is_empty());
    assert_eq!(result.trailer_writes, 1);
    let stored = handle.lock().blocks[&7];
    assert_eq!(&stored[..6], KEY_A.as_bytes());
    assert_eq!(stored[9], original[9] ^ 0xFF);
    assert_eq!(stored[10..], original[10..]);
}
