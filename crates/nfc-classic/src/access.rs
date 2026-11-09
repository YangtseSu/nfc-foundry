use nfc_core::{NfcError, Result};

/// C1/C2/C3 for one block group. The three-bit code is `C1 << 2 | C2 << 1 | C3`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccessCondition {
    pub c1: bool,
    pub c2: bool,
    pub c3: bool,
}

impl AccessCondition {
    pub const fn new(c1: bool, c2: bool, c3: bool) -> Self {
        Self { c1, c2, c3 }
    }

    pub const fn code(self) -> u8 {
        (self.c1 as u8) << 2 | (self.c2 as u8) << 1 | self.c3 as u8
    }

    /// Keys that may read a data block. Manufacturer block 0 is always
    /// readable, and sector trailers have separate trailer permissions.
    pub const fn data_read_keys(self) -> KeyMask {
        match self.code() {
            3 | 5 => KeyMask::B,
            7 => KeyMask::NONE,
            _ => KeyMask::ANY,
        }
    }

    /// Keys that may write a data block.
    pub const fn data_write_keys(self) -> KeyMask {
        match self.code() {
            0 => KeyMask::ANY,
            3 | 4 | 6 => KeyMask::B,
            _ => KeyMask::NONE,
        }
    }

    pub const fn data_increment_keys(self) -> KeyMask {
        match self.code() {
            0 => KeyMask::ANY,
            6 => KeyMask::B,
            _ => KeyMask::NONE,
        }
    }

    pub const fn data_decrement_keys(self) -> KeyMask {
        match self.code() {
            0 | 1 => KeyMask::ANY,
            6 => KeyMask::B,
            _ => KeyMask::NONE,
        }
    }

    /// Whether the card returns Key B bytes as data. Key A is never readable.
    pub const fn key_b_readable(self) -> bool {
        matches!(self.code(), 0..=2)
    }

    /// Keys that may replace the complete trailer in one operation. Conditions
    /// that permit only some trailer fields are rejected because a write always
    /// covers all 16 bytes.
    pub const fn full_trailer_write_keys(self) -> KeyMask {
        match self.code() {
            1 => KeyMask::A,
            3 => KeyMask::B,
            _ => KeyMask::NONE,
        }
    }

    pub const fn trailer_access(self) -> TrailerPermissions {
        let mut permissions = TrailerPermissions {
            read_keys: KeyMask::ANY,
            write_key_a: KeyMask::NONE,
            write_access: KeyMask::NONE,
            write_key_b: KeyMask::NONE,
            key_b_readable: self.key_b_readable(),
        };
        match self.code() {
            0 => {
                permissions.write_key_a = KeyMask::A;
                permissions.write_key_b = KeyMask::A;
            }
            1 => {
                permissions.write_key_a = KeyMask::A;
                permissions.write_access = KeyMask::A;
                permissions.write_key_b = KeyMask::A;
            }
            3 => {
                permissions.write_key_a = KeyMask::B;
                permissions.write_access = KeyMask::B;
                permissions.write_key_b = KeyMask::B;
            }
            4 => {
                permissions.write_key_a = KeyMask::B;
                permissions.write_key_b = KeyMask::B;
            }
            5 => permissions.write_access = KeyMask::B,
            _ => {}
        }
        permissions
    }
}

/// One key-permission set. `ANY` means either Key A or Key B succeeds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KeyMask(u8);

impl KeyMask {
    pub const NONE: Self = Self(0);
    pub const A: Self = Self(1);
    pub const B: Self = Self(2);
    pub const ANY: Self = Self(3);

    pub const fn allows_a(self) -> bool {
        self.0 & Self::A.0 != 0
    }

    pub const fn allows_b(self) -> bool {
        self.0 & Self::B.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// Decoded trailer permissions for one access group.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrailerPermissions {
    pub read_keys: KeyMask,
    pub write_key_a: KeyMask,
    pub write_access: KeyMask,
    pub write_key_b: KeyMask,
    pub key_b_readable: bool,
}

/// Decoded bytes 6..9 of a sector trailer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AccessBits {
    pub groups: [AccessCondition; 4],
}

impl AccessBits {
    /// Decodes the three access bytes and rejects broken inverted copies. The
    /// input is never repaired.
    pub fn decode(bytes: [u8; 3]) -> Result<Self> {
        let [b6, b7, b8] = bytes;
        if (b6 & 0x0F) ^ ((b7 >> 4) & 0x0F) != 0x0F
            || ((b6 >> 4) & 0x0F) ^ (b8 & 0x0F) != 0x0F
            || (b7 & 0x0F) ^ ((b8 >> 4) & 0x0F) != 0x0F
        {
            return Err(NfcError::InvalidArgument(format!(
                "invalid MIFARE Classic access-bit complements: {b6:02X} {b7:02X} {b8:02X}"
            )));
        }
        let mut groups = [AccessCondition::default(); 4];
        for (group, condition) in groups.iter_mut().enumerate() {
            let mask = 1u8 << group;
            *condition = AccessCondition {
                c1: b7 & (mask << 4) != 0,
                c2: b8 & mask != 0,
                c3: b8 & (mask << 4) != 0,
            };
        }
        Ok(Self { groups })
    }

    pub fn encode(self) -> [u8; 3] {
        let mut c1 = 0u8;
        let mut c2 = 0u8;
        let mut c3 = 0u8;
        for (group, condition) in self.groups.iter().enumerate() {
            let mask = 1u8 << group;
            if condition.c1 {
                c1 |= mask;
            }
            if condition.c2 {
                c2 |= mask;
            }
            if condition.c3 {
                c3 |= mask;
            }
        }
        [
            (!c2 & 0x0F) << 4 | (!c1 & 0x0F),
            c1 << 4 | (!c3 & 0x0F),
            c3 << 4 | c2,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn condition_for_code(code: u8) -> AccessCondition {
        AccessCondition::new(code & 4 != 0, code & 2 != 0, code & 1 != 0)
    }

    #[test]
    fn transport_configuration_round_trips() {
        let decoded = AccessBits::decode([0xFF, 0x07, 0x80]).expect("valid access bits");
        for group in 0..3 {
            assert_eq!(decoded.groups[group].code(), 0);
        }
        assert_eq!(decoded.groups[3].code(), 1);
        assert_eq!(decoded.encode(), [0xFF, 0x07, 0x80]);
    }

    #[test]
    fn all_access_condition_combinations_round_trip() {
        for packed in 0..1 << 12 {
            let mut bits = AccessBits::default();
            for (group, condition) in bits.groups.iter_mut().enumerate() {
                *condition = condition_for_code(((packed >> (group * 3)) & 7) as u8);
            }
            let decoded = AccessBits::decode(bits.encode()).expect("encoded access bits");
            assert_eq!(decoded, bits);
        }
    }

    #[test]
    fn broken_complements_are_rejected() {
        for index in 0..3 {
            let mut bytes = [0xFF, 0x07, 0x80];
            bytes[index] ^= 1;
            assert!(AccessBits::decode(bytes).is_err());
        }
    }

    #[test]
    fn data_permissions_follow_the_mifare_table() {
        let cases = [
            (0, KeyMask::ANY, KeyMask::ANY, KeyMask::ANY, KeyMask::ANY),
            (1, KeyMask::ANY, KeyMask::NONE, KeyMask::NONE, KeyMask::ANY),
            (2, KeyMask::ANY, KeyMask::NONE, KeyMask::NONE, KeyMask::NONE),
            (3, KeyMask::B, KeyMask::B, KeyMask::NONE, KeyMask::NONE),
            (4, KeyMask::ANY, KeyMask::B, KeyMask::NONE, KeyMask::NONE),
            (5, KeyMask::B, KeyMask::NONE, KeyMask::NONE, KeyMask::NONE),
            (6, KeyMask::ANY, KeyMask::B, KeyMask::B, KeyMask::B),
            (
                7,
                KeyMask::NONE,
                KeyMask::NONE,
                KeyMask::NONE,
                KeyMask::NONE,
            ),
        ];
        for (code, read, write, increment, decrement) in cases {
            let condition = condition_for_code(code);
            assert_eq!(condition.data_read_keys(), read, "code {code:03b}");
            assert_eq!(condition.data_write_keys(), write, "code {code:03b}");
            assert_eq!(
                condition.data_increment_keys(),
                increment,
                "code {code:03b}"
            );
            assert_eq!(
                condition.data_decrement_keys(),
                decrement,
                "code {code:03b}"
            );
        }
    }

    #[test]
    fn trailer_permissions_follow_the_mifare_table() {
        let cases = [
            (0, KeyMask::A, KeyMask::NONE, KeyMask::A, true),
            (1, KeyMask::A, KeyMask::A, KeyMask::A, true),
            (2, KeyMask::NONE, KeyMask::NONE, KeyMask::NONE, true),
            (3, KeyMask::B, KeyMask::B, KeyMask::B, false),
            (4, KeyMask::B, KeyMask::NONE, KeyMask::B, false),
            (5, KeyMask::NONE, KeyMask::B, KeyMask::NONE, false),
            (6, KeyMask::NONE, KeyMask::NONE, KeyMask::NONE, false),
            (7, KeyMask::NONE, KeyMask::NONE, KeyMask::NONE, false),
        ];
        for (code, write_a, write_access, write_b, key_b_readable) in cases {
            let permissions = condition_for_code(code).trailer_access();
            assert_eq!(permissions.read_keys, KeyMask::ANY);
            assert_eq!(permissions.write_key_a, write_a, "code {code:03b}");
            assert_eq!(permissions.write_access, write_access, "code {code:03b}");
            assert_eq!(permissions.write_key_b, write_b, "code {code:03b}");
            assert_eq!(
                permissions.key_b_readable, key_b_readable,
                "code {code:03b}"
            );
        }
    }

    #[test]
    fn full_trailer_writes_require_the_matching_key() {
        assert_eq!(
            condition_for_code(0).full_trailer_write_keys(),
            KeyMask::NONE
        );
        assert_eq!(condition_for_code(1).full_trailer_write_keys(), KeyMask::A);
        assert_eq!(condition_for_code(3).full_trailer_write_keys(), KeyMask::B);
        assert!(KeyMask::ANY.allows_a());
        assert!(KeyMask::ANY.allows_b());
        assert!(!KeyMask::A.allows_b());
        assert!(KeyMask::NONE.is_empty());
    }
}
