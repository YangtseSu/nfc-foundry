//! Bluetooth LE out-of-band record template.

use nfc_core::{NfcError, Result};

use crate::record::{NdefRecord, Tnf};

const MEDIA_TYPE: &[u8] = b"application/vnd.bluetooth.le.oob";

const AD_FLAGS: u8 = 0x01;
const AD_SHORT_NAME: u8 = 0x08;
const AD_COMPLETE_NAME: u8 = 0x09;
const AD_APPEARANCE: u8 = 0x19;
const AD_LE_ADDRESS: u8 = 0x1B;
const AD_LE_ROLE: u8 = 0x1C;

const MAX_NAME_OCTETS: usize = 248;

/// Address type carried in the LE Bluetooth Device Address AD structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeAddressType {
    /// Public device address.
    Public,
    /// Random device address.
    Random,
}

impl LeAddressType {
    const fn wire_code(self) -> u8 {
        match self {
            Self::Public => 0,
            Self::Random => 1,
        }
    }

    fn from_wire(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Public),
            1 => Ok(Self::Random),
            other => Err(NfcError::InvalidArgument(format!(
                "Bluetooth LE address type {other} is not defined"
            ))),
        }
    }
}

/// LE role capabilities from the LE Role AD structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeRole {
    /// Peripheral only.
    PeripheralOnly,
    /// Central only.
    CentralOnly,
    /// Peripheral preferred.
    PeripheralPreferred,
    /// Central preferred.
    CentralPreferred,
}

impl LeRole {
    const fn wire_code(self) -> u8 {
        match self {
            Self::PeripheralOnly => 0,
            Self::CentralOnly => 1,
            Self::PeripheralPreferred => 2,
            Self::CentralPreferred => 3,
        }
    }

    fn from_wire(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::PeripheralOnly),
            1 => Ok(Self::CentralOnly),
            2 => Ok(Self::PeripheralPreferred),
            3 => Ok(Self::CentralPreferred),
            other => Err(NfcError::InvalidArgument(format!(
                "Bluetooth LE role value {other} is not defined"
            ))),
        }
    }
}

/// A Bluetooth LE out-of-band pairing record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BluetoothLeRecord {
    /// LE Bluetooth Device Address in display order and its address type.
    pub address: Option<([u8; 6], LeAddressType)>,
    /// LE Role.
    pub role: Option<LeRole>,
    /// Complete local name.
    pub name: Option<String>,
    /// Appearance value.
    pub appearance: Option<u16>,
    /// Advertising flags.
    pub flags: Option<u8>,
    /// Unknown advertising-data structures, preserved in original order.
    pub extra: Vec<(u8, Vec<u8>)>,
}

impl BluetoothLeRecord {
    /// Validate name length and unknown AD structures.
    pub fn validate(&self) -> Result<()> {
        if let Some(name) = &self.name
            && name.len() > MAX_NAME_OCTETS
        {
            return Err(NfcError::InvalidArgument(format!(
                "Bluetooth LE local name is {} octets, maximum is {MAX_NAME_OCTETS}",
                name.len()
            )));
        }
        for (ad_type, value) in &self.extra {
            if matches!(
                *ad_type,
                AD_FLAGS
                    | AD_SHORT_NAME
                    | AD_COMPLETE_NAME
                    | AD_APPEARANCE
                    | AD_LE_ADDRESS
                    | AD_LE_ROLE
            ) {
                return Err(NfcError::InvalidArgument(format!(
                    "Bluetooth LE extra AD type 0x{ad_type:02X} conflicts with a known field"
                )));
            }
            if value.len() > 254 {
                return Err(NfcError::InvalidArgument(format!(
                    "Bluetooth LE AD type 0x{ad_type:02X} value is too large"
                )));
            }
        }
        if self.address.is_none()
            && self.role.is_none()
            && self.name.is_none()
            && self.appearance.is_none()
            && self.flags.is_none()
            && self.extra.is_empty()
        {
            return Err(NfcError::InvalidArgument(
                "Bluetooth LE out-of-band record is empty".to_owned(),
            ));
        }
        Ok(())
    }

    /// Encode the record.
    pub fn to_record(&self) -> Result<NdefRecord> {
        self.validate()?;
        let mut payload = Vec::new();
        if let Some(flags) = self.flags {
            push_ad(&mut payload, AD_FLAGS, &[flags])?;
        }
        if let Some(appearance) = self.appearance {
            push_ad(&mut payload, AD_APPEARANCE, &appearance.to_le_bytes())?;
        }
        if let Some((address, address_type)) = self.address {
            let mut value = Vec::with_capacity(7);
            value.extend(address.iter().rev());
            value.push(address_type.wire_code());
            push_ad(&mut payload, AD_LE_ADDRESS, &value)?;
        }
        if let Some(role) = self.role {
            push_ad(&mut payload, AD_LE_ROLE, &[role.wire_code()])?;
        }
        if let Some(name) = &self.name {
            push_ad(&mut payload, AD_COMPLETE_NAME, name.as_bytes())?;
        }
        for (ad_type, value) in &self.extra {
            push_ad(&mut payload, *ad_type, value)?;
        }
        NdefRecord::media(MEDIA_TYPE, payload)
    }

    /// Decode a Bluetooth LE out-of-band record.
    pub fn from_record(record: &NdefRecord) -> Result<Self> {
        if record.tnf() != Tnf::Media || record.record_type() != MEDIA_TYPE {
            return Err(NfcError::InvalidArgument(
                "record is not an application/vnd.bluetooth.le.oob media record".to_owned(),
            ));
        }
        let mut decoded = Self::default();
        let mut offset = 0;
        let payload = record.payload();
        while offset < payload.len() {
            let length = usize::from(payload[offset]);
            offset += 1;
            if length == 0 {
                continue;
            }
            let end = offset.checked_add(length).ok_or_else(|| {
                NfcError::InvalidArgument("Bluetooth LE AD length overflows".to_owned())
            })?;
            let data = payload.get(offset..end).ok_or_else(|| {
                NfcError::InvalidArgument(format!(
                    "Bluetooth LE AD structure at byte {offset} declares {length} octets but only {} are available",
                    payload.len().saturating_sub(offset)
                ))
            })?;
            offset = end;
            let ad_type = data[0];
            let value = &data[1..];
            match ad_type {
                AD_FLAGS => {
                    require_absent(decoded.flags.is_some(), "Flags")?;
                    let byte = *value.first().ok_or_else(|| {
                        NfcError::InvalidArgument("Bluetooth LE Flags AD is empty".to_owned())
                    })?;
                    if value.len() != 1 {
                        return Err(NfcError::InvalidArgument(format!(
                            "Bluetooth LE Flags AD is {} octets, expected 1",
                            value.len()
                        )));
                    }
                    decoded.flags = Some(byte);
                }
                AD_APPEARANCE => {
                    require_absent(decoded.appearance.is_some(), "Appearance")?;
                    let bytes: [u8; 2] = value.try_into().map_err(|_| {
                        NfcError::InvalidArgument(format!(
                            "Bluetooth LE Appearance AD is {} octets, expected 2",
                            value.len()
                        ))
                    })?;
                    decoded.appearance = Some(u16::from_le_bytes(bytes));
                }
                AD_LE_ADDRESS => {
                    require_absent(decoded.address.is_some(), "LE Bluetooth Device Address")?;
                    if value.len() != 7 {
                        return Err(NfcError::InvalidArgument(format!(
                            "Bluetooth LE address AD is {} octets, expected 7",
                            value.len()
                        )));
                    }
                    let mut address = [0u8; 6];
                    for (index, byte) in address.iter_mut().enumerate() {
                        *byte = value[5 - index];
                    }
                    decoded.address = Some((address, LeAddressType::from_wire(value[6])?));
                }
                AD_LE_ROLE => {
                    require_absent(decoded.role.is_some(), "LE Role")?;
                    if value.len() != 1 {
                        return Err(NfcError::InvalidArgument(format!(
                            "Bluetooth LE Role AD is {} octets, expected 1",
                            value.len()
                        )));
                    }
                    decoded.role = Some(LeRole::from_wire(value[0])?);
                }
                AD_SHORT_NAME | AD_COMPLETE_NAME => {
                    require_absent(decoded.name.is_some(), "Local Name")?;
                    decoded.name = Some(
                        std::str::from_utf8(value)
                            .map_err(|_| {
                                NfcError::InvalidArgument(
                                    "Bluetooth LE local name is not valid UTF-8".to_owned(),
                                )
                            })?
                            .to_owned(),
                    );
                }
                _ => decoded.extra.push((ad_type, value.to_vec())),
            }
        }
        decoded.validate()?;
        Ok(decoded)
    }
}

fn push_ad(output: &mut Vec<u8>, ad_type: u8, value: &[u8]) -> Result<()> {
    let length = value.len() + 1;
    if length > usize::from(u8::MAX) {
        return Err(NfcError::InvalidArgument(format!(
            "Bluetooth LE AD type 0x{ad_type:02X} is too large"
        )));
    }
    output.push(length as u8);
    output.push(ad_type);
    output.extend_from_slice(value);
    Ok(())
}

fn require_absent(present: bool, name: &str) -> Result<()> {
    if present {
        Err(NfcError::InvalidArgument(format!(
            "Bluetooth LE record contains more than one {name} AD structure"
        )))
    } else {
        Ok(())
    }
}
