//! Wi-Fi Simple Configuration (WSC) credential record template.

use nfc_core::{NfcError, Result};

use crate::record::{NdefRecord, Tnf};

const MEDIA_TYPE: &[u8] = b"application/vnd.wfa.wsc";

const ATTR_AUTHENTICATION_TYPE: u16 = 0x1003;
const ATTR_CREDENTIAL: u16 = 0x100E;
const ATTR_ENCRYPTION_TYPE: u16 = 0x100F;
const ATTR_MAC_ADDRESS: u16 = 0x1020;
const ATTR_NETWORK_KEY: u16 = 0x1027;
const ATTR_SSID: u16 = 0x1045;

/// A WSC authentication-type bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WifiAuthentication(u16);

impl WifiAuthentication {
    /// Open network.
    pub const OPEN: Self = Self(0x0001);
    /// WPA-Personal (PSK).
    pub const WPA_PERSONAL: Self = Self(0x0002);
    /// Shared-key authentication (deprecated).
    pub const SHARED: Self = Self(0x0004);
    /// WPA-Enterprise.
    pub const WPA_ENTERPRISE: Self = Self(0x0008);
    /// WPA2-Enterprise.
    pub const WPA2_ENTERPRISE: Self = Self(0x0010);
    /// WPA2-Personal (PSK).
    pub const WPA2_PERSONAL: Self = Self(0x0020);

    /// Create a bitmap value. Zero is rejected.
    pub fn from_bits(bits: u16) -> Result<Self> {
        if bits == 0 {
            return Err(NfcError::InvalidArgument(
                "Wi-Fi authentication type must not be zero".to_owned(),
            ));
        }
        Ok(Self(bits))
    }

    /// The raw bitmap.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether every bit in `other` is present.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A WSC encryption-type bitmap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WifiEncryption(u16);

impl WifiEncryption {
    /// No encryption.
    pub const NONE: Self = Self(0x0001);
    /// WEP (deprecated).
    pub const WEP: Self = Self(0x0002);
    /// TKIP.
    pub const TKIP: Self = Self(0x0004);
    /// AES.
    pub const AES: Self = Self(0x0008);

    /// Create a bitmap value. Zero is rejected.
    pub fn from_bits(bits: u16) -> Result<Self> {
        if bits == 0 {
            return Err(NfcError::InvalidArgument(
                "Wi-Fi encryption type must not be zero".to_owned(),
            ));
        }
        Ok(Self(bits))
    }

    /// The raw bitmap.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether every bit in `other` is present.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A Wi-Fi network credential encoded as a WSC Credential attribute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiRecord {
    /// Network SSID, 1 to 32 octets.
    pub ssid: Vec<u8>,
    /// Authentication type bitmap.
    pub authentication: WifiAuthentication,
    /// Encryption type bitmap.
    pub encryption: WifiEncryption,
    /// Network key/passphrase when the authentication type uses one.
    pub network_key: Option<Vec<u8>>,
    /// Optional access-point MAC address.
    pub mac_address: Option<[u8; 6]>,
    /// Unknown credential attributes, preserved in original order.
    pub credential_attributes: Vec<(u16, Vec<u8>)>,
    /// Unknown top-level attributes, preserved in original order.
    pub outer_attributes: Vec<(u16, Vec<u8>)>,
}

impl WifiRecord {
    /// Create a credential with no network key or MAC address.
    pub fn new(
        ssid: impl Into<Vec<u8>>,
        authentication: WifiAuthentication,
        encryption: WifiEncryption,
    ) -> Result<Self> {
        let record = Self {
            ssid: ssid.into(),
            authentication,
            encryption,
            network_key: None,
            mac_address: None,
            credential_attributes: Vec::new(),
            outer_attributes: Vec::new(),
        };
        record.validate()?;
        Ok(record)
    }

    /// Set the network key.
    pub fn with_network_key(mut self, key: impl Into<Vec<u8>>) -> Result<Self> {
        self.network_key = Some(key.into());
        self.validate()?;
        Ok(self)
    }

    /// Set the access-point MAC address.
    pub fn with_mac_address(mut self, address: [u8; 6]) -> Result<Self> {
        self.mac_address = Some(address);
        self.validate()?;
        Ok(self)
    }

    /// Validate SSID, authentication, encryption, and key rules.
    pub fn validate(&self) -> Result<()> {
        if self.ssid.is_empty() || self.ssid.len() > 32 {
            return Err(NfcError::InvalidArgument(format!(
                "Wi-Fi SSID is {} octets, expected 1 to 32",
                self.ssid.len()
            )));
        }
        if self.authentication.bits() == 0 {
            return Err(NfcError::InvalidArgument(
                "Wi-Fi authentication type must not be zero".to_owned(),
            ));
        }
        if self.encryption.bits() == 0 {
            return Err(NfcError::InvalidArgument(
                "Wi-Fi encryption type must not be zero".to_owned(),
            ));
        }

        let personal = WifiAuthentication(
            WifiAuthentication::WPA_PERSONAL.bits() | WifiAuthentication::WPA2_PERSONAL.bits(),
        );
        if self.authentication.contains(personal) {
            let key = self.network_key.as_deref().ok_or_else(|| {
                NfcError::InvalidArgument("WPA/WPA2-Personal requires a network key".to_owned())
            })?;
            validate_psk(key)?;
        } else if self.authentication == WifiAuthentication::OPEN {
            if self.network_key.is_some() {
                return Err(NfcError::InvalidArgument(
                    "an open Wi-Fi network must not carry a network key".to_owned(),
                ));
            }
        } else if self.network_key.as_ref().is_some_and(Vec::is_empty) {
            return Err(NfcError::InvalidArgument(
                "Wi-Fi network key must not be empty when present".to_owned(),
            ));
        }

        for (attribute, value) in &self.credential_attributes {
            if value.len() > usize::from(u16::MAX) {
                return Err(NfcError::InvalidArgument(format!(
                    "WSC credential attribute 0x{attribute:04X} is too large"
                )));
            }
        }
        for (attribute, value) in &self.outer_attributes {
            if value.len() > usize::from(u16::MAX) {
                return Err(NfcError::InvalidArgument(format!(
                    "WSC attribute 0x{attribute:04X} is too large"
                )));
            }
        }
        Ok(())
    }

    /// Encode the credential as an `application/vnd.wfa.wsc` media record.
    pub fn to_record(&self) -> Result<NdefRecord> {
        self.validate()?;
        let mut credential = Vec::new();
        push_attribute(&mut credential, ATTR_SSID, &self.ssid)?;
        push_attribute(
            &mut credential,
            ATTR_AUTHENTICATION_TYPE,
            &self.authentication.bits().to_be_bytes(),
        )?;
        push_attribute(
            &mut credential,
            ATTR_ENCRYPTION_TYPE,
            &self.encryption.bits().to_be_bytes(),
        )?;
        if let Some(key) = &self.network_key {
            push_attribute(&mut credential, ATTR_NETWORK_KEY, key)?;
        }
        if let Some(address) = &self.mac_address {
            push_attribute(&mut credential, ATTR_MAC_ADDRESS, address)?;
        }
        for (attribute, value) in &self.credential_attributes {
            push_attribute(&mut credential, *attribute, value)?;
        }

        let mut payload = Vec::new();
        push_attribute(&mut payload, ATTR_CREDENTIAL, &credential)?;
        for (attribute, value) in &self.outer_attributes {
            push_attribute(&mut payload, *attribute, value)?;
        }
        NdefRecord::media(MEDIA_TYPE, payload)
    }

    /// Decode an `application/vnd.wfa.wsc` media record.
    pub fn from_record(record: &NdefRecord) -> Result<Self> {
        if record.tnf() != Tnf::Media || record.record_type() != MEDIA_TYPE {
            return Err(NfcError::InvalidArgument(
                "record is not an application/vnd.wfa.wsc media record".to_owned(),
            ));
        }
        let payload = record.payload();
        let start = if payload.len() >= 2
            && usize::from(u16::from_be_bytes([payload[0], payload[1]])) == payload.len() - 2
        {
            2
        } else {
            0
        };
        let outer = parse_attributes(&payload[start..], "WSC payload")?;

        let mut credential: Option<Vec<u8>> = None;
        let mut outer_attributes = Vec::new();
        for (attribute, value) in outer {
            if attribute == ATTR_CREDENTIAL && credential.is_none() {
                credential = Some(value);
            } else {
                outer_attributes.push((attribute, value));
            }
        }
        let credential = credential.ok_or_else(|| {
            NfcError::InvalidArgument("WSC payload has no Credential attribute".to_owned())
        })?;
        let inner = parse_attributes(&credential, "WSC Credential")?;

        let ssid = take_attribute(&inner, ATTR_SSID)?
            .ok_or_else(|| NfcError::InvalidArgument("WSC Credential has no SSID".to_owned()))?;
        let auth = take_attribute(&inner, ATTR_AUTHENTICATION_TYPE)?.ok_or_else(|| {
            NfcError::InvalidArgument("WSC Credential has no authentication type".to_owned())
        })?;
        let encryption = take_attribute(&inner, ATTR_ENCRYPTION_TYPE)?.ok_or_else(|| {
            NfcError::InvalidArgument("WSC Credential has no encryption type".to_owned())
        })?;
        let network_key = take_attribute(&inner, ATTR_NETWORK_KEY)?;
        let mac_address = take_attribute(&inner, ATTR_MAC_ADDRESS)?;

        let authentication =
            WifiAuthentication::from_bits(read_u16(&auth, "authentication type")?)?;
        let encryption = WifiEncryption::from_bits(read_u16(&encryption, "encryption type")?)?;
        let mac_address = match mac_address {
            Some(value) => {
                let bytes: [u8; 6] = value.try_into().map_err(|value: Vec<u8>| {
                    NfcError::InvalidArgument(format!(
                        "WSC MAC address is {} octets, expected 6",
                        value.len()
                    ))
                })?;
                Some(bytes)
            }
            None => None,
        };

        let credential_attributes = inner
            .into_iter()
            .filter(|(attribute, _)| {
                !matches!(
                    *attribute,
                    ATTR_SSID
                        | ATTR_AUTHENTICATION_TYPE
                        | ATTR_ENCRYPTION_TYPE
                        | ATTR_NETWORK_KEY
                        | ATTR_MAC_ADDRESS
                )
            })
            .collect();

        let record = Self {
            ssid,
            authentication,
            encryption,
            network_key,
            mac_address,
            credential_attributes,
            outer_attributes,
        };
        record.validate()?;
        Ok(record)
    }
}

fn validate_psk(key: &[u8]) -> Result<()> {
    if (8..=63).contains(&key.len()) {
        return Ok(());
    }
    if key.len() == 64 && key.iter().all(u8::is_ascii_hexdigit) {
        return Ok(());
    }
    Err(NfcError::InvalidArgument(format!(
        "WPA passphrase is {} octets; expected 8 to 63 octets or 64 hexadecimal digits",
        key.len()
    )))
}

fn push_attribute(output: &mut Vec<u8>, attribute: u16, value: &[u8]) -> Result<()> {
    if value.len() > usize::from(u16::MAX) {
        return Err(NfcError::InvalidArgument(format!(
            "WSC attribute 0x{attribute:04X} is too large"
        )));
    }
    output.extend_from_slice(&attribute.to_be_bytes());
    output.extend_from_slice(&(value.len() as u16).to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn parse_attributes(input: &[u8], context: &str) -> Result<Vec<(u16, Vec<u8>)>> {
    let mut attributes = Vec::new();
    let mut offset = 0;
    while offset < input.len() {
        let header = input.get(offset..offset + 4).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "{context} attribute header at byte {offset} is truncated"
            ))
        })?;
        let attribute = u16::from_be_bytes([header[0], header[1]]);
        let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
        let value_start = offset + 4;
        let value_end = value_start.checked_add(length).ok_or_else(|| {
            NfcError::InvalidArgument(format!("{context} attribute length overflows"))
        })?;
        let value = input.get(value_start..value_end).ok_or_else(|| {
            NfcError::InvalidArgument(format!(
                "{context} attribute 0x{attribute:04X} at byte {offset} declares {length} octets but only {} are available",
                input.len().saturating_sub(value_start)
            ))
        })?;
        attributes.push((attribute, value.to_vec()));
        offset = value_end;
    }
    Ok(attributes)
}

fn take_attribute(attributes: &[(u16, Vec<u8>)], wanted: u16) -> Result<Option<Vec<u8>>> {
    let mut found = None;
    for (attribute, value) in attributes {
        if *attribute == wanted {
            if found.is_some() {
                return Err(NfcError::InvalidArgument(format!(
                    "WSC attribute 0x{wanted:04X} appears more than once"
                )));
            }
            found = Some(value.clone());
        }
    }
    Ok(found)
}

fn read_u16(value: &[u8], name: &str) -> Result<u16> {
    let bytes: [u8; 2] = value.try_into().map_err(|_| {
        NfcError::InvalidArgument(format!("WSC {name} is {} octets, expected 2", value.len()))
    })?;
    Ok(u16::from_be_bytes(bytes))
}
