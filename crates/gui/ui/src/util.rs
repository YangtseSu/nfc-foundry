//! Small parsing and file helpers shared by the workspace crates.

use std::fs;
use std::path::Path;

use nfc_core::{NfcError, Result};

/// Parse a fixed-size hexadecimal value, ignoring spaces and colons.
pub fn parse_hex_array<const N: usize>(value: &str) -> Result<[u8; N]> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if compact.len() != N * 2 {
        return Err(NfcError::InvalidArgument(format!(
            "expected {} hexadecimal digits",
            N * 2
        )));
    }
    let bytes = compact.as_bytes();
    let mut output = [0u8; N];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = (hex_digit(bytes[index * 2])? << 4) | hex_digit(bytes[index * 2 + 1])?;
    }
    Ok(output)
}

/// Parse an arbitrary-length hexadecimal value, ignoring spaces and colons.
pub fn parse_hex_bytes(value: &str) -> Result<Vec<u8>> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_ascii_whitespace() && *character != ':')
        .collect();
    if !compact.len().is_multiple_of(2) {
        return Err(NfcError::InvalidArgument(
            "the hexadecimal value has an odd number of digits".to_owned(),
        ));
    }
    let bytes = compact.as_bytes();
    let mut output = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks(2) {
        output.push((hex_digit(chunk[0])? << 4) | hex_digit(chunk[1])?);
    }
    Ok(output)
}

/// Parse a comma/space separated page list with `first-last` ranges.
pub fn parse_page_list(value: &str) -> Result<Vec<u8>> {
    let mut pages = Vec::new();
    for part in value.split([',', ' ']).filter(|part| !part.is_empty()) {
        if let Some((first, last)) = part.split_once('-') {
            let first: u8 = first
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {first:?}")))?;
            let last: u8 = last
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {last:?}")))?;
            if first > last {
                return Err(NfcError::InvalidArgument(format!(
                    "page range {part:?} is reversed"
                )));
            }
            pages.extend(first..=last);
        } else {
            let page: u8 = part
                .parse()
                .map_err(|_| NfcError::InvalidArgument(format!("invalid page {part:?}")))?;
            pages.push(page);
        }
    }
    pages.sort_unstable();
    pages.dedup();
    Ok(pages)
}

/// Write a user file with `0600` permissions and create its parent directory.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|error| {
            NfcError::Io(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    fs::write(path, bytes)
        .map_err(|error| NfcError::Io(format!("could not write {}: {error}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let permissions = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, permissions).map_err(|error| {
            NfcError::Io(format!(
                "could not restrict permissions on {}: {error}",
                path.display()
            ))
        })?;
    }
    Ok(())
}

fn hex_digit(character: u8) -> Result<u8> {
    match character {
        b'0'..=b'9' => Ok(character - b'0'),
        b'a'..=b'f' => Ok(character - b'a' + 10),
        b'A'..=b'F' => Ok(character - b'A' + 10),
        _ => Err(NfcError::InvalidArgument(
            "the value contains a non-hexadecimal character".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_hex_ignores_separators() {
        assert_eq!(
            parse_hex_array::<4>("04 A2:24 8A").expect("hex"),
            [0x04, 0xA2, 0x24, 0x8A]
        );
        assert!(parse_hex_array::<4>("04A2").is_err());
        assert!(parse_hex_array::<4>("04A2ZZ8A00").is_err());
    }

    #[test]
    fn byte_hex_accepts_even_lengths_only() {
        assert_eq!(parse_hex_bytes("A0 A1").expect("hex"), vec![0xA0, 0xA1]);
        assert!(parse_hex_bytes("A0A").is_err());
    }

    #[test]
    fn page_lists_sort_and_deduplicate_ranges() {
        assert_eq!(
            parse_page_list("8, 4-6, 5").expect("pages"),
            vec![4, 5, 6, 8]
        );
        assert!(parse_page_list("9-4").is_err());
        assert!(parse_page_list("x").is_err());
    }
}
