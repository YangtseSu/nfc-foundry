//! Read-only EMV payment-card public data.
//!
//! The profile selects the PPSE, selects the highest-priority application,
//! sends Get Processing Options with zero-amount terminal parameters, and reads
//! the records named by the AFL. It returns the application list, masked PAN,
//! expiry, cardholder name, AIP/AFL facts, ATC, and record count.
//!
//! Only SELECT, Get Processing Options, and READ RECORD are constructed. There
//! is no GENERATE AC, issuer authentication, PIN, balance, transaction-history,
//! top-up, or write path. The card may include a cryptogram in its Get
//! Processing Options response; this profile discards it and never sends an
//! authorization request.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use nfc_core::{CancelToken, NfcError, Result};
use nfc_transport::Reader;

use crate::apdu;
use crate::fci;
use crate::tlv;
use crate::tunion::{CardDate, bcd_u8, hex_upper, mask_card_number, valid_date};

/// Payment System Environment directory used by contactless EMV cards.
pub const PPSE_NAME: &[u8] = b"2PAY.SYS.DDF01";
/// qVSDC terminal transaction qualifiers accepted by UnionPay and Visa.
pub const TTQ_QVSDC: [u8; 4] = [0x26, 0x00, 0x00, 0x00];
/// Terminal country code used for the zero-amount read (China).
pub const TERMINAL_COUNTRY: [u8; 2] = [0x01, 0x56];
/// Transaction currency code used for the zero-amount read (CNY).
pub const TRANSACTION_CURRENCY: [u8; 2] = [0x01, 0x56];
/// Largest application list shown and searched.
pub const MAX_APPLICATIONS: usize = 32;
/// Largest AFL entry count accepted from one response.
pub const MAX_AFL_ENTRIES: usize = 16;
/// Largest record count read during one workflow.
pub const MAX_RECORDS: usize = 64;
/// Largest GPO data field a 247-byte command APDU can carry.
const MAX_GPO_DATA: usize = 238;

/// One application advertised by the PPSE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmvApplication {
    pub aid: Vec<u8>,
    pub label: Option<String>,
    pub priority: Option<u8>,
}

impl EmvApplication {
    pub fn aid_hex(&self) -> String {
        hex_upper(&self.aid)
    }

    pub fn display_label(&self) -> &str {
        self.label.as_deref().unwrap_or("payment application")
    }
}

/// Expiry at month precision from Track 2, or day precision from `5F24`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmvExpiry {
    pub year: u16,
    pub month: u8,
    pub day: Option<u8>,
}

impl EmvExpiry {
    const fn from_date(date: CardDate) -> Self {
        Self {
            year: date.year,
            month: date.month,
            day: Some(date.day),
        }
    }
}

impl fmt::Display for EmvExpiry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.day {
            Some(day) => write!(formatter, "{:04}-{:02}-{:02}", self.year, self.month, day),
            None => write!(formatter, "{:04}-{:02}", self.year, self.month),
        }
    }
}

/// Read-only facts returned by one payment application.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmvCard {
    pub applications: Vec<EmvApplication>,
    pub aid: Vec<u8>,
    pub label: String,
    pub priority: Option<u8>,
    pub app_version: Option<Vec<u8>>,
    /// Masked PAN. The full number never leaves the read function.
    pub pan_masked: Option<String>,
    pub expires: Option<EmvExpiry>,
    pub effective: Option<CardDate>,
    pub cardholder_name: Option<String>,
    pub pan_sequence: Option<u8>,
    pub service_code: Option<[u8; 3]>,
    pub aip: [u8; 2],
    pub atc: Option<u16>,
    /// Issuer country code `5F28`, uppercase hexadecimal.
    pub issuer_country: Option<[u8; 2]>,
    /// Application usage control `9F07`, uppercase hexadecimal.
    pub application_usage: Option<[u8; 2]>,
    pub records_read: usize,
    pub complete: bool,
    pub warnings: Vec<String>,
}

impl EmvCard {
    pub fn aid_hex(&self) -> String {
        hex_upper(&self.aid)
    }

    pub fn app_version_hex(&self) -> Option<String> {
        self.app_version.as_deref().map(hex_upper)
    }

    pub fn aip_hex(&self) -> String {
        hex_upper(&self.aip)
    }

    pub fn aip_capabilities(&self) -> Vec<&'static str> {
        aip_capabilities(self.aip)
    }

    pub fn service_code_text(&self) -> Option<String> {
        self.service_code
            .map(|code| String::from_utf8_lossy(&code).into_owned())
    }

    pub fn issuer_country_hex(&self) -> Option<String> {
        self.issuer_country.map(|value| hex_upper(&value))
    }

    pub fn application_usage_hex(&self) -> Option<String> {
        self.application_usage.map(|value| hex_upper(&value))
    }
}

/// Terminal values inserted into the application PDOL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalParameters {
    pub ttq: [u8; 4],
    pub amount: [u8; 6],
    pub amount_other: [u8; 6],
    pub terminal_country: [u8; 2],
    pub tvr: [u8; 5],
    pub currency: [u8; 2],
    /// Transaction date in BCD `YYMMDD`.
    pub date: [u8; 3],
    pub transaction_type: u8,
    pub unpredictable_number: [u8; 4],
}

impl Default for TerminalParameters {
    fn default() -> Self {
        Self {
            ttq: TTQ_QVSDC,
            amount: [0; 6],
            amount_other: [0; 6],
            terminal_country: TERMINAL_COUNTRY,
            tvr: [0; 5],
            currency: TRANSACTION_CURRENCY,
            date: current_date_bcd(),
            transaction_type: 0x00,
            unpredictable_number: [0x01, 0x02, 0x03, 0x04],
        }
    }
}

/// Read the highest-priority application with zero-amount terminal parameters.
pub fn read(reader: &mut dyn Reader, cancel: &CancelToken) -> Result<EmvCard> {
    read_with(reader, TerminalParameters::default(), cancel)
}

/// Read the highest-priority application with explicit terminal parameters.
pub fn read_with(
    reader: &mut dyn Reader,
    parameters: TerminalParameters,
    cancel: &CancelToken,
) -> Result<EmvCard> {
    cancel.check()?;
    let mut warnings = Vec::new();

    let ppse = apdu::select_by_name(reader, PPSE_NAME, "selecting the EMV PPSE", cancel)?;
    if refused(ppse.sw) {
        return Err(NfcError::Unsupported(
            "the card does not expose an EMV PPSE directory",
        ));
    }
    apdu::ensure_ok(&ppse, "selecting the EMV PPSE")?;
    let entries = fci::parse_ppse(&ppse.data)?;
    if entries.is_empty() {
        return Err(NfcError::Unsupported(
            "the PPSE directory lists no payment application",
        ));
    }
    if entries.len() > MAX_APPLICATIONS {
        note(
            &mut warnings,
            format!(
                "the PPSE lists {} applications; only the first {MAX_APPLICATIONS} are shown",
                entries.len()
            ),
        );
    }
    let applications: Vec<EmvApplication> = entries
        .iter()
        .take(MAX_APPLICATIONS)
        .map(|entry| EmvApplication {
            aid: entry.aid.clone(),
            label: entry.label.clone(),
            priority: entry.priority,
        })
        .collect();
    let selected = entries
        .iter()
        .enumerate()
        .min_by_key(|(index, entry)| (entry.priority.unwrap_or(u8::MAX), *index))
        .map(|(index, _)| &entries[index])
        .ok_or(NfcError::NoCard)?;

    let selected_response = apdu::select_by_name(
        reader,
        &selected.aid,
        "selecting the EMV application",
        cancel,
    )?;
    if refused(selected_response.sw) {
        return Err(NfcError::Unsupported(
            "the card does not expose the selected payment application",
        ));
    }
    apdu::ensure_ok(&selected_response, "selecting the EMV application")?;
    let application = fci::parse_application(&selected_response.data)?;
    if application.df_name != selected.aid {
        note(
            &mut warnings,
            format!(
                "the selected DF name is {} instead of the PPSE AID {}",
                hex_upper(&application.df_name),
                hex_upper(&selected.aid)
            ),
        );
    }

    let pdol = application.pdol.as_deref().unwrap_or(&[]);
    let pdol_data = build_pdol_data(pdol, &parameters, &mut warnings)?;
    let gpo_apdu = gpo_apdu(&pdol_data)?;
    let gpo = apdu::exchange(reader, &gpo_apdu, "Get Processing Options", cancel)?;
    if gpo.sw == [0x69, 0x85] {
        return Err(NfcError::Io(
            "Get Processing Options returned 6985 (conditions of use not satisfied)".to_owned(),
        ));
    }
    apdu::ensure_ok(&gpo, "Get Processing Options")?;
    let parsed = parse_gpo(&gpo.data)?;

    let mut collected = Collected::default();
    merge_fields(&parsed.fields, &mut collected, &mut warnings);
    let mut complete = true;
    let records_read = read_afl(
        reader,
        &parsed.afl,
        &mut collected,
        &mut warnings,
        &mut complete,
        cancel,
    )?;

    if let (Some(day), Some(month)) = (
        collected.expiry_5f24.map(|value| (value.year, value.month)),
        collected
            .expiry_track2
            .map(|value| (value.year, value.month)),
    ) && day != month
    {
        note(
            &mut warnings,
            "the expiry date differs between tag 5F24 and Track 2".to_owned(),
        );
    }
    let expires = collected.expiry_5f24.or(collected.expiry_track2);
    if collected.pan.is_none() {
        note(
            &mut warnings,
            "the card did not return a primary account number".to_owned(),
        );
        complete = false;
    }
    if expires.is_none() {
        note(
            &mut warnings,
            "the card did not return an expiry date".to_owned(),
        );
    }
    let app_version = application.version.or(collected.app_version);
    let label = application
        .label
        .or_else(|| selected.label.clone())
        .unwrap_or_else(|| format!("AID {}", hex_upper(&selected.aid)));
    let aid = if application.df_name.is_empty() {
        selected.aid.clone()
    } else {
        application.df_name
    };

    Ok(EmvCard {
        applications,
        aid,
        label,
        priority: application.priority.or(selected.priority),
        app_version,
        pan_masked: collected.pan.as_deref().map(mask_card_number),
        expires,
        effective: collected.effective,
        cardholder_name: collected.name,
        pan_sequence: collected.pan_sequence,
        service_code: collected.service_code,
        aip: parsed.aip,
        atc: collected.atc,
        issuer_country: collected.issuer_country,
        application_usage: collected.application_usage,
        records_read,
        complete,
        warnings,
    })
}

/// Decode the AIP byte that carries the application capabilities.
pub fn aip_capabilities(aip: [u8; 2]) -> Vec<&'static str> {
    let byte = aip[0];
    [
        (0x40, "SDA"),
        (0x20, "DDA"),
        (0x10, "cardholder verification"),
        (0x08, "terminal risk management"),
        (0x04, "issuer authentication"),
        (0x02, "on-device CVM"),
        (0x01, "CDA"),
    ]
    .into_iter()
    .filter_map(|(bit, label)| (byte & bit != 0).then_some(label))
    .collect()
}

/// AIP and AFL from a Get Processing Options response.
struct Gpo {
    aip: [u8; 2],
    afl: Vec<AflEntry>,
    fields: Vec<(u32, Vec<u8>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AflEntry {
    sfi: u8,
    first: u8,
    last: u8,
}

#[derive(Default)]
struct Collected {
    pan: Option<String>,
    expiry_5f24: Option<EmvExpiry>,
    expiry_track2: Option<EmvExpiry>,
    effective: Option<CardDate>,
    name: Option<String>,
    pan_sequence: Option<u8>,
    service_code: Option<[u8; 3]>,
    issuer_country: Option<[u8; 2]>,
    application_usage: Option<[u8; 2]>,
    app_version: Option<Vec<u8>>,
    atc: Option<u16>,
}

fn parse_gpo(data: &[u8]) -> Result<Gpo> {
    let objects = tlv::parse(data)?;
    let template = objects
        .first()
        .ok_or_else(|| NfcError::Io("the Get Processing Options response is empty".to_owned()))?;
    match template.tag {
        0x77 => {
            let fields = tlv::collect(template.value)?;
            let aip_value = fields
                .iter()
                .find(|(tag, _)| *tag == 0x82)
                .map(|(_, value)| value)
                .ok_or_else(|| NfcError::Io("the GPO response has no 82 AIP".to_owned()))?;
            if aip_value.len() != 2 {
                return Err(NfcError::Io(format!(
                    "the GPO AIP is {} bytes, expected 2",
                    aip_value.len()
                )));
            }
            let aip = [aip_value[0], aip_value[1]];
            let afl = fields
                .iter()
                .find(|(tag, _)| *tag == 0x94)
                .map(|(_, value)| parse_afl(value))
                .transpose()?
                .unwrap_or_default();
            Ok(Gpo { aip, afl, fields })
        }
        0x80 => {
            let body = template.value;
            if body.len() < 2 {
                return Err(NfcError::Io(
                    "the GPO response is shorter than the two-byte AIP".to_owned(),
                ));
            }
            let aip = [body[0], body[1]];
            let afl = parse_afl(&body[2..])?;
            Ok(Gpo {
                aip,
                afl,
                fields: Vec::new(),
            })
        }
        other => Err(NfcError::Io(format!(
            "unexpected GPO response template {:02X}",
            other
        ))),
    }
}

fn parse_afl(data: &[u8]) -> Result<Vec<AflEntry>> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    if !data.len().is_multiple_of(4) {
        return Err(NfcError::Io(format!(
            "the GPO AFL is {} bytes, expected a multiple of 4",
            data.len()
        )));
    }
    if data.len() / 4 > MAX_AFL_ENTRIES {
        return Err(NfcError::Io(format!(
            "the GPO AFL has {} entries, at most {MAX_AFL_ENTRIES} are accepted",
            data.len() / 4
        )));
    }
    let mut entries = Vec::with_capacity(data.len() / 4);
    for chunk in data.chunks(4) {
        let sfi = chunk[0] >> 3;
        let first = chunk[1];
        let last = chunk[2];
        if sfi == 0 || sfi > 30 {
            return Err(NfcError::Io(format!("the AFL SFI {sfi} is outside 1..=30")));
        }
        if first == 0 || last < first || last > 30 {
            return Err(NfcError::Io(format!(
                "the AFL records {first}..={last} on SFI {sfi} are invalid"
            )));
        }
        entries.push(AflEntry { sfi, first, last });
    }
    Ok(entries)
}

fn read_afl(
    reader: &mut dyn Reader,
    afl: &[AflEntry],
    collected: &mut Collected,
    warnings: &mut Vec<String>,
    complete: &mut bool,
    cancel: &CancelToken,
) -> Result<usize> {
    let mut read_count = 0;
    for entry in afl {
        for record in entry.first..=entry.last {
            if read_count >= MAX_RECORDS {
                note(warnings, format!("stopped after {MAX_RECORDS} records"));
                *complete = false;
                return Ok(read_count);
            }
            let context = format!("reading EMV record {record} on SFI {}", entry.sfi);
            let response = apdu::read_record(reader, record, entry.sfi, &context, cancel)?;
            if response.sw == [0x6A, 0x83] {
                break;
            }
            if !response.is_ok() {
                note(
                    warnings,
                    format!(
                        "{context} returned status {}",
                        apdu::status_hex(response.sw)
                    ),
                );
                *complete = false;
                break;
            }
            match parse_record(&response.data) {
                Ok(fields) => {
                    merge_fields(&fields, collected, warnings);
                    read_count += 1;
                }
                Err(error) => {
                    note(warnings, format!("{context} could not be parsed: {error}"));
                    *complete = false;
                }
            }
        }
    }
    Ok(read_count)
}

fn parse_record(data: &[u8]) -> Result<Vec<(u32, Vec<u8>)>> {
    let objects = tlv::parse(data)?;
    let template = objects
        .iter()
        .find(|object| object.tag == 0x70)
        .ok_or_else(|| NfcError::Io("the record has no 70 template".to_owned()))?;
    tlv::collect(template.value)
}

fn merge_fields(fields: &[(u32, Vec<u8>)], collected: &mut Collected, warnings: &mut Vec<String>) {
    for (tag, value) in fields {
        match *tag {
            0x5A => match numeric_digits(value) {
                Some(pan) => set_pan(collected, pan, warnings),
                None => note(
                    warnings,
                    "tag 5A is not a numeric primary account number".to_owned(),
                ),
            },
            0x57 => match parse_track2(value) {
                Ok(track2) => {
                    if let Some(pan) = track2.pan {
                        set_pan(collected, pan, warnings);
                    }
                    if collected.expiry_track2.is_none() {
                        collected.expiry_track2 = track2.expiry;
                    }
                    if collected.service_code.is_none() {
                        collected.service_code = track2.service_code;
                    }
                }
                Err(error) => note(warnings, format!("tag 57 could not be parsed: {error}")),
            },
            0x5F24 => match bcd_date(value) {
                Some(date) if collected.expiry_5f24.is_none() => {
                    collected.expiry_5f24 = Some(EmvExpiry::from_date(date));
                }
                Some(_) => {}
                None => note(
                    warnings,
                    "tag 5F24 is not a valid BCD expiry date".to_owned(),
                ),
            },
            0x5F25 => match bcd_date(value) {
                Some(date) if collected.effective.is_none() => {
                    collected.effective = Some(date);
                }
                Some(_) => {}
                None => note(
                    warnings,
                    "tag 5F25 is not a valid BCD effective date".to_owned(),
                ),
            },
            0x5F20 => {
                if collected.name.is_none() {
                    match parse_name(value) {
                        Ok(Some(name)) => collected.name = Some(name),
                        Ok(None) => {}
                        Err(error) => {
                            note(warnings, format!("tag 5F20 could not be parsed: {error}"))
                        }
                    }
                }
            }
            0x5F34 => {
                if value.len() == 1 {
                    collected.pan_sequence = Some(value[0]);
                } else {
                    note(
                        warnings,
                        format!("tag 5F34 is {} bytes, expected 1", value.len()),
                    );
                }
            }
            0x5F28 if value.len() == 2 => {
                collected.issuer_country = Some([value[0], value[1]]);
            }
            0x9F07 if value.len() == 2 => {
                collected.application_usage = Some([value[0], value[1]]);
            }
            0x9F08 => {
                if collected.app_version.is_none() {
                    collected.app_version = Some(value.clone());
                }
            }
            0x9F36 if value.len() == 2 => {
                collected.atc = Some(u16::from_be_bytes([value[0], value[1]]));
            }
            _ => {}
        }
    }
}

fn set_pan(collected: &mut Collected, pan: String, warnings: &mut Vec<String>) {
    if pan.len() < 8 || pan.len() > 19 {
        note(
            warnings,
            format!(
                "the primary account number is {} digits, expected 8..=19",
                pan.len()
            ),
        );
        return;
    }
    match &collected.pan {
        Some(existing) if *existing != pan => note(
            warnings,
            "the card returned more than one primary account number".to_owned(),
        ),
        Some(_) => {}
        None => collected.pan = Some(pan),
    }
}

fn parse_track2(data: &[u8]) -> std::result::Result<Track2, String> {
    if data
        .iter()
        .position(|byte| *byte == 0x44 || *byte == 0x3D)
        .is_some_and(|end| data[..end].iter().all(|byte| byte.is_ascii_digit()))
    {
        return parse_ascii_track2(data);
    }
    parse_bcd_track2(data)
}

fn parse_ascii_track2(data: &[u8]) -> std::result::Result<Track2, String> {
    let end = data
        .iter()
        .position(|byte| *byte == 0x44 || *byte == 0x3D)
        .ok_or_else(|| "track 2 has no D separator".to_owned())?;
    let pan = ascii_digits(&data[..end]);
    let mut rest = &data[end + 1..];
    while matches!(rest.last(), Some(0x00 | 0x0F | 0xFF)) {
        rest = &rest[..rest.len() - 1];
    }
    let expiry = if rest.len() >= 4 {
        ascii_digits(&rest[..4]).and_then(|value| parse_yy_mm(&value))
    } else {
        None
    };
    let service_code = if rest.len() >= 7 {
        let digits = ascii_digits(&rest[4..7])
            .ok_or_else(|| "track 2 service code is not numeric".to_owned())?;
        let bytes = digits.as_bytes();
        Some([bytes[0], bytes[1], bytes[2]])
    } else {
        None
    };
    Ok(Track2 {
        pan,
        expiry,
        service_code,
    })
}

fn parse_bcd_track2(data: &[u8]) -> std::result::Result<Track2, String> {
    let mut nibbles = Vec::with_capacity(data.len() * 2);
    for byte in data {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0F);
    }
    let separator = nibbles
        .iter()
        .position(|nibble| *nibble == 0x0D)
        .ok_or_else(|| "track 2 has no D separator".to_owned())?;
    let mut pan = String::with_capacity(separator);
    for nibble in &nibbles[..separator] {
        if *nibble > 9 {
            return Err("track 2 PAN is not BCD encoded".to_owned());
        }
        pan.push(char::from(b'0' + *nibble));
    }
    if pan.len() < 8 || pan.len() > 19 {
        return Err(format!(
            "track 2 PAN is {} digits, expected 8..=19",
            pan.len()
        ));
    }
    let mut rest = &nibbles[separator + 1..];
    while rest.last() == Some(&0x0F) {
        rest = &rest[..rest.len() - 1];
    }
    if rest.len() < 7 {
        return Err("track 2 is shorter than expiry and service code".to_owned());
    }
    let mut expiry = String::with_capacity(4);
    for nibble in &rest[..4] {
        if *nibble > 9 {
            return Err("track 2 expiry is not BCD encoded".to_owned());
        }
        expiry.push(char::from(b'0' + *nibble));
    }
    let mut service = String::with_capacity(3);
    for nibble in &rest[4..7] {
        if *nibble > 9 {
            return Err("track 2 service code is not BCD encoded".to_owned());
        }
        service.push(char::from(b'0' + *nibble));
    }
    if rest[7..]
        .iter()
        .any(|nibble| *nibble > 9 && *nibble != 0x0F)
    {
        return Err("track 2 discretionary data is not BCD encoded".to_owned());
    }
    let service = service.as_bytes();
    Ok(Track2 {
        pan: Some(pan),
        expiry: parse_yy_mm(&expiry),
        service_code: Some([service[0], service[1], service[2]]),
    })
}

fn numeric_digits(data: &[u8]) -> Option<String> {
    if !data.is_empty() && data.iter().all(|byte| byte.is_ascii_digit()) {
        return String::from_utf8(data.to_vec()).ok();
    }
    let mut digits = String::with_capacity(data.len() * 2);
    for (index, byte) in data.iter().enumerate() {
        let last = index + 1 == data.len();
        for nibble in [byte >> 4, byte & 0x0F] {
            if nibble <= 9 {
                digits.push(char::from(b'0' + nibble));
            } else if nibble != 0x0F || !last {
                return None;
            }
        }
    }
    (!digits.is_empty()).then_some(digits)
}

struct Track2 {
    pan: Option<String>,
    expiry: Option<EmvExpiry>,
    service_code: Option<[u8; 3]>,
}

fn parse_yy_mm(value: &str) -> Option<EmvExpiry> {
    if value.len() != 4 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = 2000 + value[..2].parse::<u16>().ok()?;
    let month = value[2..].parse::<u8>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some(EmvExpiry {
        year,
        month,
        day: None,
    })
}

fn bcd_date(data: &[u8]) -> Option<CardDate> {
    if data.len() != 3 {
        return None;
    }
    let year = 2000 + u16::from(bcd_u8(data[0])?);
    let month = bcd_u8(data[1])?;
    let day = bcd_u8(data[2])?;
    valid_date(year, month, day)
}

fn parse_name(data: &[u8]) -> std::result::Result<Option<String>, String> {
    if data.is_empty() || data == [0x20, 0x2F] {
        return Ok(None);
    }
    if !data.iter().all(|byte| (0x20..=0x7E).contains(byte)) {
        return Err("the cardholder name is not printable ASCII".to_owned());
    }
    let name = String::from_utf8(data.to_vec())
        .map_err(|_| "the cardholder name is not UTF-8".to_owned())?
        .trim()
        .to_owned();
    if name.is_empty() || name == "/" {
        return Ok(None);
    }
    if name.len() > 64 {
        return Err("the cardholder name is longer than 64 bytes".to_owned());
    }
    Ok(Some(name))
}

fn ascii_digits(data: &[u8]) -> Option<String> {
    if data.is_empty() || !data.iter().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    String::from_utf8(data.to_vec()).ok()
}

fn build_pdol_data(
    dol: &[u8],
    parameters: &TerminalParameters,
    warnings: &mut Vec<String>,
) -> Result<Vec<u8>> {
    if dol.is_empty() {
        return Ok(Vec::new());
    }
    let items = parse_dol(dol)?;
    let mut data = Vec::new();
    for (tag, length) in items {
        if data.len() + length > MAX_GPO_DATA {
            return Err(NfcError::InvalidArgument(format!(
                "the PDOL data exceeds the {MAX_GPO_DATA}-byte GPO limit"
            )));
        }
        let value = dol_value(tag, length, parameters, warnings);
        data.extend_from_slice(&value);
    }
    Ok(data)
}

fn parse_dol(dol: &[u8]) -> Result<Vec<(u32, usize)>> {
    let mut items = Vec::new();
    let mut offset = 0;
    while offset < dol.len() {
        let first = dol[offset];
        offset += 1;
        if first == 0x00 {
            return Err(NfcError::Io(
                "the application PDOL contains a zero tag".to_owned(),
            ));
        }
        let mut tag = u32::from(first);
        if first & 0x1F == 0x1F {
            let mut bytes = 1;
            loop {
                let byte = *dol.get(offset).ok_or_else(|| {
                    NfcError::Io("the application PDOL tag is truncated".to_owned())
                })?;
                offset += 1;
                bytes += 1;
                tag = (tag << 8) | u32::from(byte);
                if byte & 0x80 == 0 {
                    break;
                }
                if bytes >= 4 {
                    return Err(NfcError::Io(
                        "the application PDOL tag is longer than 4 bytes".to_owned(),
                    ));
                }
            }
        }
        let length =
            usize::from(*dol.get(offset).ok_or_else(|| {
                NfcError::Io("the application PDOL length is truncated".to_owned())
            })?);
        offset += 1;
        items.push((tag, length));
    }
    Ok(items)
}

fn dol_value(
    tag: u32,
    length: usize,
    parameters: &TerminalParameters,
    warnings: &mut Vec<String>,
) -> Vec<u8> {
    let value = match tag {
        0x9F66 => parameters.ttq.to_vec(),
        0x9F02 => parameters.amount.to_vec(),
        0x9F03 => parameters.amount_other.to_vec(),
        0x9F1A => parameters.terminal_country.to_vec(),
        0x95 => parameters.tvr.to_vec(),
        0x5F2A => parameters.currency.to_vec(),
        0x9A => parameters.date.to_vec(),
        0x9C => vec![parameters.transaction_type],
        0x9F37 => parameters.unpredictable_number.to_vec(),
        0x9F35 => vec![0x22],
        0x9F33 => vec![0xE0, 0xF8, 0xC8],
        0x9F40 => vec![0xF0, 0x00, 0xF0, 0xA0, 0x01],
        0x9F41 => 1u32.to_be_bytes().to_vec(),
        0x9F1E => vec![0; 8],
        0x9F15 => vec![0x59, 0x99],
        0x9F1C => b"NFC FNDR".to_vec(),
        0x9F1D => vec![0; 8],
        0x9F4E => b"NFC Foundry".to_vec(),
        0xDF60 => vec![0x00],
        0xDF69 => vec![0x00],
        _ => {
            note(
                warnings,
                format!(
                    "PDOL tag {} has no terminal value; sending {length} zero bytes",
                    tag_hex(tag)
                ),
            );
            vec![0; length]
        }
    };
    fit_length(value, length, tag, warnings)
}

fn fit_length(mut value: Vec<u8>, length: usize, tag: u32, warnings: &mut Vec<String>) -> Vec<u8> {
    if value.len() == length {
        return value;
    }
    if tag == 0x9F4E {
        value.resize(length, b' ');
        value.truncate(length);
        return value;
    }
    note(
        warnings,
        format!(
            "PDOL tag {} expects {length} bytes but the terminal value has {}",
            tag_hex(tag),
            value.len()
        ),
    );
    value.resize(length, 0);
    value
}

fn gpo_apdu(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() > MAX_GPO_DATA {
        return Err(NfcError::InvalidArgument(format!(
            "the PDOL data is {} bytes, at most {MAX_GPO_DATA} are supported",
            data.len()
        )));
    }
    let mut field = Vec::with_capacity(data.len() + 3);
    field.push(0x83);
    if data.len() < 0x80 {
        field.push(data.len() as u8);
    } else {
        field.push(0x81);
        field.push(data.len() as u8);
    }
    field.extend_from_slice(data);
    if field.len() > 0xFF {
        return Err(NfcError::InvalidArgument(
            "the GPO data field exceeds 255 bytes".to_owned(),
        ));
    }
    let mut apdu = Vec::with_capacity(field.len() + 6);
    apdu.extend_from_slice(&[0x80, 0xA8, 0x00, 0x00, field.len() as u8]);
    apdu.extend_from_slice(&field);
    apdu.push(0x00);
    Ok(apdu)
}

fn refused(sw: [u8; 2]) -> bool {
    matches!(sw, [0x6A, 0x81] | [0x6A, 0x82] | [0x6A, 0x83])
}

fn note(warnings: &mut Vec<String>, warning: String) {
    if !warnings.contains(&warning) {
        warnings.push(warning);
    }
}

fn tag_hex(tag: u32) -> String {
    if tag <= 0xFF {
        format!("{tag:02X}")
    } else if tag <= 0xFFFF {
        format!("{tag:04X}")
    } else {
        format!("{tag:06X}")
    }
}

/// Current UTC date in BCD `YYMMDD`. The transaction date is not a security
/// boundary for a zero-amount read.
fn current_date_bcd() -> [u8; 3] {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    [bcd_value(year % 100), bcd_value(month), bcd_value(day)]
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    (year + i64::from(month <= 2), month, day)
}

fn bcd_value(value: i64) -> u8 {
    let value = value.clamp(0, 99);
    (((value / 10) << 4) | (value % 10)) as u8
}

#[cfg(test)]
mod tests {
    use nfc_core::NfcError;

    use super::*;
    use crate::mock::{MOCK_EMV_AID, MOCK_EMV_PAN, MOCK_EMV_TRACK2, MockEmvCard, MockEmvReader};

    fn parameters() -> TerminalParameters {
        TerminalParameters {
            date: [0x26, 0x11, 0x15],
            ..TerminalParameters::default()
        }
    }

    fn read_mock(card: MockEmvCard) -> (EmvCard, MockEmvReader) {
        let mut reader = MockEmvReader::new(card);
        let result = read_with(&mut reader, parameters(), &CancelToken::new()).expect("EMV read");
        (result, reader)
    }

    #[test]
    fn reads_the_mock_payment_card() {
        let (card, reader) = read_mock(MockEmvCard::default());
        assert_eq!(card.aid, MOCK_EMV_AID);
        assert_eq!(card.label, "Mock Payment");
        assert_eq!(card.priority, Some(1));
        assert_eq!(card.app_version_hex().as_deref(), Some("0030"));
        assert_eq!(card.pan_masked.as_deref(), Some("**************0001"));
        assert!(
            card.pan_masked
                .as_deref()
                .is_some_and(|pan| !pan.contains("622908"))
        );
        assert_eq!(card.expires.expect("expiry").to_string(), "2030-12-31");
        assert_eq!(card.effective.expect("effective").to_string(), "2020-01-01");
        assert_eq!(card.cardholder_name.as_deref(), Some("TEST/CARDHOLDER"));
        assert_eq!(card.pan_sequence, Some(1));
        assert_eq!(card.service_code_text().as_deref(), Some("220"));
        assert_eq!(card.issuer_country_hex().as_deref(), Some("0156"));
        assert_eq!(card.application_usage_hex().as_deref(), Some("FF00"));
        assert_eq!(card.aip_hex(), "7C00");
        assert_eq!(card.atc, Some(0x11));
        assert_eq!(card.records_read, 1);
        assert!(card.complete);
        assert!(card.warnings.is_empty(), "{:?}", card.warnings);
        assert_eq!(card.applications.len(), 1);
        assert_eq!(
            card.aip_capabilities(),
            vec![
                "SDA",
                "DDA",
                "cardholder verification",
                "terminal risk management",
                "issuer authentication"
            ]
        );

        let gpo = reader.gpo_request.expect("GPO request");
        assert_eq!(&gpo[..5], &[0x80, 0xA8, 0x00, 0x00, 0x25]);
        assert_eq!(&gpo[5..7], &[0x83, 0x23]);
        assert_eq!(&gpo[7..11], &[0x26, 0x00, 0x00, 0x00]);
        assert_eq!(&gpo[23..25], &[0x01, 0x56]);
        assert_eq!(&gpo[32..35], &[0x26, 0x11, 0x15]);
        assert_eq!(gpo.last(), Some(&0x00));
        assert_eq!(gpo.len(), 43);
    }

    #[test]
    fn missing_ppse_is_unsupported() {
        let mut reader = MockEmvReader::new(MockEmvCard {
            present: false,
            ..MockEmvCard::default()
        });
        let error =
            read_with(&mut reader, parameters(), &CancelToken::new()).expect_err("missing PPSE");
        assert!(matches!(error, NfcError::Unsupported(_)), "{error}");
    }

    #[test]
    fn refused_gpo_is_an_io_error() {
        let mut reader = MockEmvReader::new(MockEmvCard {
            refuse_gpo: true,
            ..MockEmvCard::default()
        });
        let error =
            read_with(&mut reader, parameters(), &CancelToken::new()).expect_err("refused GPO");
        assert!(matches!(error, NfcError::Io(_)), "{error}");
        assert!(error.to_string().contains("6985"), "{error}");
    }

    #[test]
    fn record_failure_marks_the_result_incomplete() {
        let (card, _reader) = read_mock(MockEmvCard {
            fail_record: Some(1),
            ..MockEmvCard::default()
        });
        assert!(!card.complete);
        assert_eq!(card.records_read, 0);
        assert!(
            card.warnings.iter().any(|warning| warning.contains("6D00")),
            "{:?}",
            card.warnings
        );
    }

    #[test]
    fn unknown_pdol_tags_are_zero_filled_with_a_warning() {
        let (card, reader) = read_mock(MockEmvCard {
            pdol: vec![0x9F, 0x5A, 0x02],
            ..MockEmvCard::default()
        });
        assert!(
            card.warnings.iter().any(|warning| warning.contains("9F5A")),
            "{:?}",
            card.warnings
        );
        let gpo = reader.gpo_request.expect("GPO request");
        assert_eq!(&gpo[5..9], &[0x83, 0x02, 0x00, 0x00]);
    }

    #[test]
    fn track2_decodes_pan_expiry_and_service_code() {
        let track2 = parse_track2(MOCK_EMV_TRACK2).expect("track 2");
        assert_eq!(track2.pan.as_deref(), Some(MOCK_EMV_PAN));
        assert_eq!(track2.expiry.expect("expiry").to_string(), "2030-12");
        assert_eq!(track2.service_code, Some(*b"220"));
        assert!(parse_track2(b"622908000000000001").is_err());
        assert!(parse_track2(b"622908000000000001D3012XX00000000000F").is_err());
    }

    #[test]
    fn masks_the_pan_to_the_last_four_digits() {
        assert_eq!(mask_card_number(MOCK_EMV_PAN), "**************0001");
    }

    #[test]
    fn parses_the_shortest_and_longest_gpo_templates() {
        let format_one =
            parse_gpo(&[0x80, 0x06, 0x20, 0x00, 0x18, 0x01, 0x01, 0x00]).expect("format 1");
        assert_eq!(format_one.aip, [0x20, 0x00]);
        assert_eq!(format_one.afl.len(), 1);

        let format_two = parse_gpo(&MockEmvCard::default().gpo()).expect("format 2");
        assert_eq!(format_two.aip, [0x7C, 0x00]);
        assert_eq!(format_two.afl.len(), 1);
    }

    #[test]
    fn rejects_malformed_afl_entries() {
        assert!(parse_afl(&[0x18, 0x01, 0x04]).is_err());
        assert!(parse_afl(&[0x00, 0x01, 0x01, 0x00]).is_err());
        assert!(parse_afl(&[0x18, 0x04, 0x01, 0x00]).is_err());
    }
}
