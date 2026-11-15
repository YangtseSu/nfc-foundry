//! BER-TLV parsing for EMV-style file control information.
//!
//! Tags are returned as big-endian integers (`9F0C` becomes `0x9F0C`). Lengths
//! use the short form or the one- and two-byte long forms; indefinite lengths
//! and lengths beyond two bytes are rejected. A constructed value is not parsed
//! automatically: callers decide which templates they need.

use nfc_core::{NfcError, Result};

/// One BER-TLV object borrowed from the response buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tlv<'a> {
    pub tag: u32,
    pub constructed: bool,
    pub value: &'a [u8],
}

impl Tlv<'_> {
    /// Uppercase tag with an even number of digits.
    pub fn tag_hex(self) -> String {
        if self.tag <= 0xFF {
            format!("{:02X}", self.tag)
        } else if self.tag <= 0xFFFF {
            format!("{:04X}", self.tag)
        } else {
            format!("{:06X}", self.tag)
        }
    }

    /// Parse this object's value as a nested TLV list.
    pub fn children(&self) -> Result<Vec<Tlv<'_>>> {
        parse(self.value)
    }
}

/// Parse a complete TLV buffer.
pub fn parse(data: &[u8]) -> Result<Vec<Tlv<'_>>> {
    let mut objects = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let (object, next) = parse_at(data, offset)?;
        objects.push(object);
        offset = next;
    }
    Ok(objects)
}

/// Parse as many objects as possible and return the first failure.
///
/// Some Chinese transit cards encode a label length one byte short, so the
/// final label character becomes the next tag byte. Callers that only need the
/// objects before the malformed tail use this instead of [`parse`].
pub fn parse_lossy(data: &[u8]) -> (Vec<Tlv<'_>>, Option<NfcError>) {
    let mut objects = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        match parse_at(data, offset) {
            Ok((object, next)) => {
                objects.push(object);
                offset = next;
            }
            Err(error) => return (objects, Some(error)),
        }
    }
    (objects, None)
}

fn parse_at(data: &[u8], offset: usize) -> Result<(Tlv<'_>, usize)> {
    let (tag, constructed, next) = parse_tag(data, offset)?;
    let (length, next) = parse_length(data, next)?;
    let end = next
        .checked_add(length)
        .ok_or_else(|| malformed(offset, "length overflows the buffer"))?;
    if end > data.len() {
        return Err(malformed(
            offset,
            &format!("value length {length} exceeds the remaining buffer"),
        ));
    }
    Ok((
        Tlv {
            tag,
            constructed,
            value: &data[next..end],
        },
        end,
    ))
}

/// Find the first object with `tag`.
pub fn find<'a, 'b>(objects: &'b [Tlv<'a>], tag: u32) -> Option<&'b Tlv<'a>> {
    objects.iter().find(|object| object.tag == tag)
}

/// Collect every object, including objects nested in constructed values.
///
/// The returned values are owned copies so the caller can keep them after the
/// response buffer is released. Nesting deeper than [`MAX_DEPTH`] is rejected.
pub fn collect(data: &[u8]) -> Result<Vec<(u32, Vec<u8>)>> {
    let mut output = Vec::new();
    collect_into(data, 0, &mut output)?;
    Ok(output)
}

/// Collect every object that parses and return the first failure.
///
/// Objects before a malformed tail and inside templates parsed before the
/// failure are retained. Callers that need a required field can still use the
/// partial list.
pub fn collect_lossy(data: &[u8]) -> (Vec<(u32, Vec<u8>)>, Option<NfcError>) {
    let mut output = Vec::new();
    let error = collect_into_lossy(data, 0, &mut output);
    (output, error)
}

/// Maximum accepted TLV nesting. Real FCI structures use at most four levels.
pub const MAX_DEPTH: usize = 16;

fn collect_into(data: &[u8], depth: usize, output: &mut Vec<(u32, Vec<u8>)>) -> Result<()> {
    if depth > MAX_DEPTH {
        return Err(malformed(0, "TLV nesting is too deep"));
    }
    for object in parse(data)? {
        output.push((object.tag, object.value.to_vec()));
        if object.constructed {
            collect_into(object.value, depth + 1, output)?;
        }
    }
    Ok(())
}

fn collect_into_lossy(
    data: &[u8],
    depth: usize,
    output: &mut Vec<(u32, Vec<u8>)>,
) -> Option<NfcError> {
    if depth > MAX_DEPTH {
        return Some(malformed(0, "TLV nesting is too deep"));
    }
    let (objects, error) = parse_lossy(data);
    for object in objects {
        output.push((object.tag, object.value.to_vec()));
        if object.constructed
            && let Some(nested) = collect_into_lossy(object.value, depth + 1, output)
        {
            return Some(nested);
        }
    }
    error
}

fn parse_tag(data: &[u8], offset: usize) -> Result<(u32, bool, usize)> {
    let first = *data
        .get(offset)
        .ok_or_else(|| malformed(offset, "tag is truncated"))?;
    let constructed = first & 0x20 != 0;
    let mut tag = u32::from(first);
    if first & 0x1F != 0x1F {
        return Ok((tag, constructed, offset + 1));
    }
    let mut position = offset + 1;
    let mut count = 1;
    loop {
        let byte = *data
            .get(position)
            .ok_or_else(|| malformed(offset, "tag is truncated"))?;
        tag = (tag << 8) | u32::from(byte);
        position += 1;
        count += 1;
        if byte & 0x80 == 0 {
            break;
        }
        if count >= 4 {
            return Err(malformed(offset, "tag is longer than four bytes"));
        }
    }
    Ok((tag, constructed, position))
}

fn parse_length(data: &[u8], offset: usize) -> Result<(usize, usize)> {
    let first = *data
        .get(offset)
        .ok_or_else(|| malformed(offset, "length is missing"))?;
    match first {
        0x00..=0x7F => Ok((usize::from(first), offset + 1)),
        0x81 => {
            let value = *data
                .get(offset + 1)
                .ok_or_else(|| malformed(offset, "one-byte long length is truncated"))?;
            Ok((usize::from(value), offset + 2))
        }
        0x82 => {
            let high = *data
                .get(offset + 1)
                .ok_or_else(|| malformed(offset, "two-byte long length is truncated"))?;
            let low = *data
                .get(offset + 2)
                .ok_or_else(|| malformed(offset, "two-byte long length is truncated"))?;
            Ok((usize::from(u16::from_be_bytes([high, low])), offset + 3))
        }
        0x80 => Err(malformed(offset, "indefinite lengths are not supported")),
        _ => Err(malformed(
            offset,
            "lengths longer than two bytes are not supported",
        )),
    }
}

fn malformed(offset: usize, reason: &str) -> NfcError {
    NfcError::Io(format!("malformed TLV at byte {offset}: {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nested_constructed_objects() {
        let data = [
            0x6F, 0x0C, 0x84, 0x02, 0xA0, 0x01, 0xA5, 0x06, 0x50, 0x04, b'T', b'e', b's', b't',
        ];
        let outer = parse(&data).expect("outer TLV");
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].tag, 0x6F);
        assert!(outer[0].constructed);
        let inner = outer[0].children().expect("inner TLV");
        assert_eq!(find(&inner, 0x84).expect("DF name").value, &[0xA0, 0x01]);
        let a5 = find(&inner, 0xA5).expect("proprietary template");
        let label = a5.children().expect("label TLV");
        assert_eq!(find(&label, 0x50).expect("label").value, b"Test");
    }

    #[test]
    fn parses_two_byte_tags_and_long_lengths() {
        let mut data = vec![0x9F, 0x0C, 0x81, 0x03, 0x01, 0x02, 0x03];
        data.extend_from_slice(&[0xBF, 0x0C, 0x82, 0x00, 0x02, 0xAA, 0xBB]);
        let objects = parse(&data).expect("TLV");
        assert_eq!(objects[0].tag, 0x9F0C);
        assert_eq!(objects[0].value, &[1, 2, 3]);
        assert_eq!(objects[1].tag, 0xBF0C);
        assert!(objects[1].constructed);
        assert_eq!(objects[1].value, &[0xAA, 0xBB]);
    }

    #[test]
    fn rejects_truncated_and_indefinite_values() {
        assert!(parse(&[0x6F, 0x05, 0x84]).is_err());
        assert!(parse(&[0x6F, 0x80]).is_err());
        assert!(parse(&[0x9F, 0x0C, 0x03, 0x01]).is_err());
    }

    #[test]
    fn collect_includes_nested_values() {
        let data = [0x6F, 0x06, 0xA5, 0x04, 0x50, 0x02, b'O', b'K'];
        let objects = collect(&data).expect("collect");
        assert!(objects.iter().any(|(tag, _)| *tag == 0x6F));
        assert!(
            objects
                .iter()
                .any(|(tag, value)| *tag == 0x50 && value == b"OK")
        );
    }

    #[test]
    fn tag_hex_uses_even_width() {
        let object = Tlv {
            tag: 0x9F0C,
            constructed: false,
            value: &[],
        };
        assert_eq!(object.tag_hex(), "9F0C");
        let object = Tlv {
            tag: 0x6F,
            constructed: true,
            value: &[],
        };
        assert_eq!(object.tag_hex(), "6F");
    }
}
