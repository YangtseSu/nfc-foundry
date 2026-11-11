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

    /// Reader access for recovery acquisition and post-write verification.
    /// Only `nfc-app` workflows use this; the GUI and CLI still call service
    /// methods.
    pub(crate) fn reader_mut(&mut self) -> &mut dyn Reader {
        self.reader
    }

    /// Whether the selected reader exposes a protected manufacturer writer.
    pub(crate) fn has_manufacturer_writer(&mut self) -> bool {
        self.reader.manufacturer_writer().is_some()
    }

    /// Writes manufacturer block 0 through the reader's protected writer.
    pub(crate) fn write_manufacturer_block(&mut self, data: [u8; BLOCK_SIZE]) -> Result<()> {
        self.cancel.check()?;
        let writer = self
            .reader
            .manufacturer_writer()
            .ok_or(NfcError::Unsupported(
                "the selected reader has no protected manufacturer writer",
            ))?;
        writer.write_manufacturer_block(data)
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

    /// Writes a sector trailer, preserving masked key bytes and verifying the
    /// fields the card can actually return.
    ///
    /// A trailer read never returns Key A and may mask Key B. When the target
    /// bytes leave such a slot at zero, the authenticated key fills it so a
    /// read-modify-write cannot silently destroy the key. Verification
    /// re-authenticates with the resulting Key A, compares the access bytes
    /// and GPB, and proves Key B when the new access bits permit Key B
    /// trailer writes.
    pub(crate) fn write_trailer_verified(
        &mut self,
        block: u8,
        data: [u8; BLOCK_SIZE],
        authenticated: (KeyType, Key),
        current_access: nfc_classic::AccessBits,
    ) -> Result<()> {
        self.cancel.check()?;
        let mut final_data = data;
        if final_data[..6].iter().all(|byte| *byte == 0) {
            if authenticated.0 != KeyType::A {
                return Err(NfcError::InvalidArgument(format!(
                    "block {block} masks Key A and the sector authenticated with Key B; supply the Key A bytes"
                )));
            }
            final_data[..6].copy_from_slice(authenticated.1.as_bytes());
        }
        if final_data[10..].iter().all(|byte| *byte == 0)
            && !current_access.groups[3].key_b_readable()
        {
            if authenticated.0 != KeyType::B {
                return Err(NfcError::InvalidArgument(format!(
                    "block {block} masks Key B and the sector authenticated with Key A; supply the Key B bytes"
                )));
            }
            final_data[10..].copy_from_slice(authenticated.1.as_bytes());
        }
        let access =
            nfc_classic::AccessBits::decode([final_data[6], final_data[7], final_data[8]])?;
        self.reader.write_block(block, final_data)?;

        let key_a = Key::new(
            final_data[..6]
                .try_into()
                .expect("trailer Key A is six bytes"),
        );
        self.reader.authenticate(block, KeyType::A, key_a)?;
        let read_back = self.reader.read_block(block)?;
        if read_back[6..10] != final_data[6..10] {
            return Err(NfcError::VerificationFailed(format!(
                "block {block} access bytes did not match after the trailer write"
            )));
        }
        if access.groups[3].full_trailer_write_keys().allows_b() {
            let key_b = Key::new(
                final_data[10..]
                    .try_into()
                    .expect("trailer Key B is six bytes"),
            );
            self.reader.authenticate(block, KeyType::B, key_b)?;
        }
        Ok(())
    }

    /// Writes a sector trailer without a read-back. Callers that change keys
    /// must re-authenticate with the new key before verifying the block.
    pub(crate) fn write_block_unverified(
        &mut self,
        block: u8,
        data: [u8; BLOCK_SIZE],
    ) -> Result<()> {
        self.cancel.check()?;
        self.reader.write_block(block, data)
    }
}
