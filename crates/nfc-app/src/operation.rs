use nfc_core::{BLOCK_SIZE, CancelToken, CardInfo, Key, KeyType, NfcError, Result};
use nfc_transport::Reader;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    Scan,
    CardInfo,
    Read,
    Write,
    KeyScan,
    Recovery,
}

/// Exclusive reader lease for one application operation.
///
/// The guard caches the selected card identity and checks cancellation before
/// card I/O. Writes are read back before they are reported as successful.
/// `check_card` is explicit because reselecting a card resets Classic
/// authentication; workflows call it at boundaries where that is safe, such as
/// before authentication and after a write sequence.
pub struct Operation<'a> {
    kind: OperationKind,
    reader: &'a mut dyn Reader,
    card: CardInfo,
    cancel: CancelToken,
}

impl<'a> Operation<'a> {
    pub(crate) fn new(
        kind: OperationKind,
        reader: &'a mut dyn Reader,
        card: CardInfo,
        cancel: CancelToken,
    ) -> Self {
        Self {
            kind,
            reader,
            card,
            cancel,
        }
    }

    pub fn kind(&self) -> OperationKind {
        self.kind
    }

    pub fn card(&self) -> &CardInfo {
        &self.card
    }

    pub fn cancel(&self) -> &CancelToken {
        &self.cancel
    }

    /// Reselect the card and reject any UID, ATQA, or SAK change.
    pub fn check_card(&mut self) -> Result<()> {
        let current = self.reader.card_info()?;
        if !current.same_identity(&self.card) {
            return Err(NfcError::CardChanged);
        }
        Ok(())
    }

    pub fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()> {
        self.cancel.check()?;
        self.check_card()?;
        self.reader.authenticate(block, key_type, key)
    }

    pub fn read_block(&mut self, block: u8) -> Result<[u8; BLOCK_SIZE]> {
        self.cancel.check()?;
        self.reader.read_block(block)
    }

    pub fn write_block_verified(&mut self, block: u8, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.cancel.check()?;
        self.reader.write_block(block, data)?;
        self.cancel.check()?;
        let read_back = self.reader.read_block(block)?;
        if read_back != data {
            return Err(NfcError::VerificationFailed(format!(
                "block {block} did not match after write"
            )));
        }
        Ok(())
    }
}
