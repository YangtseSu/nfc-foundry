use nfc_core::{BLOCK_SIZE, NfcError, Result};

/// Decoded form of a MIFARE Classic value block. The card stores a signed
/// little-endian value three times and the block address twice, each with a
/// byte-wise inverted copy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ValueBlock {
    pub value: i32,
    pub address: u8,
}

/// Decodes a value block and validates every redundant copy. Malformed data is
/// rejected, never repaired.
pub fn decode(data: [u8; BLOCK_SIZE]) -> Result<ValueBlock> {
    let value = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    if !inverted_copy_matches(&data[0..4], &data[4..8]) {
        return Err(NfcError::InvalidArgument(
            "value block inverted value copy does not match bytes 0-3".to_owned(),
        ));
    }
    if data[0..4] != data[8..12] {
        return Err(NfcError::InvalidArgument(
            "value block repeated value copy does not match bytes 0-3".to_owned(),
        ));
    }
    let address = data[12];
    if data[14] != address {
        return Err(NfcError::InvalidArgument(format!(
            "value block address copies disagree: {:02X} and {:02X}",
            address, data[14]
        )));
    }
    if data[13] != !address {
        return Err(NfcError::InvalidArgument(format!(
            "value block inverted address {:02X} does not match address {address:02X}",
            data[13]
        )));
    }
    if data[15] != !address {
        return Err(NfcError::InvalidArgument(format!(
            "value block second inverted address {:02X} does not match address {address:02X}",
            data[15]
        )));
    }
    Ok(ValueBlock { value, address })
}

/// Produces the canonical 16-byte value block.
pub fn encode(value: ValueBlock) -> [u8; BLOCK_SIZE] {
    let mut data = [0u8; BLOCK_SIZE];
    data[0..4].copy_from_slice(&value.value.to_le_bytes());
    for index in 0..4 {
        data[4 + index] = !data[index];
        data[8 + index] = data[index];
    }
    data[12] = value.address;
    data[13] = !value.address;
    data[14] = value.address;
    data[15] = !value.address;
    data
}

fn inverted_copy_matches(value: &[u8], inverted: &[u8]) -> bool {
    value
        .iter()
        .zip(inverted)
        .all(|(value, inverted)| *inverted == !*value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_value_blocks_match_the_card_format() {
        let cases = [
            (0, 0, "00000000FFFFFFFF0000000000FF00FF"),
            (1, 4, "01000000FEFFFFFF0100000004FB04FB"),
            (100, 5, "640000009BFFFFFF6400000005FA05FA"),
            (-1, 3, "FFFFFFFF00000000FFFFFFFF03FC03FC"),
        ];
        for (value, address, expected) in cases {
            let data = encode(ValueBlock { value, address });
            let hex = data
                .iter()
                .map(|byte| format!("{byte:02X}"))
                .collect::<String>();
            assert_eq!(hex, expected);
            assert_eq!(
                decode(data).expect("canonical value block"),
                ValueBlock { value, address }
            );
        }
    }

    #[test]
    fn inconsistent_copies_are_rejected() {
        let valid = encode(ValueBlock {
            value: 100,
            address: 5,
        });
        type Mutation = fn(&mut [u8; BLOCK_SIZE]);
        let cases: [(&str, Mutation); 5] = [
            ("inverted value", |data| data[4] ^= 0x01),
            ("repeated value", |data| data[8] ^= 0x01),
            ("address copies", |data| data[14] ^= 0x01),
            ("inverted address", |data| data[13] ^= 0x01),
            ("second inverted address", |data| data[15] ^= 0x01),
        ];
        for (needle, mutate) in cases {
            let mut data = valid;
            mutate(&mut data);
            let error = decode(data).expect_err("malformed value block");
            assert!(error.to_string().contains(needle), "{needle}: {error}");
        }
    }

    #[test]
    fn ordinary_data_is_not_a_value_block() {
        let mut data = [0u8; BLOCK_SIZE];
        data[..14].copy_from_slice(b"NFC Foundry 1K");
        assert!(decode(data).is_err());
    }
}
