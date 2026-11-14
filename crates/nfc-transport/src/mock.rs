//! Scripted reader and transport used by application and workflow tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nfc_core::{
    BLOCK_SIZE, CardInfo, Key, KeyType, NfcError, PAGE_QUAD_SIZE, PAGE_SIZE, Result,
    TYPE2_VERSION_SIZE,
};

use crate::{
    ApduCapabilities, ApduResponse, DeviceInfo, ManufacturerWriter, Reader, ReaderCapabilities,
    TrailerWriter, Transport,
};

/// Reader entry points that can be scripted with a fault.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MockCall {
    CardInfo,
    Authenticate,
    ReadBlock,
    WriteBlock,
    Transceive,
    ReadPageQuad,
    WritePage,
    PasswordAuth,
    Type2Version,
    Apdu,
    /// An UPDATE BINARY APDU. Kept separate so `WriteIgnored` targets a
    /// state-changing exchange instead of the next SELECT or READ.
    ApduWrite,
    /// The protected manufacturer-block writer. Scripted separately so a
    /// Gen1A path can fail without consuming a fault meant for a normal write.
    ManufacturerWrite,
}

/// Injected behavior for the next matching call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MockFault {
    AuthenticationFailed,
    NoCard,
    Timeout,
    Cancelled,
    /// The write reports success but leaves the stored block unchanged.
    WriteIgnored,
    /// The write is applied but the acknowledgement is lost with an I/O error.
    WriteAppliedError,
}

impl MockFault {
    fn into_error(self) -> NfcError {
        match self {
            Self::AuthenticationFailed => NfcError::AuthenticationFailed,
            Self::NoCard => NfcError::NoCard,
            Self::Timeout => NfcError::Timeout("mock reader timed out".to_owned()),
            Self::Cancelled => NfcError::Cancelled,
            Self::WriteIgnored | Self::WriteAppliedError => {
                NfcError::Io("mock write acknowledgement".to_owned())
            }
        }
    }
}

/// A scripted NFC Forum Type 4 tag. The simulator answers the ISO 7816-4
/// SELECT, READ BINARY, and UPDATE BINARY commands used by the Type 4 adapter,
/// so workflow tests exercise the real APDU sequence instead of a canned list.
///
/// The NDEF file is stored as complete file bytes, including its two-byte NLEN
/// prefix. The default file IDs are `E103` (CC) and `E104` (NDEF).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MockType4Tag {
    cc: Vec<u8>,
    ndef: Vec<u8>,
    cc_file_id: u16,
    ndef_file_id: u16,
    write_access: u8,
    selected_application: bool,
    selected_file: Option<u16>,
    commands: Vec<Vec<u8>>,
}

impl MockType4Tag {
    pub fn new(cc: impl Into<Vec<u8>>, ndef: impl Into<Vec<u8>>) -> Self {
        Self {
            cc: cc.into(),
            ndef: ndef.into(),
            cc_file_id: 0xE103,
            ndef_file_id: 0xE104,
            write_access: 0x00,
            selected_application: false,
            selected_file: None,
            commands: Vec::new(),
        }
    }

    /// A factory NTAG 424 DNA: 23-byte CC, 256-byte NDEF file, free access.
    pub fn ntag424() -> Self {
        Self::new(
            vec![
                0x00, 0x17, 0x20, 0x01, 0x00, 0x00, 0xFF, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
                0x00, 0x05, 0x06, 0xE1, 0x05, 0x00, 0x80, 0x82, 0x83,
            ],
            vec![0u8; 256],
        )
    }

    /// A DESFire-style 15-byte CC and a 256-byte NDEF file.
    pub fn desfire_256() -> Self {
        Self::new(
            vec![
                0x00, 0x0F, 0x20, 0x00, 0x3A, 0x00, 0x34, 0x04, 0x06, 0xE1, 0x04, 0x01, 0x00, 0x00,
                0x00,
            ],
            vec![0u8; 256],
        )
    }

    pub fn with_write_access(mut self, access: u8) -> Self {
        self.write_access = access;
        self
    }

    pub fn with_ndef_file_id(mut self, file_id: u16) -> Self {
        self.ndef_file_id = file_id;
        self
    }

    pub fn cc(&self) -> &[u8] {
        &self.cc
    }

    pub fn ndef(&self) -> &[u8] {
        &self.ndef
    }

    pub fn commands(&self) -> &[Vec<u8>] {
        &self.commands
    }

    fn handle(&mut self, apdu: &[u8], ignore_write: bool) -> ApduResponse {
        self.commands.push(apdu.to_vec());
        if apdu.len() < 4 {
            return sw(0x67, 0x00);
        }
        match (apdu[0], apdu[1]) {
            (0x00, 0xA4) => self.select(apdu),
            (0x00, 0xB0) => self.read_binary(apdu),
            (0x00, 0xD6) => self.update_binary(apdu, ignore_write),
            _ => sw(0x6D, 0x00),
        }
    }

    fn select(&mut self, apdu: &[u8]) -> ApduResponse {
        let (p1, p2) = (apdu[2], apdu[3]);
        if p1 == 0x04 {
            if p2 != 0x00 || apdu.len() < 6 {
                return sw(0x6A, 0x86);
            }
            let length = usize::from(apdu[4]);
            if apdu.len() < 5 + length {
                return sw(0x67, 0x00);
            }
            if apdu[5..5 + length] == [0xD2, 0x76, 0x00, 0x00, 0x85, 0x01, 0x01] {
                self.selected_application = true;
                self.selected_file = None;
                sw(0x90, 0x00)
            } else {
                sw(0x6A, 0x82)
            }
        } else if p1 == 0x00 {
            if (p2 != 0x00 && p2 != 0x0C) || apdu.len() < 7 || apdu[4] != 0x02 {
                return sw(0x6A, 0x86);
            }
            let file_id = u16::from_be_bytes([apdu[5], apdu[6]]);
            if file_id == self.cc_file_id || file_id == self.ndef_file_id {
                self.selected_file = Some(file_id);
                sw(0x90, 0x00)
            } else {
                sw(0x6A, 0x82)
            }
        } else {
            sw(0x6A, 0x86)
        }
    }

    fn read_binary(&mut self, apdu: &[u8]) -> ApduResponse {
        let Some(file_id) = self.selected_file else {
            return sw(0x69, 0x82);
        };
        if !self.selected_application && file_id != self.cc_file_id {
            return sw(0x69, 0x82);
        }
        if apdu.len() < 5 {
            return sw(0x67, 0x00);
        }
        let offset = usize::from(u16::from_be_bytes([apdu[2], apdu[3]]));
        let requested = match apdu[4] {
            0x00 => 256,
            value => usize::from(value),
        };
        let file = if file_id == self.cc_file_id {
            &self.cc
        } else {
            &self.ndef
        };
        if offset >= file.len() {
            return sw(0x6B, 0x00);
        }
        let end = (offset + requested).min(file.len());
        let data = file[offset..end].to_vec();
        if data.len() < requested {
            ApduResponse::new(data, [0x62, 0x82])
        } else {
            ApduResponse::new(data, [0x90, 0x00])
        }
    }

    fn update_binary(&mut self, apdu: &[u8], ignore_write: bool) -> ApduResponse {
        let Some(file_id) = self.selected_file else {
            return sw(0x69, 0x82);
        };
        if file_id != self.ndef_file_id {
            return sw(0x69, 0x82);
        }
        if self.write_access != 0x00 {
            return sw(0x69, 0x82);
        }
        if apdu.len() < 5 {
            return sw(0x67, 0x00);
        }
        let length = usize::from(apdu[4]);
        if apdu.len() < 5 + length {
            return sw(0x67, 0x00);
        }
        let offset = usize::from(u16::from_be_bytes([apdu[2], apdu[3]]));
        if offset + length > self.ndef.len() {
            return sw(0x6B, 0x00);
        }
        if !ignore_write {
            self.ndef[offset..offset + length].copy_from_slice(&apdu[5..5 + length]);
        }
        sw(0x90, 0x00)
    }
}

fn sw(sw1: u8, sw2: u8) -> ApduResponse {
    ApduResponse::new(Vec::new(), [sw1, sw2])
}

type WriteHook = Arc<dyn Fn() + Send + Sync>;

/// Test hook that runs after a write is applied. It is not `Debug`, so the
/// state carries a wrapper with a stable placeholder.
#[derive(Clone)]
struct MockWriteHook(WriteHook);

impl std::fmt::Debug for MockWriteHook {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MockWriteHook(..)")
    }
}

#[derive(Debug)]
struct MockState {
    card: Option<CardInfo>,
    blocks: HashMap<u8, [u8; BLOCK_SIZE]>,
    pages: HashMap<u8, [u8; PAGE_SIZE]>,
    type2_version: Option<[u8; TYPE2_VERSION_SIZE]>,
    password: Option<([u8; PAGE_SIZE], [u8; 2])>,
    authenticated: bool,
    password_auths: usize,
    type4: Option<MockType4Tag>,
    raw_mode: bool,
    open: bool,
    open_count: usize,
    close_count: usize,
    faults: Vec<(MockCall, MockFault)>,
    write_hook: Option<MockWriteHook>,
    capabilities: ReaderCapabilities,
    /// An ordinary write to block 0 is accepted, as on a CUID/Gen2 card.
    block_zero_writable: bool,
    /// Type 2 pages 0/1 update the selected UID, as on a UID-changeable clone.
    type2_uid_writable: bool,
    /// Selection succeeds but every MIFARE authentication times out, as on a
    /// phone-emulated UID-only card.
    mifare_unsupported: bool,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            card: None,
            blocks: HashMap::new(),
            pages: HashMap::new(),
            type2_version: None,
            password: None,
            authenticated: false,
            password_auths: 0,
            type4: None,
            raw_mode: false,
            open: false,
            open_count: 0,
            close_count: 0,
            faults: Vec::new(),
            write_hook: None,
            capabilities: ReaderCapabilities {
                classic_read: true,
                classic_write: true,
                raw_bytes: true,
                raw_bits: true,
                manufacturer_write: true,
                type2_read: true,
                type2_write: true,
                recovery_parity: false,
            },
            block_zero_writable: true,
            type2_uid_writable: false,
            mifare_unsupported: false,
        }
    }
}

impl MockState {
    fn take_fault(&mut self, call: MockCall) -> Option<MockFault> {
        let index = self.faults.iter().position(|(target, _)| *target == call)?;
        Some(self.faults.remove(index).1)
    }

    fn present_card(&self) -> Result<&CardInfo> {
        self.card.as_ref().ok_or(NfcError::NoCard)
    }

    fn check_call(&mut self, call: MockCall) -> Result<()> {
        if let Some(fault) = self.take_fault(call) {
            return Err(fault.into_error());
        }
        self.present_card().map(|_| ())
    }

    fn run_write_hook(&self) {
        if let Some(hook) = &self.write_hook {
            (hook.0)();
        }
    }

    /// NTAG21x configuration pages derived from the scripted capability
    /// container size: (CFG0, CFG1).
    fn config_pages(&self) -> Option<(u8, u8)> {
        match self.pages.get(&3)?.get(2).copied()? {
            0x12 => Some((41, 42)),
            0x3E => Some((131, 132)),
            0x6D => Some((227, 228)),
            _ => None,
        }
    }

    /// Whether `page` needs PWD_AUTH before the requested access.
    fn requires_auth(&self, page: u8, for_read: bool) -> bool {
        let Some((cfg0, cfg1)) = self.config_pages() else {
            return false;
        };
        let Some(auth0) = self.pages.get(&cfg0).map(|data| data[3]) else {
            return false;
        };
        if auth0 == 0xFF || page < auth0 {
            return false;
        }
        !for_read || self.pages.get(&cfg1).is_none_or(|data| data[0] & 0x80 != 0)
    }

    fn require_access(&self, page: u8, for_read: bool) -> Result<()> {
        if self.requires_auth(page, for_read) && !self.authenticated {
            return Err(NfcError::AuthenticationFailed);
        }
        Ok(())
    }
}

/// Shared handle for scripting a mock reader while a service owns it.
#[derive(Clone, Debug)]
pub struct MockHandle(Arc<Mutex<MockState>>);

impl Default for MockHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl MockHandle {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(MockState::default())))
    }

    fn lock(&self) -> MutexGuard<'_, MockState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn set_card(&self, card: Option<CardInfo>) {
        let mut state = self.lock();
        state.card = card;
        state.authenticated = false;
    }

    pub fn set_block(&self, block: u8, data: [u8; BLOCK_SIZE]) {
        self.lock().blocks.insert(block, data);
    }

    pub fn block(&self, block: u8) -> Option<[u8; BLOCK_SIZE]> {
        self.lock().blocks.get(&block).copied()
    }

    /// Store one Type 2 page.
    pub fn set_page(&self, page: u8, data: [u8; PAGE_SIZE]) {
        self.lock().pages.insert(page, data);
    }

    /// Make Type 2 page 0/1 writes update the selected card UID.
    pub fn set_type2_uid_writable(&self, enabled: bool) {
        self.lock().type2_uid_writable = enabled;
    }

    /// Remove every stored Type 2 page.
    pub fn clear_pages(&self) {
        let mut state = self.lock();
        state.pages.clear();
        state.authenticated = false;
    }

    pub fn page(&self, page: u8) -> Option<[u8; PAGE_SIZE]> {
        self.lock().pages.get(&page).copied()
    }

    /// Configure the GET_VERSION response. `None` makes the mock report
    /// `Unsupported`, so callers exercise the capability-container fallback.
    pub fn set_type2_version(&self, version: Option<[u8; TYPE2_VERSION_SIZE]>) {
        self.lock().type2_version = version;
    }

    /// Configure the PWD_AUTH secret and expected PACK.
    pub fn set_password(&self, password: [u8; PAGE_SIZE], pack: [u8; 2]) {
        let mut state = self.lock();
        state.password = Some((password, pack));
        state.authenticated = false;
    }

    /// Number of PWD_AUTH calls the scripted tag has answered.
    pub fn password_auths(&self) -> usize {
        self.lock().password_auths
    }

    /// Install or replace the scripted Type 4 tag.
    pub fn set_type4_tag(&self, tag: MockType4Tag) {
        self.lock().type4 = Some(tag);
    }

    pub fn type4_tag(&self) -> Option<MockType4Tag> {
        self.lock().type4.clone()
    }

    pub fn type4_ndef(&self) -> Option<Vec<u8>> {
        self.lock().type4.as_ref().map(|tag| tag.ndef.clone())
    }

    pub fn type4_cc(&self) -> Option<Vec<u8>> {
        self.lock().type4.as_ref().map(|tag| tag.cc.clone())
    }

    /// Fail the next matching call. `WriteIgnored` applies to
    /// `MockCall::WriteBlock`, `MockCall::WritePage`, and an UPDATE BINARY
    /// `MockCall::ApduWrite`.
    pub fn fail_next(&self, call: MockCall, fault: MockFault) {
        self.lock().faults.push((call, fault));
    }

    /// Run `hook` after a block or page write is applied and before the mock
    /// returns. Tests use it to cancel a token exactly inside a write. The
    /// hook runs under the mock's state lock and must not call back into this
    /// handle.
    pub fn on_write_applied(&self, hook: impl Fn() + Send + Sync + 'static) {
        self.lock().write_hook = Some(MockWriteHook(Arc::new(hook)));
    }

    /// Replace the advertised capability profile.
    pub fn set_capabilities(&self, capabilities: ReaderCapabilities) {
        self.lock().capabilities = capabilities;
    }

    pub fn capabilities(&self) -> ReaderCapabilities {
        self.lock().capabilities
    }

    /// Advertise the protected Gen1A manufacturer writer.
    pub fn set_manufacturer_write(&self, enabled: bool) {
        self.lock().capabilities.manufacturer_write = enabled;
    }

    /// Accept or refuse an ordinary write to manufacturer block 0.
    pub fn set_block_zero_writable(&self, enabled: bool) {
        self.lock().block_zero_writable = enabled;
    }

    /// Make every MIFARE authentication time out, as on a phone-emulated
    /// UID-only card. Selection and card identity stay available.
    pub fn set_mifare_unsupported(&self, enabled: bool) {
        self.lock().mifare_unsupported = enabled;
    }

    pub fn manufacturer_write(&self) -> bool {
        self.lock().capabilities.manufacturer_write
    }

    pub fn block_zero_writable(&self) -> bool {
        self.lock().block_zero_writable
    }

    pub fn is_raw_mode(&self) -> bool {
        self.lock().raw_mode
    }

    pub fn is_open(&self) -> bool {
        self.lock().open
    }

    pub fn open_count(&self) -> usize {
        self.lock().open_count
    }

    pub fn close_count(&self) -> usize {
        self.lock().close_count
    }
}

#[derive(Debug)]
pub struct MockReader {
    info: DeviceInfo,
    state: MockHandle,
}

impl MockReader {
    pub fn new(info: DeviceInfo, card: Option<CardInfo>) -> Self {
        let state = MockHandle::new();
        state.set_card(card);
        Self { info, state }
    }

    fn from_handle(info: DeviceInfo, state: MockHandle) -> Self {
        Self { info, state }
    }

    pub fn set_card(&mut self, card: Option<CardInfo>) {
        self.state.set_card(card);
    }

    pub fn set_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) {
        self.state.set_block(block, data);
    }

    pub fn set_page(&mut self, page: u8, data: [u8; PAGE_SIZE]) {
        self.state.set_page(page, data);
    }

    pub fn page(&self, page: u8) -> Option<[u8; PAGE_SIZE]> {
        self.state.page(page)
    }

    pub fn set_type2_version(&mut self, version: Option<[u8; TYPE2_VERSION_SIZE]>) {
        self.state.set_type2_version(version);
    }

    pub fn set_type2_uid_writable(&mut self, enabled: bool) {
        self.state.set_type2_uid_writable(enabled);
    }

    pub fn set_password(&mut self, password: [u8; PAGE_SIZE], pack: [u8; 2]) {
        self.state.set_password(password, pack);
    }

    pub fn password_auths(&self) -> usize {
        self.state.password_auths()
    }

    pub fn set_type4_tag(&mut self, tag: MockType4Tag) {
        self.state.set_type4_tag(tag);
    }

    pub fn type4_tag(&self) -> Option<MockType4Tag> {
        self.state.type4_tag()
    }

    pub fn fail_next(&mut self, call: MockCall, fault: MockFault) {
        self.state.fail_next(call, fault);
    }

    pub fn on_write_applied(&self, hook: impl Fn() + Send + Sync + 'static) {
        self.state.on_write_applied(hook);
    }

    pub fn set_capabilities(&mut self, capabilities: ReaderCapabilities) {
        self.state.set_capabilities(capabilities);
    }

    pub fn set_manufacturer_write(&mut self, enabled: bool) {
        self.state.set_manufacturer_write(enabled);
    }

    pub fn is_raw_mode(&self) -> bool {
        self.state.is_raw_mode()
    }
}

impl Reader for MockReader {
    fn device(&self) -> &DeviceInfo {
        &self.info
    }

    fn firmware(&self) -> Option<String> {
        Some("mock reader firmware".to_owned())
    }

    fn capabilities(&self) -> ReaderCapabilities {
        self.state.lock().capabilities
    }

    fn open(&mut self, conn_string: &str) -> Result<()> {
        self.info.conn_string = conn_string.to_owned();
        let mut state = self.state.lock();
        state.open = true;
        state.open_count += 1;
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        let mut state = self.state.lock();
        state.open = false;
        state.close_count += 1;
        Ok(())
    }

    fn card_info(&mut self) -> Result<CardInfo> {
        let mut state = self.state.lock();
        state.check_call(MockCall::CardInfo)?;
        state.authenticated = false;
        state.present_card().cloned()
    }

    fn authenticate(&mut self, _block: u8, _key_type: KeyType, _key: Key) -> Result<()> {
        let mut state = self.state.lock();
        if state.mifare_unsupported {
            return Err(NfcError::Timeout(
                "the card did not answer MIFARE authentication".to_owned(),
            ));
        }
        state.check_call(MockCall::Authenticate)
    }

    fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        let mut state = self.state.lock();
        if state.mifare_unsupported {
            return Err(NfcError::Timeout(
                "the card did not answer a MIFARE read".to_owned(),
            ));
        }
        state.check_call(MockCall::ReadBlock)?;
        let data = state.blocks.get(&block).copied().ok_or_else(|| {
            NfcError::InvalidArgument(format!("mock block {block} is not loaded"))
        })?;
        Ok(if block % 4 == 3 {
            trailer_as_read(data)
        } else {
            data
        })
    }

    fn write_block(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        let mut state = self.state.lock();
        if let Some(fault) = state.take_fault(MockCall::WriteBlock) {
            match fault {
                MockFault::WriteIgnored => return Ok(()),
                MockFault::WriteAppliedError => {
                    state.present_card()?;
                    apply_block_write(&mut state, block, data, false)?;
                    return Err(fault.into_error());
                }
                other => return Err(other.into_error()),
            }
        }
        state.present_card()?;
        apply_block_write(&mut state, block, data, false)?;
        state.run_write_hook();
        Ok(())
    }

    fn read_page_quad(&mut self, start_page: u8) -> Result<[u8; PAGE_QUAD_SIZE]> {
        let mut state = self.state.lock();
        state.check_call(MockCall::ReadPageQuad)?;
        let mut output = [0u8; PAGE_QUAD_SIZE];
        for offset in 0..4u8 {
            let page = start_page.wrapping_add(offset);
            let data = state.pages.get(&page).copied().ok_or_else(|| {
                NfcError::InvalidArgument(format!("mock Type 2 page {page} is not loaded"))
            })?;
            state.require_access(page, true)?;
            let index = usize::from(offset) * PAGE_SIZE;
            output[index..index + PAGE_SIZE].copy_from_slice(&data);
        }
        Ok(output)
    }

    fn write_page(&mut self, page: u8, data: [u8; PAGE_SIZE]) -> Result<()> {
        let mut state = self.state.lock();
        if let Some(fault) = state.take_fault(MockCall::WritePage) {
            if matches!(fault, MockFault::WriteIgnored) {
                return Ok(());
            }
            return Err(fault.into_error());
        }
        state.present_card()?;
        state.require_access(page, false)?;
        if let Some((_, cfg1)) = state.config_pages() {
            if page == cfg1.saturating_add(1) {
                let pack = state.password.map(|(_, pack)| pack).unwrap_or([0; 2]);
                state.password = Some((data, pack));
            } else if page == cfg1.saturating_add(2) {
                let password = state
                    .password
                    .map(|(password, _)| password)
                    .unwrap_or([0; PAGE_SIZE]);
                state.password = Some((password, [data[0], data[1]]));
            }
        }
        state.pages.insert(page, data);
        if state.type2_uid_writable && page <= 2 {
            let page0 = state.pages.get(&0).copied();
            let page1 = state.pages.get(&1).copied();
            if let (Some(page0), Some(page1), Some(card)) = (page0, page1, state.card.as_mut()) {
                let mut uid = Vec::with_capacity(7);
                uid.extend_from_slice(&page0[..3]);
                uid.extend_from_slice(&page1);
                card.uid = uid;
            }
        }
        state.run_write_hook();
        Ok(())
    }

    fn type2_version(&mut self) -> Result<[u8; TYPE2_VERSION_SIZE]> {
        let mut state = self.state.lock();
        state.check_call(MockCall::Type2Version)?;
        state.type2_version.ok_or(NfcError::Unsupported(
            "the mock reader has no Type 2 GET_VERSION",
        ))
    }

    fn password_auth(&mut self, password: [u8; PAGE_SIZE]) -> Result<[u8; 2]> {
        let mut state = self.state.lock();
        state.check_call(MockCall::PasswordAuth)?;
        state.password_auths += 1;
        match state.password {
            Some((expected, pack)) if expected == password => {
                state.authenticated = true;
                Ok(pack)
            }
            _ => Err(NfcError::AuthenticationFailed),
        }
    }

    fn apdu_capabilities(&self) -> ApduCapabilities {
        ApduCapabilities {
            supported: self.state.lock().type4.is_some(),
            max_command: 247,
        }
    }

    fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse> {
        let mut state = self.state.lock();
        state.present_card()?;
        let call = if apdu.get(1) == Some(&0xD6) {
            MockCall::ApduWrite
        } else {
            MockCall::Apdu
        };
        let mut ignore_write = false;
        if let Some(fault) = state.take_fault(call) {
            match fault {
                MockFault::WriteIgnored => ignore_write = true,
                other => return Err(other.into_error()),
            }
        }
        let tag = state.type4.as_mut().ok_or(NfcError::Unsupported(
            "the mock reader has no scripted Type 4 tag",
        ))?;
        Ok(tag.handle(apdu, ignore_write))
    }

    fn transceive_bytes(&mut self, _tx: &[u8]) -> Result<Vec<u8>> {
        self.state.lock().check_call(MockCall::Transceive)?;
        Ok(Vec::new())
    }

    fn transceive_bits(&mut self, _tx: &[u8], _tx_bits: usize) -> Result<(Vec<u8>, usize)> {
        self.state.lock().check_call(MockCall::Transceive)?;
        Ok((Vec::new(), 0))
    }

    fn set_raw_mode(&mut self, enabled: bool) -> Result<()> {
        self.state.lock().raw_mode = enabled;
        Ok(())
    }

    fn manufacturer_writer(&mut self) -> Option<&mut dyn ManufacturerWriter> {
        let available = self.state.lock().capabilities.manufacturer_write;
        if available { Some(self) } else { None }
    }
}

/// Applies one block write. A manufacturer-block write also updates the
/// scripted card identity, so a reselect observes the same UID the card would
/// report.
fn apply_block_write(
    state: &mut MockState,
    block: u8,
    data: [u8; BLOCK_SIZE],
    protected: bool,
) -> Result<()> {
    if block == 0 {
        if !protected && !state.block_zero_writable {
            return Err(NfcError::Io(
                "mock card refused the manufacturer block write".to_owned(),
            ));
        }
        if let Some(card) = state.card.as_mut() {
            card.uid = data[0..4].to_vec();
        }
    }
    state.blocks.insert(block, data);
    Ok(())
}

impl TrailerWriter for MockReader {
    fn write_sector_trailer(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        Reader::write_block(self, block, data)
    }
}

impl ManufacturerWriter for MockReader {
    fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        let mut state = self.state.lock();
        if !state.capabilities.manufacturer_write {
            return Err(NfcError::Unsupported(
                "the mock reader has no Gen1A manufacturer writer",
            ));
        }
        if let Some(fault) = state.take_fault(MockCall::ManufacturerWrite) {
            match fault {
                MockFault::WriteIgnored => return Ok(()),
                MockFault::WriteAppliedError => {
                    state.present_card()?;
                    apply_block_write(&mut state, 0, data, true)?;
                    return Err(fault.into_error());
                }
                other => return Err(other.into_error()),
            }
        }
        state.present_card()?;
        apply_block_write(&mut state, 0, data, true)
    }
}

/// Transport that creates `MockReader` instances sharing scripted state.
#[derive(Clone, Debug, Default)]
pub struct MockTransport {
    devices: Arc<Mutex<Vec<(DeviceInfo, MockHandle)>>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Vec<(DeviceInfo, MockHandle)>> {
        self.devices.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Add or replace a scripted reader and return its script handle.
    pub fn add(&self, info: DeviceInfo, card: Option<CardInfo>) -> MockHandle {
        let handle = MockHandle::new();
        handle.set_card(card);
        let mut devices = self.lock();
        if let Some((existing, existing_handle)) = devices
            .iter_mut()
            .find(|(existing, _)| existing.conn_string == info.conn_string)
        {
            *existing = info;
            *existing_handle = handle.clone();
        } else {
            devices.push((info, handle.clone()));
        }
        handle
    }

    pub fn handle(&self, conn_string: &str) -> Option<MockHandle> {
        self.lock()
            .iter()
            .find(|(info, _)| info.conn_string == conn_string)
            .map(|(_, handle)| handle.clone())
    }
}

impl Transport for MockTransport {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>> {
        Ok(self.lock().iter().map(|(info, _)| info.clone()).collect())
    }

    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>> {
        let (info, handle) = self
            .lock()
            .iter()
            .find(|(info, _)| info.conn_string == conn_string)
            .map(|(info, handle)| (info.clone(), handle.clone()))
            .ok_or_else(|| {
                NfcError::InvalidArgument(format!("mock reader {conn_string} is not available"))
            })?;
        Ok(Box::new(MockReader::from_handle(info, handle)))
    }
}

/// The bytes a MIFARE trailer read returns: Key A is always masked, and Key B
/// is masked when the trailer access bits do not expose it.
fn trailer_as_read(mut data: [u8; BLOCK_SIZE]) -> [u8; BLOCK_SIZE] {
    data[..6].fill(0);
    let c1 = data[7] & 0x80 != 0;
    let c2 = data[8] & 0x08 != 0;
    let c3 = data[8] & 0x80 != 0;
    let code = (u8::from(c1) << 2) | (u8::from(c2) << 1) | u8::from(c3);
    if code > 2 {
        data[10..].fill(0);
    }
    data
}

#[cfg(test)]
mod tests {
    use nfc_core::CardType;

    use super::*;

    fn card(uid: u8) -> CardInfo {
        CardInfo::new(vec![uid; 4], [0x00, 0x04], 0x08, CardType::MifareClassic1K)
    }

    #[test]
    fn scripted_reader_reports_card_changes_and_faults() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        assert_eq!(reader.card_info().expect("card").uid, vec![1; 4]);

        reader.set_card(None);
        assert!(matches!(reader.card_info(), Err(NfcError::NoCard)));

        reader.set_card(Some(card(2)));
        reader.fail_next(MockCall::CardInfo, MockFault::Timeout);
        assert!(matches!(reader.card_info(), Err(NfcError::Timeout(_))));
        assert_eq!(reader.card_info().expect("card").uid, vec![2, 2, 2, 2]);
    }

    #[test]
    fn mock_reader_round_trips_blocks_and_raw_mode() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        let data = [0xAA; BLOCK_SIZE];
        reader.set_block(1, data);
        assert_eq!(reader.read_block(1).expect("block"), data);
        reader.write_sector_trailer(2, data).expect("trailer");
        assert_eq!(reader.read_block(2).expect("trailer block"), data);
        reader.write_manufacturer_block(data).expect("manufacturer");
        assert_eq!(reader.read_block(0).expect("manufacturer block"), data);
        assert!(!reader.is_raw_mode());
        reader.set_raw_mode(true).expect("raw mode");
        assert!(reader.is_raw_mode());
    }

    #[test]
    fn ignored_write_keeps_the_previous_block() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        let original = [0x11; BLOCK_SIZE];
        reader.set_block(1, original);
        reader.fail_next(MockCall::WriteBlock, MockFault::WriteIgnored);
        reader.write_block(1, [0xAA; BLOCK_SIZE]).expect("write");
        assert_eq!(reader.read_block(1).expect("block"), original);
    }

    #[test]
    fn capabilities_gate_the_manufacturer_writer() {
        let mut reader = MockReader::new(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        assert!(reader.capabilities().manufacturer_write);
        let mut capabilities = reader.capabilities();
        capabilities.manufacturer_write = false;
        capabilities.recovery_parity = false;
        reader.set_capabilities(capabilities);
        assert_eq!(reader.capabilities(), capabilities);
        assert!(reader.manufacturer_writer().is_none());
        assert!(matches!(
            reader.write_manufacturer_block([0xAA; BLOCK_SIZE]),
            Err(NfcError::Unsupported(_))
        ));
    }

    #[test]
    fn mock_transport_shares_state_with_opened_readers() {
        let transport = MockTransport::new();
        let handle = transport.add(DeviceInfo::new("mock", "mock:0"), Some(card(1)));
        let mut reader = transport.open("mock:0").expect("open");
        assert_eq!(
            transport.enumerate().expect("enumerate"),
            vec![DeviceInfo::new("mock", "mock:0")]
        );

        handle.set_card(None);
        assert!(matches!(reader.card_info(), Err(NfcError::NoCard)));
        assert!(matches!(
            transport.open("mock:1"),
            Err(NfcError::InvalidArgument(_))
        ));
    }
}
