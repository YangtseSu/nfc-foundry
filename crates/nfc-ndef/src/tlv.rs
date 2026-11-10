//! NFC Forum Type 2 TLV framing used by Type 2 and Classic NDEF mappings.

use nfc_core::{NfcError, Result};

use crate::message::NdefMessage;

/// TLV type: NULL.
pub const TAG_NULL: u8 = 0x00;
/// TLV type: Lock Control.
pub const TAG_LOCK_CONTROL: u8 = 0x01;
/// TLV type: Memory Control.
pub const TAG_MEMORY_CONTROL: u8 = 0x02;
/// TLV type: NDEF Message.
pub const TAG_NDEF: u8 = 0x03;
/// TLV type: Proprietary.
pub const TAG_PROPRIETARY: u8 = 0xFD;
/// TLV type: Terminator.
pub const TAG_TERMINATOR: u8 = 0xFE;
/// First length octet that selects the three-octet length form.
pub const LENGTH_LONG_FORM: u8 = 0xFF;
/// Maximum value length representable by the three-octet length form.
pub const MAX_TLV_VALUE_LENGTH: usize = u16::MAX as usize;
/// Lock Control and Memory Control value lengths are fixed by the Type 2
/// specification.
const CONTROL_VALUE_LENGTH: usize = 3;

/// A single Type 2 TLV.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tlv {
    /// NULL padding TLV.
    Null,
    /// Lock Control TLV (0x01); the three value octets are preserved.
    LockControl(Vec<u8>),
    /// Memory Control TLV (0x02); the three value octets are preserved.
    MemoryControl(Vec<u8>),
    /// NDEF Message TLV (0x03); the raw NDEF message is preserved.
    Ndef(Vec<u8>),
    /// Proprietary TLV (0xFD).
    Proprietary(Vec<u8>),
    /// Reserved or unknown TLV (0x04 through 0xFC).
    Unknown { tag: u8, value: Vec<u8> },
    /// Terminator TLV (0xFE); parsing stops here.
    Terminator,
}

impl Tlv {
    /// The TLV type octet.
    pub const fn tag(&self) -> u8 {
        match self {
            Self::Null => TAG_NULL,
            Self::LockControl(_) => TAG_LOCK_CONTROL,
            Self::MemoryControl(_) => TAG_MEMORY_CONTROL,
            Self::Ndef(_) => TAG_NDEF,
            Self::Proprietary(_) => TAG_PROPRIETARY,
            Self::Unknown { tag, .. } => *tag,
            Self::Terminator => TAG_TERMINATOR,
        }
    }

    /// The value octets, or an empty slice for NULL and Terminator.
    pub fn value(&self) -> &[u8] {
        match self {
            Self::Null | Self::Terminator => &[],
            Self::LockControl(value)
            | Self::MemoryControl(value)
            | Self::Ndef(value)
            | Self::Proprietary(value) => value,
            Self::Unknown { value, .. } => value,
        }
    }

    /// Build an NDEF Message TLV from a message.
    pub fn ndef(message: &NdefMessage) -> Result<Self> {
        Ok(Self::Ndef(message.encode()?))
    }

    /// Decode the value of an NDEF Message TLV.
    pub fn as_ndef_message(&self) -> Result<Option<NdefMessage>> {
        match self {
            Self::Ndef(value) => Ok(Some(NdefMessage::decode(value)?)),
            _ => Ok(None),
        }
    }

    /// Validate the value for its TLV type.
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Null | Self::Terminator | Self::Proprietary(_) => Ok(()),
            Self::LockControl(value) | Self::MemoryControl(value) => {
                if value.len() == CONTROL_VALUE_LENGTH {
                    Ok(())
                } else {
                    Err(NfcError::InvalidArgument(format!(
                        "TLV 0x{:02X} value is {} octets, expected {CONTROL_VALUE_LENGTH}",
                        self.tag(),
                        value.len()
                    )))
                }
            }
            Self::Ndef(value) => {
                NdefMessage::decode(value).map_err(|error| {
                    NfcError::InvalidArgument(format!(
                        "TLV 0x03 contains an invalid NDEF message: {error}"
                    ))
                })?;
                Ok(())
            }
            Self::Unknown { tag, .. } => {
                if is_unknown_tag(*tag) {
                    Ok(())
                } else {
                    Err(NfcError::InvalidArgument(format!(
                        "TLV type 0x{tag:02X} is not an unknown/reserved type"
                    )))
                }
            }
        }
    }

    /// Exact encoded size of this TLV.
    pub fn encoded_len(&self) -> Result<usize> {
        if matches!(self, Self::Null | Self::Terminator) {
            return Ok(1);
        }
        let value = self.value();
        if value.len() > MAX_TLV_VALUE_LENGTH {
            return Err(NfcError::InvalidArgument(format!(
                "TLV value is {} octets, maximum is {MAX_TLV_VALUE_LENGTH}",
                value.len()
            )));
        }
        let length_field = if value.len() < usize::from(LENGTH_LONG_FORM) {
            1
        } else {
            3
        };
        Ok(1 + length_field + value.len())
    }

    /// Encode this TLV, validating its value first.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut output = Vec::with_capacity(self.encoded_len()?);
        self.encode_into(&mut output)?;
        Ok(output)
    }

    fn encode_into(&self, output: &mut Vec<u8>) -> Result<()> {
        output.push(self.tag());
        if matches!(self, Self::Null | Self::Terminator) {
            return Ok(());
        }
        let value = self.value();
        if value.len() > MAX_TLV_VALUE_LENGTH {
            return Err(NfcError::InvalidArgument(format!(
                "TLV value is {} octets, maximum is {MAX_TLV_VALUE_LENGTH}",
                value.len()
            )));
        }
        if value.len() < usize::from(LENGTH_LONG_FORM) {
            output.push(value.len() as u8);
        } else {
            output.push(LENGTH_LONG_FORM);
            output.extend_from_slice(&(value.len() as u16).to_be_bytes());
        }
        output.extend_from_slice(value);
        Ok(())
    }

    /// Parse one TLV and return it with the number of consumed octets.
    pub fn parse_one(input: &[u8]) -> Result<(Self, usize)> {
        parse_one_at(input, 0)
    }
}

/// A TLV stream plus any bytes that follow the Terminator.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TlvStream {
    tlvs: Vec<Tlv>,
    padding: Vec<u8>,
}

impl TlvStream {
    /// Build a stream from TLVs and post-terminator padding.
    pub fn new(tlvs: Vec<Tlv>, padding: Vec<u8>) -> Result<Self> {
        let stream = Self { tlvs, padding };
        stream.validate()?;
        Ok(stream)
    }

    /// Parse a TLV area. Parsing stops at the first Terminator; the remaining
    /// octets are kept as padding. Lock Control and Memory Control lengths are
    /// checked here; NDEF message contents are checked by `validate`.
    pub fn parse(input: &[u8]) -> Result<Self> {
        let mut tlvs = Vec::new();
        let mut offset = 0;
        while offset < input.len() {
            let (tlv, consumed) = parse_one_at(&input[offset..], offset)?;
            offset += consumed;
            let terminator = matches!(tlv, Tlv::Terminator);
            tlvs.push(tlv);
            if terminator {
                return Ok(Self {
                    tlvs,
                    padding: input[offset..].to_vec(),
                });
            }
        }
        Ok(Self {
            tlvs,
            padding: Vec::new(),
        })
    }

    /// The TLVs in order, including a Terminator when present.
    pub fn tlvs(&self) -> &[Tlv] {
        &self.tlvs
    }

    /// Octets after the Terminator, preserved verbatim.
    pub fn padding(&self) -> &[u8] {
        &self.padding
    }

    /// Validate stream structure and every TLV.
    pub fn validate(&self) -> Result<()> {
        let mut ndef_count = 0;
        for (index, tlv) in self.tlvs.iter().enumerate() {
            if matches!(tlv, Tlv::Terminator) && index + 1 != self.tlvs.len() {
                return Err(NfcError::InvalidArgument(format!(
                    "TLV {index}: Terminator must be the last TLV"
                )));
            }
            if matches!(tlv, Tlv::Ndef(_)) {
                ndef_count += 1;
            }
            tlv.validate()
                .map_err(|error| NfcError::InvalidArgument(format!("TLV {index}: {error}")))?;
        }
        if ndef_count > 1 {
            return Err(NfcError::InvalidArgument(format!(
                "TLV stream contains {ndef_count} NDEF Message TLVs, at most one is allowed"
            )));
        }
        if !self.padding.is_empty() && !matches!(self.tlvs.last(), Some(Tlv::Terminator)) {
            return Err(NfcError::InvalidArgument(
                "TLV stream has padding but no Terminator".to_owned(),
            ));
        }
        Ok(())
    }

    /// Exact encoded size of the stream.
    pub fn encoded_len(&self) -> Result<usize> {
        let mut total = 0usize;
        for tlv in &self.tlvs {
            total = total
                .checked_add(tlv.encoded_len()?)
                .ok_or_else(|| NfcError::InvalidArgument("TLV stream size overflows".to_owned()))?;
        }
        total
            .checked_add(self.padding.len())
            .ok_or_else(|| NfcError::InvalidArgument("TLV stream size overflows".to_owned()))
    }

    /// Encode the stream after validation.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut output = Vec::with_capacity(self.encoded_len()?);
        for tlv in &self.tlvs {
            tlv.encode_into(&mut output)?;
        }
        output.extend_from_slice(&self.padding);
        Ok(output)
    }

    /// Return the decoded NDEF message, if the stream has an NDEF Message TLV.
    pub fn ndef_message(&self) -> Result<Option<NdefMessage>> {
        let mut message = None;
        for tlv in &self.tlvs {
            if let Tlv::Ndef(value) = tlv {
                if message.is_some() {
                    return Err(NfcError::InvalidArgument(
                        "TLV stream contains more than one NDEF Message TLV".to_owned(),
                    ));
                }
                message = Some(NdefMessage::decode(value)?);
            }
        }
        Ok(message)
    }
}

/// Maximum NDEF message size that fits in `user_bytes` of tag memory together
/// with its NDEF Message TLV header and one Terminator TLV.
pub fn ndef_capacity(user_bytes: usize) -> usize {
    if user_bytes < 3 {
        return 0;
    }
    let short = user_bytes
        .saturating_sub(3)
        .min(usize::from(LENGTH_LONG_FORM) - 1);
    let long = user_bytes.saturating_sub(5).min(MAX_TLV_VALUE_LENGTH);
    short.max(long)
}

fn parse_one_at(input: &[u8], base: usize) -> Result<(Tlv, usize)> {
    let tag = *input.first().ok_or_else(|| {
        NfcError::InvalidArgument(format!("TLV at byte {base}: type octet is missing"))
    })?;
    match tag {
        TAG_NULL => return Ok((Tlv::Null, 1)),
        TAG_TERMINATOR => return Ok((Tlv::Terminator, 1)),
        LENGTH_LONG_FORM => {
            return Err(NfcError::InvalidArgument(format!(
                "TLV at byte {base}: 0xFF is not a valid TLV type"
            )));
        }
        _ => {}
    }

    let first = *input.get(1).ok_or_else(|| {
        NfcError::InvalidArgument(format!(
            "TLV 0x{tag:02X} at byte {base}: length field is truncated"
        ))
    })?;
    let (value_length, header_length): (usize, usize) = if first == LENGTH_LONG_FORM {
        let bytes = input.get(2..4).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "TLV 0x{tag:02X} at byte {base}: three-octet length field is truncated"
            ))
        })?;
        (usize::from(u16::from_be_bytes([bytes[0], bytes[1]])), 4)
    } else {
        (usize::from(first), 2)
    };

    let end = header_length
        .checked_add(value_length)
        .ok_or_else(|| NfcError::InvalidArgument(format!("TLV 0x{tag:02X} length overflows")))?;
    let value = input.get(header_length..end).ok_or_else(|| {
        NfcError::InvalidArgument(format!(
            "TLV 0x{tag:02X} at byte {base}: declares {value_length} value octets but only {} are available",
            input.len().saturating_sub(header_length)
        ))
    })?;

    let tlv = match tag {
        TAG_LOCK_CONTROL => {
            check_control_length(tag, value, base)?;
            Tlv::LockControl(value.to_vec())
        }
        TAG_MEMORY_CONTROL => {
            check_control_length(tag, value, base)?;
            Tlv::MemoryControl(value.to_vec())
        }
        TAG_NDEF => Tlv::Ndef(value.to_vec()),
        TAG_PROPRIETARY => Tlv::Proprietary(value.to_vec()),
        _ => {
            if is_unknown_tag(tag) {
                Tlv::Unknown {
                    tag,
                    value: value.to_vec(),
                }
            } else {
                return Err(NfcError::InvalidArgument(format!(
                    "TLV at byte {base}: type 0x{tag:02X} is not a defined or reserved type"
                )));
            }
        }
    };
    Ok((tlv, end))
}

fn check_control_length(tag: u8, value: &[u8], base: usize) -> Result<()> {
    if value.len() == CONTROL_VALUE_LENGTH {
        Ok(())
    } else {
        Err(NfcError::InvalidArgument(format!(
            "TLV 0x{tag:02X} at byte {base}: value is {} octets, expected {CONTROL_VALUE_LENGTH}",
            value.len()
        )))
    }
}

const fn is_unknown_tag(tag: u8) -> bool {
    tag >= 0x04 && tag <= 0xFC
}
