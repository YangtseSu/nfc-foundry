//! China T-Union transit card reader.
//!
//! The command sequence follows the Proxmark3 `read_t-union.py` reference:
//! select the PPSE, select application `A000000632010105`, read the electronic
//! purse balance, then read up to ten transaction records from SFI `0x18` and
//! up to thirty travel records from SFI `0x1E`. Every command is read-only.
//!
//! The card number comes from the `9F0C` application metadata and is masked by
//! default at the presentation boundary. Raw responses are never persisted.

use std::fmt;

use nfc_core::{CancelToken, NfcError, Result};
use nfc_transport::Reader;

use crate::apdu;
use crate::fci;

/// Payment System Environment name used by the T-Union reference reader.
pub const PPSE_NAME: &[u8] = b"2PAY.SYS.DDF01";
/// T-Union transit application.
pub const TUNION_AID: [u8; 8] = [0xA0, 0x00, 0x00, 0x06, 0x32, 0x01, 0x01, 0x05];
/// T-Union cash application advertised by some cards.
pub const TUNION_CASH_AID: [u8; 8] = [0xA0, 0x00, 0x00, 0x06, 0x32, 0x01, 0x01, 0x06];
/// Short EF identifier for the transaction log.
pub const TRANSACTION_FILE: u8 = 0x18;
/// Short EF identifier for the travel log.
pub const TRAVEL_FILE: u8 = 0x1E;
pub const MAX_TRANSACTIONS: u8 = 10;
pub const MAX_TRAVELS: u8 = 30;

const TRANSACTION_MIN: usize = 23;
const TRAVEL_MIN: usize = 42;
const META_MIN: usize = 28;

/// T-Union card category from the first metadata byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TUnionCardType {
    Normal,
    Student,
    Elder,
    Test,
    Soldier,
    Unknown(u8),
}

impl TUnionCardType {
    pub const fn from_byte(value: u8) -> Self {
        match value {
            1 => Self::Normal,
            2 => Self::Student,
            3 => Self::Elder,
            4 => Self::Test,
            5 => Self::Soldier,
            other => Self::Unknown(other),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Student => "student",
            Self::Elder => "elder",
            Self::Test => "test",
            Self::Soldier => "soldier",
            Self::Unknown(_) => "unknown",
        }
    }
}

/// Calendar date decoded from BCD card data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl fmt::Display for CardDate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:04}-{:02}-{:02}",
            self.year, self.month, self.day
        )
    }
}

/// Timestamp decoded from BCD card data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardDateTime {
    pub date: CardDate,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl fmt::Display for CardDateTime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} {:02}:{:02}:{:02}",
            self.date, self.hour, self.minute, self.second
        )
    }
}

/// Transaction type from the record's type byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TUnionTransactionKind {
    Load,
    Purchase,
    CompoundPurchase,
    Other(u8),
}

impl TUnionTransactionKind {
    pub const fn from_byte(value: u8) -> Self {
        match value {
            1 | 2 => Self::Load,
            5 | 6 => Self::Purchase,
            9 => Self::CompoundPurchase,
            other => Self::Other(other),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Purchase => "purchase",
            Self::CompoundPurchase => "compound purchase",
            Self::Other(_) => "other",
        }
    }

    pub const fn code(self) -> u8 {
        match self {
            Self::Load => 2,
            Self::Purchase => 6,
            Self::CompoundPurchase => 9,
            Self::Other(value) => value,
        }
    }
}

/// Travel type from the record's type byte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TUnionTravelKind {
    Entry,
    Exit,
    Single,
    Other(u8),
}

impl TUnionTravelKind {
    pub const fn from_byte(value: u8) -> Self {
        match value {
            3 => Self::Entry,
            4 => Self::Exit,
            6 => Self::Single,
            other => Self::Other(other),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::Exit => "exit",
            Self::Single => "single trip",
            Self::Other(_) => "other",
        }
    }

    pub const fn code(self) -> u8 {
        match self {
            Self::Entry => 3,
            Self::Exit => 4,
            Self::Single => 6,
            Self::Other(value) => value,
        }
    }
}

/// One electronic purse transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TUnionTransaction {
    pub sequence: u16,
    pub amount_cents: u32,
    pub kind: TUnionTransactionKind,
    pub terminal_id: String,
    pub timestamp: Option<CardDateTime>,
}

/// One travel record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TUnionTravel {
    pub kind: TUnionTravelKind,
    pub terminal_id: String,
    pub sub_type: u8,
    pub station_id: String,
    pub amount_cents: u32,
    pub balance_cents: u32,
    pub timestamp: Option<CardDateTime>,
    pub city_id: String,
    pub institution_id: String,
}

/// Everything the read-only T-Union profile returns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TUnionCard {
    pub aid: Vec<u8>,
    pub label: String,
    pub app_version: Option<Vec<u8>>,
    pub card_type: TUnionCardType,
    pub city_id: String,
    /// Full card number. Presentation layers call [`Self::masked_card_number`].
    pub card_number: String,
    pub issued: Option<CardDate>,
    pub expires: Option<CardDate>,
    pub balance_cents: u32,
    pub transactions: Vec<TUnionTransaction>,
    pub travels: Vec<TUnionTravel>,
    /// False when a record file stopped early or a record could not be decoded.
    pub complete: bool,
    pub warnings: Vec<String>,
}

impl TUnionCard {
    pub fn aid_hex(&self) -> String {
        hex_upper(&self.aid)
    }

    pub fn masked_card_number(&self) -> String {
        mask_card_number(&self.card_number)
    }

    pub fn app_version_hex(&self) -> Option<String> {
        self.app_version.as_deref().map(hex_upper)
    }
}

/// Read the T-Union application on the selected ISO-DEP card.
///
/// The PPSE is required. An absent T-Union AID returns
/// [`NfcError::Unsupported`]; later status-word failures return an I/O error.
/// A record file that ends with "not found" is a natural end. Any other record
/// failure stops that file, marks the result incomplete, and records a warning.
pub fn read(reader: &mut dyn Reader, cancel: &CancelToken) -> Result<TUnionCard> {
    cancel.check()?;
    let mut warnings = Vec::new();
    let mut directory_label = None;

    let ppse = apdu::select_by_name(reader, PPSE_NAME, "selecting the T-Union PPSE", cancel)?;
    if apdu::is_not_found(ppse.sw) {
        return Err(NfcError::Unsupported(
            "the card does not expose a PPSE directory",
        ));
    }
    apdu::ensure_ok(&ppse, "selecting the T-Union PPSE")?;
    match fci::parse_ppse(&ppse.data) {
        Ok(entries) => {
            if !entries.is_empty() && !fci::contains_aid(&entries, &TUNION_AID) {
                warnings.push("the PPSE does not list the T-Union transit AID".to_owned());
            }
            directory_label = entries
                .iter()
                .find(|entry| entry.aid == TUNION_AID)
                .and_then(|entry| entry.label.clone());
        }
        Err(error) => warnings.push(format!("the PPSE directory could not be parsed: {error}")),
    }

    let selected = apdu::select_by_name(
        reader,
        &TUNION_AID,
        "selecting the T-Union application",
        cancel,
    )?;
    if apdu::is_not_found(selected.sw) {
        return Err(NfcError::Unsupported(
            "the card does not expose the T-Union transit application",
        ));
    }
    apdu::ensure_ok(&selected, "selecting the T-Union application")?;

    let application = fci::parse_application(&selected.data)?;
    if application.df_name != TUNION_AID {
        warnings.push(format!(
            "the selected DF name is {} instead of the T-Union AID",
            hex_upper(&application.df_name)
        ));
    }
    let meta = application
        .meta
        .as_deref()
        .ok_or_else(|| NfcError::Io("the T-Union application has no 9F0C metadata".to_owned()))?;
    let metadata = decode_metadata(meta, &mut warnings)?;

    let balance = apdu::get_balance(reader, "reading the T-Union balance", cancel)?;
    if apdu::is_not_found(balance.sw) {
        return Err(NfcError::Unsupported(
            "the T-Union application does not expose a balance command",
        ));
    }
    apdu::ensure_ok(&balance, "reading the T-Union balance")?;
    if balance.data.len() != 4 {
        return Err(NfcError::Io(format!(
            "the T-Union balance is {} bytes, expected 4",
            balance.data.len()
        )));
    }
    let balance_cents = u32::from_be_bytes([
        balance.data[0],
        balance.data[1],
        balance.data[2],
        balance.data[3],
    ]);

    let (raw_transactions, transactions_complete) = read_records(
        reader,
        TRANSACTION_FILE,
        MAX_TRANSACTIONS,
        "transaction",
        cancel,
        &mut warnings,
    )?;
    let mut complete = transactions_complete;
    let mut transactions = Vec::with_capacity(raw_transactions.len());
    for (index, raw) in raw_transactions.iter().enumerate() {
        match decode_transaction(raw) {
            Ok(transaction) => {
                if transaction.timestamp.is_none() {
                    warnings.push(format!(
                        "transaction record {} has an invalid timestamp",
                        index + 1
                    ));
                }
                transactions.push(transaction);
            }
            Err(error) => {
                warnings.push(format!(
                    "transaction record {} could not be decoded: {error}",
                    index + 1
                ));
                complete = false;
            }
        }
    }

    let (raw_travels, travels_complete) = read_records(
        reader,
        TRAVEL_FILE,
        MAX_TRAVELS,
        "travel",
        cancel,
        &mut warnings,
    )?;
    complete &= travels_complete;
    let mut travels = Vec::with_capacity(raw_travels.len());
    for (index, raw) in raw_travels.iter().enumerate() {
        match decode_travel(raw) {
            Ok(travel) => {
                if travel.timestamp.is_none() {
                    warnings.push(format!(
                        "travel record {} has an invalid timestamp",
                        index + 1
                    ));
                }
                travels.push(travel);
            }
            Err(error) => {
                warnings.push(format!(
                    "travel record {} could not be decoded: {error}",
                    index + 1
                ));
                complete = false;
            }
        }
    }

    Ok(TUnionCard {
        aid: application.df_name,
        label: application
            .label
            .or(directory_label)
            .unwrap_or_else(|| "T-Union".to_owned()),
        app_version: application.version,
        card_type: metadata.card_type,
        city_id: metadata.city_id,
        card_number: metadata.card_number,
        issued: metadata.issued,
        expires: metadata.expires,
        balance_cents,
        transactions,
        travels,
        complete,
        warnings,
    })
}

struct Metadata {
    card_type: TUnionCardType,
    city_id: String,
    card_number: String,
    issued: Option<CardDate>,
    expires: Option<CardDate>,
}

fn decode_metadata(meta: &[u8], warnings: &mut Vec<String>) -> Result<Metadata> {
    if meta.len() < META_MIN {
        return Err(NfcError::Io(format!(
            "the T-Union metadata is {} bytes, expected at least {META_MIN}",
            meta.len()
        )));
    }
    let type_byte = bcd_u8(meta[0]).unwrap_or(meta[0]);
    let city_id = hex_upper(&meta[2..4]);
    let mut card_number = hex_upper(&meta[10..20]);
    if !card_number.is_empty() {
        card_number.remove(0);
    }
    if card_number.len() != 19 {
        warnings.push(format!(
            "the T-Union card number is {} digits, expected 19",
            card_number.len()
        ));
    }
    let issued = decode_date(
        [meta[20], meta[21]],
        meta[22],
        meta[23],
        "issue date",
        warnings,
    );
    let expires = decode_date(
        [meta[24], meta[25]],
        meta[26],
        meta[27],
        "expiry date",
        warnings,
    );
    Ok(Metadata {
        card_type: TUnionCardType::from_byte(type_byte),
        city_id,
        card_number,
        issued,
        expires,
    })
}

fn decode_date(
    year: [u8; 2],
    month: u8,
    day: u8,
    field: &str,
    warnings: &mut Vec<String>,
) -> Option<CardDate> {
    let Some(year) = bcd_u16(year) else {
        warnings.push(format!("the T-Union {field} has an invalid year"));
        return None;
    };
    let Some(month) = bcd_u8(month) else {
        warnings.push(format!("the T-Union {field} has an invalid month"));
        return None;
    };
    let Some(day) = bcd_u8(day) else {
        warnings.push(format!("the T-Union {field} has an invalid day"));
        return None;
    };
    valid_date(year, month, day)
}

fn valid_date(year: u16, month: u8, day: u8) -> Option<CardDate> {
    if !(1900..=2199).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some(CardDate { year, month, day })
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

const fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn read_records(
    reader: &mut dyn Reader,
    file_id: u8,
    maximum: u8,
    label: &str,
    cancel: &CancelToken,
    warnings: &mut Vec<String>,
) -> Result<(Vec<Vec<u8>>, bool)> {
    let mut records = Vec::new();
    for record in 1..=maximum {
        cancel.check()?;
        let response = apdu::read_record(
            reader,
            record,
            file_id,
            &format!("reading {label} record {record}"),
            cancel,
        )?;
        if apdu::is_not_found(response.sw) {
            return Ok((records, true));
        }
        if response.sw == [0x6D, 0x00] {
            warnings.push(format!(
                "the T-Union application does not expose the {label} record file"
            ));
            return Ok((records, false));
        }
        if !response.is_ok() {
            warnings.push(format!(
                "reading {label} record {record} returned status {}",
                apdu::status_hex(response.sw)
            ));
            return Ok((records, false));
        }
        if response.data.iter().all(|byte| *byte == 0) {
            return Ok((records, true));
        }
        records.push(response.data);
    }
    Ok((records, true))
}

fn decode_transaction(data: &[u8]) -> std::result::Result<TUnionTransaction, String> {
    if data.len() < TRANSACTION_MIN {
        return Err(format!(
            "the record is {} bytes, expected at least {TRANSACTION_MIN}",
            data.len()
        ));
    }
    Ok(TUnionTransaction {
        sequence: u16::from_be_bytes([data[0], data[1]]),
        amount_cents: u32::from_be_bytes([data[5], data[6], data[7], data[8]]),
        kind: TUnionTransactionKind::from_byte(data[9]),
        terminal_id: hex_upper(&data[10..16]),
        timestamp: decode_datetime(data, 16),
    })
}

fn decode_travel(data: &[u8]) -> std::result::Result<TUnionTravel, String> {
    if data.len() < TRAVEL_MIN {
        return Err(format!(
            "the record is {} bytes, expected at least {TRAVEL_MIN}",
            data.len()
        ));
    }
    Ok(TUnionTravel {
        kind: TUnionTravelKind::from_byte(data[0]),
        terminal_id: hex_upper(&data[1..9]),
        sub_type: data[9],
        station_id: hex_upper(&data[10..17]),
        amount_cents: u32::from_be_bytes([data[17], data[18], data[19], data[20]]),
        balance_cents: u32::from_be_bytes([data[21], data[22], data[23], data[24]]),
        timestamp: decode_datetime(data, 25),
        city_id: hex_upper(&data[32..34]),
        institution_id: hex_upper(&data[34..42]),
    })
}

fn decode_datetime(data: &[u8], offset: usize) -> Option<CardDateTime> {
    let year = bcd_u16([*data.get(offset)?, *data.get(offset + 1)?])?;
    let month = bcd_u8(*data.get(offset + 2)?)?;
    let day = bcd_u8(*data.get(offset + 3)?)?;
    let hour = bcd_u8(*data.get(offset + 4)?)?;
    let minute = bcd_u8(*data.get(offset + 5)?)?;
    let second = bcd_u8(*data.get(offset + 6)?)?;
    let date = valid_date(year, month, day)?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(CardDateTime {
        date,
        hour,
        minute,
        second,
    })
}

fn bcd_u8(byte: u8) -> Option<u8> {
    let high = byte >> 4;
    let low = byte & 0x0F;
    if high > 9 || low > 9 {
        return None;
    }
    Some(high * 10 + low)
}

fn bcd_u16(bytes: [u8; 2]) -> Option<u16> {
    Some(u16::from(bcd_u8(bytes[0])?) * 100 + u16::from(bcd_u8(bytes[1])?))
}

/// Uppercase hexadecimal without separators.
pub fn hex_upper(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02X}");
    }
    output
}

/// Format a cent amount as Chinese yuan.
pub fn format_cny(cents: u32) -> String {
    format!("¥{}.{:02}", cents / 100, cents % 100)
}

/// Mask every digit except the last four.
pub fn mask_card_number(number: &str) -> String {
    let characters: Vec<char> = number.chars().collect();
    if characters.len() <= 4 {
        return "*".repeat(characters.len());
    }
    let visible = characters.len() - 4;
    let mut output = "*".repeat(visible);
    output.extend(characters[visible..].iter());
    output
}

#[cfg(test)]
mod tests {
    use nfc_core::CancelToken;

    use super::*;
    use crate::mock::{MockTUnionCard, MockTUnionReader};

    fn reader() -> MockTUnionReader {
        MockTUnionReader::new(MockTUnionCard::default())
    }

    #[test]
    fn reads_balance_and_records_from_the_mock_card() {
        let mut reader = reader();
        let card = read(&mut reader, &CancelToken::new()).expect("T-Union read");
        assert_eq!(card.aid, TUNION_AID);
        assert_eq!(card.label, "MOT_T_EP");
        assert_eq!(card.balance_cents, 5000);
        assert_eq!(card.card_type, TUnionCardType::Normal);
        assert_eq!(card.city_id, "0010");
        assert_eq!(card.card_number, "3105000000000000001");
        assert_eq!(card.masked_card_number(), "***************0001");
        assert_eq!(card.app_version.as_deref(), Some([0x02].as_slice()));
        assert_eq!(
            card.issued,
            Some(CardDate {
                year: 2022,
                month: 1,
                day: 2
            })
        );
        assert_eq!(
            card.expires,
            Some(CardDate {
                year: 2072,
                month: 1,
                day: 2
            })
        );
        assert_eq!(card.transactions.len(), 2);
        assert_eq!(card.transactions[0].sequence, 1);
        assert_eq!(card.transactions[0].amount_cents, 500);
        assert_eq!(
            card.transactions[0].kind,
            TUnionTransactionKind::CompoundPurchase
        );
        assert_eq!(card.transactions[0].terminal_id, "413100000001");
        assert_eq!(card.travels.len(), 2);
        assert_eq!(card.travels[0].kind, TUnionTravelKind::Exit);
        assert_eq!(card.travels[0].amount_cents, 200);
        assert_eq!(card.travels[0].balance_cents, 5000);
        assert!(card.complete);
        assert!(card.warnings.is_empty(), "{:?}", card.warnings);
    }

    #[test]
    fn stops_at_the_first_missing_record() {
        let mut card = MockTUnionCard::default();
        card.transactions.truncate(1);
        let mut reader = MockTUnionReader::new(card);
        let result = read(&mut reader, &CancelToken::new()).expect("read");
        assert_eq!(result.transactions.len(), 1);
        assert!(result.complete);
    }

    #[test]
    fn a_protocol_error_marks_the_result_incomplete() {
        let card = MockTUnionCard {
            fail_transaction: Some(2),
            ..MockTUnionCard::default()
        };
        let mut reader = MockTUnionReader::new(card);
        let result = read(&mut reader, &CancelToken::new()).expect("read");
        assert_eq!(result.transactions.len(), 1);
        assert!(!result.complete);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("does not expose the transaction record file"))
        );
    }

    #[test]
    fn missing_application_is_unsupported() {
        let card = MockTUnionCard {
            tunion_present: false,
            ..MockTUnionCard::default()
        };
        let mut reader = MockTUnionReader::new(card);
        let error = read(&mut reader, &CancelToken::new()).expect_err("unsupported");
        assert!(matches!(error, NfcError::Unsupported(_)), "{error}");
    }

    #[test]
    fn malformed_metadata_is_an_io_error() {
        let mut card = MockTUnionCard::default();
        card.meta.truncate(10);
        let mut reader = MockTUnionReader::new(card);
        let error = read(&mut reader, &CancelToken::new()).expect_err("metadata");
        assert!(matches!(error, NfcError::Io(_)), "{error}");
    }

    #[test]
    fn bcd_and_amount_helpers_handle_boundaries() {
        assert_eq!(bcd_u8(0x25), Some(25));
        assert_eq!(bcd_u8(0x2A), None);
        assert_eq!(bcd_u16([0x20, 0x26]), Some(2026));
        assert_eq!(format_cny(0), "¥0.00");
        assert_eq!(format_cny(25376), "¥253.76");
        assert_eq!(mask_card_number("1234"), "****");
        assert_eq!(mask_card_number("123456"), "**3456");
    }

    #[test]
    fn rejects_invalid_calendar_dates() {
        assert!(valid_date(2026, 2, 29).is_none());
        assert!(valid_date(2028, 2, 29).is_some());
        assert!(valid_date(2026, 13, 1).is_none());
        assert!(valid_date(2026, 4, 31).is_none());
    }

    #[test]
    fn transaction_and_travel_decoders_use_reference_offsets() {
        let transaction = [
            0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0xD2, 0x09, 0x41, 0x31, 0x00, 0x00,
            0x00, 0x01, 0x20, 0x26, 0x01, 0x02, 0x03, 0x04, 0x05,
        ];
        let decoded = decode_transaction(&transaction).expect("transaction");
        assert_eq!(decoded.sequence, 7);
        assert_eq!(decoded.amount_cents, 1234);
        assert_eq!(decoded.kind, TUnionTransactionKind::CompoundPurchase);
        assert_eq!(decoded.terminal_id, "413100000001");
        assert_eq!(
            decoded.timestamp.expect("timestamp").to_string(),
            "2026-01-02 03:04:05"
        );

        let mut travel = [0u8; 48];
        travel[0] = 0x04;
        travel[1..9].copy_from_slice(&[0x41, 0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]);
        travel[9] = 0x01;
        travel[10..17].copy_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        travel[17..21].copy_from_slice(&200u32.to_be_bytes());
        travel[21..25].copy_from_slice(&5000u32.to_be_bytes());
        travel[25..32].copy_from_slice(&[0x20, 0x26, 0x01, 0x02, 0x04, 0x05, 0x06]);
        travel[32..34].copy_from_slice(&[0x00, 0x10]);
        travel[34..42].copy_from_slice(&[0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77]);
        let decoded = decode_travel(&travel).expect("travel");
        assert_eq!(decoded.kind, TUnionTravelKind::Exit);
        assert_eq!(decoded.amount_cents, 200);
        assert_eq!(decoded.balance_cents, 5000);
        assert_eq!(decoded.city_id, "0010");
        assert_eq!(
            decoded.timestamp.expect("timestamp").to_string(),
            "2026-01-02 04:05:06"
        );
    }
}
