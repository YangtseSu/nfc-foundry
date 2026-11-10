//! NDEF message codec, chunk reassembly, and capacity checks.

use nfc_core::{NfcError, Result};

use crate::record::{NdefRecord, Tnf};

const FLAG_MB: u8 = 0x80;
const FLAG_ME: u8 = 0x40;
const FLAG_CF: u8 = 0x20;
const FLAG_SR: u8 = 0x10;
const FLAG_IL: u8 = 0x08;
const TNF_MASK: u8 = 0x07;

/// A logical NDEF message: one or more records in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NdefMessage {
    records: Vec<NdefRecord>,
}

impl NdefMessage {
    /// Create a message from logical records. The message must not be empty and
    /// every record must be valid.
    pub fn new(records: Vec<NdefRecord>) -> Result<Self> {
        let message = Self { records };
        message.validate()?;
        Ok(message)
    }

    /// Create a single-record message.
    pub fn single(record: NdefRecord) -> Result<Self> {
        Self::new(vec![record])
    }

    /// The logical records, with chunked payloads reassembled.
    pub fn records(&self) -> &[NdefRecord] {
        &self.records
    }

    /// Validate the message and every record.
    pub fn validate(&self) -> Result<()> {
        if self.records.is_empty() {
            return Err(NfcError::InvalidArgument(
                "an NDEF message must contain at least one record".to_owned(),
            ));
        }
        for (index, record) in self.records.iter().enumerate() {
            record
                .validate()
                .map_err(|error| context(index, error.to_string()))?;
        }
        Ok(())
    }

    /// Return the exact encoded size in octets.
    pub fn encoded_len(&self) -> Result<usize> {
        self.validate()?;
        let mut total: u128 = 0;
        for record in &self.records {
            let id_field = usize::from(!record.id().is_empty());
            let length_field = if record.payload().len() <= usize::from(u8::MAX) {
                1
            } else {
                4
            };
            total += (1
                + 1
                + length_field
                + id_field
                + record.record_type().len()
                + record.id().len()
                + record.payload().len()) as u128;
        }
        usize::try_from(total).map_err(|_| {
            NfcError::InvalidArgument("NDEF message is too large for this platform".to_owned())
        })
    }

    /// Check that the encoded message fits in `capacity` octets.
    pub fn fits_in(&self, capacity: usize) -> Result<()> {
        let needed = self.encoded_len()?;
        if needed > capacity {
            return Err(NfcError::InvalidArgument(format!(
                "NDEF message needs {needed} octets but only {capacity} are available"
            )));
        }
        Ok(())
    }

    /// Encode the message with one physical record per logical record.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut output = Vec::with_capacity(self.encoded_len()?);
        let last = self.records.len() - 1;
        for (index, record) in self.records.iter().enumerate() {
            encode_record(&mut output, record, index == 0, index == last);
        }
        Ok(output)
    }

    /// Encode the message, splitting each payload into chunks of at most
    /// `max_chunk` octets. Payloads that already fit stay in one record.
    pub fn encode_chunked(&self, max_chunk: usize) -> Result<Vec<u8>> {
        self.validate()?;
        if max_chunk == 0 {
            return Err(NfcError::InvalidArgument(
                "NDEF chunk size must be at least one octet".to_owned(),
            ));
        }
        let mut output = Vec::new();
        let last = self.records.len() - 1;
        for (index, record) in self.records.iter().enumerate() {
            let payload = record.payload();
            if payload.len() <= max_chunk {
                encode_record(&mut output, record, index == 0, index == last);
                continue;
            }
            let chunks = payload.chunks(max_chunk);
            let chunk_count = chunks.len();
            for (chunk_index, chunk) in chunks.enumerate() {
                let first = chunk_index == 0;
                let final_chunk = chunk_index + 1 == chunk_count;
                let flags = chunk_flags(
                    index == 0 && first,
                    index == last && final_chunk,
                    !final_chunk,
                );
                if first {
                    encode_raw(
                        &mut output,
                        flags,
                        record.tnf(),
                        record.record_type(),
                        record.id(),
                        chunk,
                    );
                } else {
                    encode_raw(&mut output, flags, Tnf::Unchanged, &[], &[], chunk);
                }
            }
        }
        Ok(output)
    }

    /// Decode one complete NDEF message and reject trailing octets.
    pub fn decode(input: &[u8]) -> Result<Self> {
        let (message, consumed) = Self::decode_prefix(input)?;
        if consumed != input.len() {
            return Err(NfcError::InvalidArgument(format!(
                "NDEF message ends at byte {consumed} but {} trailing octets remain",
                input.len() - consumed
            )));
        }
        Ok(message)
    }

    /// Decode an NDEF message and return the message plus the number of octets
    /// it occupied. The message ends at the record with the ME flag.
    pub fn decode_prefix(input: &[u8]) -> Result<(Self, usize)> {
        let mut offset = 0;
        let mut index = 0;
        let mut records = Vec::new();
        let mut chunk: Option<PendingChunk> = None;

        loop {
            if offset >= input.len() {
                return Err(NfcError::InvalidArgument(if index == 0 {
                    "NDEF message is empty".to_owned()
                } else {
                    format!("NDEF message at byte 0 ends after {index} records without ME")
                }));
            }

            let (raw, consumed) = RawRecord::parse(&input[offset..], offset, index)?;
            let cf = raw.cf();
            let me = raw.me();

            if index == 0 && !raw.mb() {
                return Err(context_at(
                    index,
                    offset,
                    "the first record does not set the MB flag",
                ));
            }
            if index > 0 && raw.mb() {
                return Err(context_at(
                    index,
                    offset,
                    "the MB flag is only valid on the first record",
                ));
            }
            if cf && me {
                return Err(context_at(
                    index,
                    offset,
                    "a chunk record must not set the ME flag",
                ));
            }

            if raw.tnf == Tnf::Unchanged {
                let Some(pending) = chunk.as_mut() else {
                    return Err(context_at(
                        index,
                        offset,
                        "Unchanged TNF appears outside a chunked payload",
                    ));
                };
                pending.payload().extend_from_slice(&raw.payload);
                if pending.payload().len() > crate::record::MAX_PAYLOAD_LENGTH {
                    return Err(context_at(
                        index,
                        offset,
                        "reassembled chunked payload exceeds the 32-bit payload limit",
                    ));
                }
                if !cf && let Some(pending) = chunk.take() {
                    records.push(
                        pending
                            .finish()
                            .map_err(|error| context_at(index, offset, error.to_string()))?,
                    );
                }
            } else {
                if chunk.is_some() {
                    return Err(context_at(
                        index,
                        offset,
                        "a new record starts before the chunked payload is terminated",
                    ));
                }
                if cf {
                    chunk = Some(PendingChunk::new(
                        raw.tnf,
                        raw.record_type,
                        raw.id,
                        raw.payload,
                    ));
                } else {
                    records.push(NdefRecord::from_decoded(
                        raw.tnf,
                        raw.record_type,
                        raw.id,
                        raw.payload,
                    ));
                }
            }

            if me {
                if chunk.is_some() {
                    return Err(context_at(
                        index,
                        offset,
                        "the message ends inside a chunked payload",
                    ));
                }
                if records.is_empty() {
                    return Err(context_at(
                        index,
                        offset,
                        "the message contains no logical record",
                    ));
                }
                return Ok((Self { records }, offset + consumed));
            }

            offset += consumed;
            index += 1;
        }
    }
}

struct PendingChunk {
    tnf: Tnf,
    record_type: Vec<u8>,
    id: Vec<u8>,
    payload: Vec<u8>,
}

impl PendingChunk {
    fn new(tnf: Tnf, record_type: Vec<u8>, id: Vec<u8>, payload: Vec<u8>) -> Self {
        Self {
            tnf,
            record_type,
            id,
            payload,
        }
    }

    fn payload(&mut self) -> &mut Vec<u8> {
        &mut self.payload
    }

    fn finish(self) -> Result<NdefRecord> {
        let record = NdefRecord::from_decoded(self.tnf, self.record_type, self.id, self.payload);
        record.validate()?;
        Ok(record)
    }
}

struct RawRecord {
    flags: u8,
    tnf: Tnf,
    record_type: Vec<u8>,
    id: Vec<u8>,
    payload: Vec<u8>,
}

impl RawRecord {
    fn parse(input: &[u8], base: usize, index: usize) -> Result<(Self, usize)> {
        let header = *input
            .first()
            .ok_or_else(|| context_at(index, base, "record header is missing"))?;
        let tnf = Tnf::from_bits(header & TNF_MASK)
            .map_err(|error| context_at(index, base, error.to_string()))?;
        let sr = header & FLAG_SR != 0;
        let il = header & FLAG_IL != 0;

        let mut cursor = 1;
        let type_length = read_u8(input, &mut cursor, base, index, "TYPE_LENGTH")? as usize;
        let payload_length = if sr {
            u32::from(read_u8(
                input,
                &mut cursor,
                base,
                index,
                "short PAYLOAD_LENGTH",
            )?) as usize
        } else {
            let bytes = read_array::<4>(input, &mut cursor, base, index, "PAYLOAD_LENGTH")?;
            u32::from_be_bytes(bytes) as usize
        };
        let id_length = if il {
            read_u8(input, &mut cursor, base, index, "ID_LENGTH")? as usize
        } else {
            0
        };

        validate_header_fields(tnf, il, type_length, id_length, payload_length)
            .map_err(|message| context_at(index, base, message))?;

        let total = cursor
            .checked_add(type_length)
            .and_then(|value| value.checked_add(id_length))
            .and_then(|value| value.checked_add(payload_length))
            .ok_or_else(|| context_at(index, base, "record length overflows"))?;
        if total > input.len() {
            return Err(context_at(
                index,
                base,
                format!(
                    "record declares {total} octets but only {} are available",
                    input.len()
                ),
            ));
        }

        let record_type = input[cursor..cursor + type_length].to_vec();
        cursor += type_length;
        let id = input[cursor..cursor + id_length].to_vec();
        cursor += id_length;
        let payload = input[cursor..cursor + payload_length].to_vec();

        Ok((
            Self {
                flags: header,
                tnf,
                record_type,
                id,
                payload,
            },
            total,
        ))
    }

    const fn mb(&self) -> bool {
        self.flags & FLAG_MB != 0
    }

    const fn me(&self) -> bool {
        self.flags & FLAG_ME != 0
    }

    const fn cf(&self) -> bool {
        self.flags & FLAG_CF != 0
    }
}

fn validate_header_fields(
    tnf: Tnf,
    il: bool,
    type_length: usize,
    id_length: usize,
    payload_length: usize,
) -> std::result::Result<(), String> {
    match tnf {
        Tnf::Empty => {
            if type_length != 0 || id_length != 0 || payload_length != 0 {
                return Err(
                    "Empty TNF requires TYPE_LENGTH, ID_LENGTH, and PAYLOAD_LENGTH to be zero"
                        .to_owned(),
                );
            }
        }
        Tnf::Unknown => {
            if type_length != 0 {
                return Err("Unknown TNF requires TYPE_LENGTH to be zero".to_owned());
            }
        }
        Tnf::Unchanged => {
            if type_length != 0 {
                return Err("Unchanged TNF requires TYPE_LENGTH to be zero".to_owned());
            }
            if il || id_length != 0 {
                return Err("chunk continuation records must not carry an ID".to_owned());
            }
        }
        Tnf::WellKnown | Tnf::Media | Tnf::AbsoluteUri | Tnf::External => {
            if type_length == 0 {
                return Err(format!("{tnf:?} TNF requires a non-empty TYPE field"));
            }
        }
    }
    Ok(())
}

fn read_u8(input: &[u8], cursor: &mut usize, base: usize, index: usize, field: &str) -> Result<u8> {
    let value = *input.get(*cursor).ok_or_else(|| {
        context_at(
            index,
            base,
            format!(
                "record is truncated before {field} at byte {}",
                base + *cursor
            ),
        )
    })?;
    *cursor += 1;
    Ok(value)
}

fn read_array<const N: usize>(
    input: &[u8],
    cursor: &mut usize,
    base: usize,
    index: usize,
    field: &str,
) -> Result<[u8; N]> {
    let end = cursor
        .checked_add(N)
        .ok_or_else(|| context_at(index, base, "record length overflows"))?;
    let slice = input.get(*cursor..end).ok_or_else(|| {
        context_at(
            index,
            base,
            format!(
                "record is truncated inside {field} at byte {}",
                base + *cursor
            ),
        )
    })?;
    let mut bytes = [0u8; N];
    bytes.copy_from_slice(slice);
    *cursor = end;
    Ok(bytes)
}

fn chunk_flags(mb: bool, me: bool, cf: bool) -> u8 {
    let mut flags = 0;
    if mb {
        flags |= FLAG_MB;
    }
    if me {
        flags |= FLAG_ME;
    }
    if cf {
        flags |= FLAG_CF;
    }
    flags
}

fn encode_record(output: &mut Vec<u8>, record: &NdefRecord, mb: bool, me: bool) {
    encode_raw(
        output,
        chunk_flags(mb, me, false),
        record.tnf(),
        record.record_type(),
        record.id(),
        record.payload(),
    );
}

fn encode_raw(
    output: &mut Vec<u8>,
    flags: u8,
    tnf: Tnf,
    record_type: &[u8],
    id: &[u8],
    payload: &[u8],
) {
    let short = payload.len() <= usize::from(u8::MAX);
    let has_id = !id.is_empty();
    let mut header = flags | tnf.bits();
    if short {
        header |= FLAG_SR;
    }
    if has_id {
        header |= FLAG_IL;
    }
    output.push(header);
    output.push(record_type.len() as u8);
    if short {
        output.push(payload.len() as u8);
    } else {
        output.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    }
    if has_id {
        output.push(id.len() as u8);
    }
    output.extend_from_slice(record_type);
    output.extend_from_slice(id);
    output.extend_from_slice(payload);
}

fn context(index: usize, message: impl Into<String>) -> NfcError {
    NfcError::InvalidArgument(format!("NDEF record {index}: {}", message.into()))
}

fn context_at(index: usize, offset: usize, message: impl Into<String>) -> NfcError {
    NfcError::InvalidArgument(format!(
        "NDEF record {index} at byte {offset}: {}",
        message.into()
    ))
}
