//! Scripted Classic workflow tests. No hardware is touched.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nfc_app::classic::{
    ClassicBlockWrite, ClassicKeyscan, ClassicRead, ClassicRestore, ClassicWriteExecution,
    ClassicWritePreflight, KeyTypes, RestoreBlockState, RestoreSelection, WorkbenchMergeReport,
};
use nfc_app::{BackupStore, KeyEntry, KeyKind, KeySource, KeyStore, Service};
use nfc_classic::{AccessBits, AccessCondition, Dump, DumpFormat, Layout, bcc_for_uid, exchange};
use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, CardType, Key, KeyType, NfcError, Result};
use nfc_transport::{DeviceInfo, Reader, Transport};

const UID: [u8; 4] = [0x04, 0xA2, 0x24, 0x8A];
const KEY_A: Key = Key::new([0xFF; 6]);
const KEY_B: Key = Key::new([0xA0; 6]);
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
    write_log: Vec<u8>,
    auth_log: Vec<(u8, KeyType, Key)>,
    cancel_after_auths: Option<(usize, CancelToken)>,
}

impl TestState {
    fn with_card(card: CardInfo) -> Self {
        Self {
            card: Some(card),
            blocks: default_image(),
            ..Self::default()
        }
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
        state.auth_log.push((block, key_type, key));
        let sector = block / 4;
        if let Some(rules) = state.sector_rules.get(&sector)
            && !rules.contains(&(key_type, key))
        {
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
        state
            .blocks
            .get(&block)
            .copied()
            .ok_or_else(|| NfcError::Io(format!("test block {block} is not loaded")))
    }

    fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        let mut state = self.handle.lock();
        if state.card.is_none() {
            return Err(NfcError::NoCard);
        }
        state.write_log.push(block);
        if state.fail_write == Some(block) {
            return Err(NfcError::Io(format!("test write of block {block} failed")));
        }
        if state.ignore_write == Some(block) {
            return Ok(());
        }
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
    assert_eq!(backup_dump.blocks[7].data, original_trailer);
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
