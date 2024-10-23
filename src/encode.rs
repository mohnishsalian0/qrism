use crate::types::{ECLevel, QRError, QRResult, Version};
use std::{
    cmp::{min, Ordering},
    mem::swap,
};

// Mode
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Mode {
    Numeric = 0b0001,
    Alphanumeric = 0b0010,
    Byte = 0b0100,
}

impl PartialOrd for Mode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Mode {
    fn cmp(&self, other: &Self) -> Ordering {
        match (*self, *other) {
            (a, b) if a == b => Ordering::Equal,
            (Self::Numeric, _) | (_, Self::Byte) => Ordering::Less,
            (_, Self::Numeric) | (Self::Byte, _) => Ordering::Greater,
            _ => unreachable!(),
        }
    }
}

impl Mode {
    #[inline]
    fn numeric_digit(char: u8) -> u16 {
        debug_assert!(Mode::Numeric.contains(char), "Invalid numeric data: {char}");
        (char - b'0') as u16
    }

    #[inline]
    fn alphanumeric_digit(char: u8) -> u16 {
        debug_assert!(Mode::Alphanumeric.contains(char), "Invalid alphanumeric data: {char}");
        match char {
            b'0'..=b'9' => (char - b'0') as u16,
            b'A'..=b'Z' => (char - b'A' + 10) as u16,
            b' ' => 36,
            b'$' => 37,
            b'%' => 38,
            b'*' => 39,
            b'+' => 40,
            b'-' => 41,
            b'.' => 42,
            b'/' => 43,
            b':' => 44,
            _ => unreachable!("Invalid alphanumeric {char}"),
        }
    }

    pub fn from(&self, data: &[u8]) -> u16 {
        let len = data.len();
        match self {
            Self::Numeric => {
                debug_assert!(len <= 3, "Data is too long for numeric conversion: {len}");
                data.iter().fold(0_u16, |n, b| n * 10 + Self::numeric_digit(*b))
            }
            Self::Alphanumeric => {
                debug_assert!(len <= 2, "Data is too long for alphanumeric conversion: {len}");
                data.iter().fold(0_u16, |n, b| n * 45 + Self::alphanumeric_digit(*b))
            }
            Self::Byte => {
                debug_assert!(len == 1, "Data is too long for byte conversion: {len}");
                data[0] as u16
            }
        }
    }

    pub fn contains(&self, byte: u8) -> bool {
        match self {
            Self::Numeric => byte.is_ascii_digit(),
            Self::Alphanumeric => {
                matches!(byte, b'0'..=b'9' | b'A'..=b'Z' | b' ' | b'$' | b'%' | b'*' | b'+' | b'-' | b'.' | b'/' | b':')
            }
            Self::Byte => true,
        }
    }

    pub fn char_count_bit_len(&self, version: Version) -> usize {
        debug_assert!(
            matches!(version, Version::Micro(1..=4) | Version::Normal(1..=40)),
            "Invalid version"
        );

        match version {
            Version::Micro(v) => match *self {
                Self::Numeric => v + 2,
                Self::Alphanumeric => v + 1,
                Self::Byte => v + 1,
            },
            Version::Normal(1..=9) => match *self {
                Self::Numeric => 10,
                Self::Alphanumeric => 9,
                Self::Byte => 8,
            },
            Version::Normal(10..=26) => match *self {
                Self::Numeric => 12,
                Self::Alphanumeric => 11,
                Self::Byte => 16,
            },
            Version::Normal(_) => match *self {
                Self::Numeric => 14,
                Self::Alphanumeric => 13,
                Self::Byte => 16,
            },
        }
    }

    pub fn encoded_len(&self, len: usize) -> usize {
        match *self {
            Self::Numeric => (len * 10 + 2) / 3,
            Self::Alphanumeric => (len * 11 + 1) / 2,
            Self::Byte => len * 8,
        }
    }
}

#[cfg(test)]
mod mode_tests {
    use crate::encode::Mode;
    use crate::types::Version;

    use super::Mode::*;
    use super::Version::*;

    #[test]
    #[should_panic]
    fn test_char_count_bit_len_invalid_version_low() {
        Numeric.char_count_bit_len(Normal(0));
    }

    #[test]
    #[should_panic]
    fn test_char_count_bit_len_invalid_version_high() {
        Alphanumeric.char_count_bit_len(Normal(41));
    }

    #[test]
    #[should_panic]
    fn test_char_count_bit_len_invalid_version_max() {
        Alphanumeric.char_count_bit_len(Normal(usize::MAX));
    }

    #[test]
    fn test_comparison() {
        assert!(Numeric == Numeric);
        assert!(Numeric < Alphanumeric);
        assert!(Numeric < Byte);
        assert!(Alphanumeric == Alphanumeric);
        assert!(Alphanumeric < Byte);
        assert!(Byte == Byte);
    }

    #[test]
    fn test_numeric_digit() {
        assert_eq!(Mode::numeric_digit(b'0'), 0);
        assert_eq!(Mode::numeric_digit(b'9'), 9);
    }

    #[test]
    #[should_panic]
    fn test_invalid_numeric_digit() {
        Mode::numeric_digit(b'A');
    }

    #[test]
    fn test_alphanumeric_digit() {
        assert_eq!(Mode::alphanumeric_digit(b'0'), 0);
        assert_eq!(Mode::alphanumeric_digit(b'9'), 9);
        assert_eq!(Mode::alphanumeric_digit(b'A'), 10);
        assert_eq!(Mode::alphanumeric_digit(b'Z'), 35);
        assert_eq!(Mode::alphanumeric_digit(b' '), 36);
        assert_eq!(Mode::alphanumeric_digit(b':'), 44);
    }

    #[test]
    #[should_panic]
    fn test_invalid_alphanumeric_digit() {
        Mode::alphanumeric_digit(b'a');
    }

    #[test]
    fn test_numeric_conversion() {
        assert_eq!(Mode::Numeric.from("012".as_bytes()), 0b0000001100);
        assert_eq!(Mode::Numeric.from("345".as_bytes()), 0b0101011001);
        assert_eq!(Mode::Numeric.from("901".as_bytes()), 0b1110000101);
        assert_eq!(Mode::Numeric.from("67".as_bytes()), 0b1000011);
        assert_eq!(Mode::Numeric.from("8".as_bytes()), 0b1000);
    }

    #[test]
    #[should_panic]
    fn test_invalid_numeric_conversion() {
        Mode::Numeric.from("1234".as_bytes());
    }

    #[test]
    fn test_alphanumeric_conversion() {
        assert_eq!(Mode::Alphanumeric.from("AC".as_bytes()), 0b00111001110);
        assert_eq!(Mode::Alphanumeric.from("-4".as_bytes()), 0b11100111001);
        assert_eq!(Mode::Alphanumeric.from("2".as_bytes()), 0b000010);
    }

    #[test]
    #[should_panic]
    fn test_invalid_alphanumeric_conversion() {
        Mode::Alphanumeric.from("1234".as_bytes());
    }

    #[test]
    fn test_is_numeric() {
        assert!(Mode::Numeric.contains(b'0'));
        assert!(Mode::Numeric.contains(b'9'));
        assert!(!Mode::Numeric.contains(b'A'));
        assert!(!Mode::Numeric.contains(b'Z'));
        assert!(!Mode::Numeric.contains(b' '));
        assert!(!Mode::Numeric.contains(b':'));
    }

    #[test]
    fn test_is_alphanumeric() {
        assert!(Mode::Alphanumeric.contains(b'0'));
        assert!(Mode::Alphanumeric.contains(b'9'));
        assert!(Mode::Alphanumeric.contains(b'A'));
        assert!(Mode::Alphanumeric.contains(b'Z'));
        assert!(Mode::Alphanumeric.contains(b' '));
        assert!(Mode::Alphanumeric.contains(b':'));
        assert!(!Mode::Alphanumeric.contains(b'@'));
        assert!(!Mode::Alphanumeric.contains(b'('));
    }

    #[test]
    fn test_char_count_bit_len() {
        assert_eq!(Mode::Numeric.char_count_bit_len(Version::Normal(1)), 10);
        assert_eq!(Mode::Numeric.char_count_bit_len(Version::Normal(9)), 10);
        assert_eq!(Mode::Numeric.char_count_bit_len(Version::Normal(10)), 12);
        assert_eq!(Mode::Numeric.char_count_bit_len(Version::Normal(26)), 12);
        assert_eq!(Mode::Numeric.char_count_bit_len(Version::Normal(27)), 14);
        assert_eq!(Mode::Numeric.char_count_bit_len(Version::Normal(40)), 14);
        assert_eq!(Mode::Alphanumeric.char_count_bit_len(Version::Normal(1)), 9);
        assert_eq!(Mode::Alphanumeric.char_count_bit_len(Version::Normal(9)), 9);
        assert_eq!(Mode::Alphanumeric.char_count_bit_len(Version::Normal(10)), 11);
        assert_eq!(Mode::Alphanumeric.char_count_bit_len(Version::Normal(26)), 11);
        assert_eq!(Mode::Alphanumeric.char_count_bit_len(Version::Normal(27)), 13);
        assert_eq!(Mode::Alphanumeric.char_count_bit_len(Version::Normal(40)), 13);
        assert_eq!(Mode::Byte.char_count_bit_len(Version::Normal(1)), 8);
        assert_eq!(Mode::Byte.char_count_bit_len(Version::Normal(9)), 8);
        assert_eq!(Mode::Byte.char_count_bit_len(Version::Normal(10)), 16);
        assert_eq!(Mode::Byte.char_count_bit_len(Version::Normal(26)), 16);
        assert_eq!(Mode::Byte.char_count_bit_len(Version::Normal(27)), 16);
        assert_eq!(Mode::Byte.char_count_bit_len(Version::Normal(40)), 16);
    }

    #[test]
    fn test_encoded_len() {
        assert_eq!(Mode::Numeric.encoded_len(3), 10);
        assert_eq!(Mode::Numeric.encoded_len(2), 7);
        assert_eq!(Mode::Numeric.encoded_len(1), 4);
        assert_eq!(Mode::Alphanumeric.encoded_len(2), 11);
        assert_eq!(Mode::Alphanumeric.encoded_len(1), 6);
        assert_eq!(Mode::Byte.encoded_len(1), 8);
    }
}

// Segment
//------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment<'a> {
    mode: Mode,
    data: &'a [u8], // Reference to raw data
}

impl<'a> Segment<'a> {
    pub fn new(mode: Mode, data: &'a [u8]) -> Self {
        Self { mode, data }
    }

    pub fn bit_len(&self, version: Version) -> usize {
        let mode_len = version.mode_len();
        let char_count_len = self.mode.char_count_bit_len(version);
        let encoded_len = self.mode.encoded_len(self.data.len());
        mode_len + char_count_len + encoded_len
    }
}

#[cfg(test)]
mod segment_tests {
    use super::Segment;

    #[test]
    fn test_bit_len_numeric_mode_1() {
        let seg = Segment::new(super::Mode::Numeric, "123".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(1)), 24);
        let seg = Segment::new(super::Mode::Numeric, "45".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(1)), 21);
        let seg = Segment::new(super::Mode::Numeric, "6".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(1)), 18);
    }

    #[test]
    fn test_bit_len_numeric_mode_10() {
        let seg = Segment::new(super::Mode::Numeric, "123".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(10)), 26);
        let seg = Segment::new(super::Mode::Numeric, "45".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(10)), 23);
        let seg = Segment::new(super::Mode::Numeric, "6".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(10)), 20);
    }

    #[test]
    fn test_bit_len_numeric_mode_27() {
        let seg = Segment::new(super::Mode::Numeric, "123".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(27)), 28);
        let seg = Segment::new(super::Mode::Numeric, "45".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(27)), 25);
        let seg = Segment::new(super::Mode::Numeric, "6".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(27)), 22);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_1() {
        let seg = Segment::new(super::Mode::Alphanumeric, "AZ".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(1)), 24);
        let seg = Segment::new(super::Mode::Alphanumeric, "-".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(1)), 19);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_10() {
        let seg = Segment::new(super::Mode::Alphanumeric, "AZ".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(10)), 26);
        let seg = Segment::new(super::Mode::Alphanumeric, "-".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(10)), 21);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_27() {
        let seg = Segment::new(super::Mode::Alphanumeric, "AZ".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(27)), 28);
        let seg = Segment::new(super::Mode::Alphanumeric, "-".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(27)), 23);
    }

    #[test]
    fn test_bit_len_byte_mode_1() {
        let seg = Segment::new(super::Mode::Byte, "a".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(1)), 20);
    }

    #[test]
    fn test_bit_len_byte_mode_10() {
        let seg = Segment::new(super::Mode::Byte, "ab".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(10)), 36);
    }

    #[test]
    fn test_bit_len_byte_mode_27() {
        let seg = Segment::new(super::Mode::Byte, "abc".as_bytes());
        assert_eq!(seg.bit_len(crate::types::Version::Normal(27)), 44);
    }
}

// Encoded Blob
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct EncodedBlob {
    data: Vec<u8>,
    bit_offset: usize,
    version: Version,
    bit_capacity: usize,
}

impl EncodedBlob {
    fn new(version: Version, ec_level: ECLevel) -> Self {
        let bit_capacity = version.bit_capacity(ec_level);
        Self {
            data: Vec::with_capacity((bit_capacity + 7) / 8),
            bit_offset: 0,
            version,
            bit_capacity,
        }
    }

    pub fn bit_len(&self) -> usize {
        match self.bit_offset {
            0 => self.data.len() * 8,
            o => (self.data.len() - 1) * 8 + o,
        }
    }

    fn push_header(&mut self, mode: Mode, char_count: usize) {
        self.push_bits(4, mode as u16);
        let char_count_bit_len = mode.char_count_bit_len(self.version);
        debug_assert!(char_count < (1 << char_count_bit_len), "Char count exceeds bit length");
        self.push_bits(char_count_bit_len, char_count as u16);
    }

    fn push_segment(&mut self, seg: Segment) {
        match seg.mode {
            Mode::Numeric => self.push_numeric_data(seg.data),
            Mode::Alphanumeric => self.push_alphanumeric_data(seg.data),
            Mode::Byte => self.push_byte_data(seg.data),
        }
    }

    fn push_numeric_data(&mut self, data: &[u8]) {
        self.push_header(Mode::Numeric, data.len());
        for chunk in data.chunks(3) {
            let len = (chunk.len() * 10 + 2) / 3;
            let data = Mode::Numeric.from(chunk);
            self.push_bits(len, data);
        }
    }

    fn push_alphanumeric_data(&mut self, data: &[u8]) {
        self.push_header(Mode::Alphanumeric, data.len());
        for chunk in data.chunks(2) {
            let len = (chunk.len() * 11 + 1) / 2;
            let data = Mode::Alphanumeric.from(chunk);
            self.push_bits(len, data);
        }
    }

    fn push_byte_data(&mut self, data: &[u8]) {
        self.push_header(Mode::Byte, data.len());
        for chunk in data.chunks(1) {
            let data = Mode::Byte.from(chunk);
            self.push_bits(8, data);
        }
    }

    pub fn push_terminator(&mut self) {
        let bit_len = self.bit_len();
        if bit_len < self.bit_capacity {
            let term_len = min(4, self.bit_capacity - bit_len);
            self.push_bits(term_len, 0);
        }
    }

    // TODO: Maybe this function should be moved to builder
    pub fn pad_remaining_capacity(&mut self) {
        self.push_padding_bits();
        self.push_padding_codewords();
    }

    fn push_padding_bits(&mut self) {
        if self.bit_offset > 0 {
            let padding_bits_len = 8 - self.bit_offset;
            self.push_bits(padding_bits_len, 0);
        }
    }

    fn push_padding_codewords(&mut self) {
        debug_assert!(
            self.bit_offset == 0,
            "Bit offset should be zero before padding codewords: {}",
            self.bit_offset
        );

        let remain_byte_capacity = (self.bit_capacity - self.bit_len()) / 8;
        PADDING_CODEWORDS.iter().copied().cycle().take(remain_byte_capacity).for_each(|pc| {
            self.push_bits(8, pc as u16);
        });
    }

    fn push_bits(&mut self, bit_len: usize, bits: u16) {
        debug_assert!(
            bit_len >= (16 - bits.leading_zeros()) as usize,
            "Bit count shouldn't exceed bit length: Length {bit_len}, Bits {bits}"
        );
        if bit_len == 0 {
            return;
        }
        debug_assert!(
            self.bit_len() + bit_len <= self.bit_capacity,
            "Capacity overflow: Capacity {}, Size {}",
            self.bit_capacity,
            self.bit_len() + bit_len
        );

        let shifted_len = self.bit_offset + bit_len;
        if self.bit_offset == 0 {
            if shifted_len <= 8 {
                self.data.push((bits << (8 - shifted_len)) as u8);
            } else {
                self.data.push((bits >> (shifted_len - 8)) as u8);
                self.data.push((bits << (16 - shifted_len)) as u8);
            }
        } else {
            let last = self.data.len() - 1;
            if shifted_len <= 8 {
                self.data[last] |= (bits << (8 - shifted_len)) as u8;
            } else if shifted_len <= 16 {
                self.data[last] |= (bits >> (shifted_len - 8)) as u8;
                self.data.push((bits << (16 - shifted_len)) as u8);
            } else {
                self.data[last] |= (bits >> (shifted_len - 8)) as u8;
                self.data.push((bits >> (shifted_len - 16)) as u8);
                self.data.push((bits << (24 - shifted_len)) as u8);
            }
        }
        self.bit_offset = shifted_len & 7;
    }
}

#[cfg(test)]
mod encoding_region_tests {
    use crate::{
        encode::{Mode, PADDING_CODEWORDS},
        types::{ECLevel, Version},
    };

    use super::EncodedBlob;

    #[test]
    fn test_len() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        assert_eq!(eb.bit_len(), 0);
        eb.push_bits(0, 0);
        assert_eq!(eb.bit_len(), 0);
        eb.push_bits(4, 0b1000);
        assert_eq!(eb.bit_len(), 4);
        eb.push_bits(8, 0b1000);
        assert_eq!(eb.bit_len(), 12);
        eb.push_bits(4, 0b1000);
        assert_eq!(eb.bit_len(), 16);
        eb.push_bits(7, 0b1111111);
        assert_eq!(eb.bit_len(), 23);
    }

    #[test]
    fn test_push_bits() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_bits(0, 0);
        assert_eq!(eb.data, vec![]);
        eb.push_bits(4, 0b1000);
        assert_eq!(eb.data, vec![0b10000000]);
        eb.push_bits(4, 0b1000);
        assert_eq!(eb.data, vec![0b10001000]);
        eb.push_bits(8, 0b1000);
        assert_eq!(eb.data, vec![0b10001000, 0b00001000]);
        eb.push_bits(9, 0b1000);
        assert_eq!(eb.data, vec![0b10001000, 0b00001000, 0b00000100, 0b0]);
        eb.push_bits(7, 0b1000);
        assert_eq!(eb.data, vec![0b10001000, 0b00001000, 0b00000100, 0b00001000]);
        eb.push_bits(16, 0b1111111111111111);
        assert_eq!(
            eb.data,
            vec![0b10001000, 0b00001000, 0b00000100, 0b00001000, 0b11111111, 0b11111111]
        );
        eb.push_bits(1, 0b1);
        assert_eq!(
            eb.data,
            vec![
                0b10001000, 0b00001000, 0b00000100, 0b00001000, 0b11111111, 0b11111111, 0b10000000
            ]
        );
        eb.push_bits(11, 0b100);
        assert_eq!(
            eb.data,
            vec![
                0b10001000, 0b00001000, 0b00000100, 0b00001000, 0b11111111, 0b11111111, 0b10000000,
                0b01000000
            ]
        );
        eb.push_bits(16, 0b100);
        assert_eq!(
            eb.data,
            vec![
                0b10001000, 0b00001000, 0b00000100, 0b00001000, 0b11111111, 0b11111111, 0b10000000,
                0b01000000, 0b00000000, 0b01000000
            ]
        );
    }

    #[test]
    #[should_panic]
    fn test_push_bits_capacity_overflow() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let capacity = (version.bit_capacity(ec_level) + 7) / 8;
        let mut eb = EncodedBlob::new(version, ec_level);
        for _ in 0..capacity {
            eb.push_bits(8, 0b1);
        }
        eb.push_bits(1, 0b1)
    }

    #[test]
    fn test_push_header_v1() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_header(Mode::Numeric, 0b11_1111_1111);
        assert_eq!(eb.data, vec![0b00011111, 0b11111100]);
        eb.push_header(Mode::Alphanumeric, 0b1_1111_1111);
        assert_eq!(eb.data, vec![0b00011111, 0b11111100, 0b10111111, 0b11100000]);
        eb.push_header(Mode::Byte, 0b11111111);
        assert_eq!(eb.data, vec![0b00011111, 0b11111100, 0b10111111, 0b11101001, 0b11111110]);
    }

    #[test]
    fn test_push_header_v10() {
        let version = Version::Normal(10);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_header(Mode::Numeric, 0b1111_1111_1111);
        assert_eq!(eb.data, vec![0b00011111, 0b11111111]);
        eb.push_header(Mode::Alphanumeric, 0b111_1111_1111);
        assert_eq!(eb.data, vec![0b00011111, 0b11111111, 0b00101111, 0b11111110]);
        eb.push_header(Mode::Byte, 0b11111111_11111111);
        assert_eq!(
            eb.data,
            vec![
                0b00011111, 0b11111111, 0b00101111, 0b11111110, 0b10011111, 0b11111111, 0b11100000
            ]
        );
    }

    #[test]
    fn test_push_header_v27() {
        let version = Version::Normal(27);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_header(Mode::Numeric, 0b11_1111_1111_1111);
        assert_eq!(eb.data, vec![0b00011111, 0b11111111, 0b11000000]);
        eb.push_header(Mode::Alphanumeric, 0b1_1111_1111_1111);
        assert_eq!(eb.data, vec![0b00011111, 0b11111111, 0b11001011, 0b11111111, 0b11100000]);
        eb.push_header(Mode::Byte, 0b11111111_11111111);
        assert_eq!(
            eb.data,
            vec![
                0b00011111, 0b11111111, 0b11001011, 0b11111111, 0b11101001, 0b11111111, 0b11111110
            ]
        );
    }

    #[test]
    fn test_push_numeric_data() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_numeric_data("01234567".as_bytes());
        assert_eq!(
            eb.data,
            vec![0b00010000, 0b00100000, 0b00001100, 0b01010110, 0b01100001, 0b10000000]
        );
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_numeric_data("8".as_bytes());
        assert_eq!(eb.data, vec![0b00010000, 0b00000110, 0b00]);
    }

    #[test]
    fn test_push_alphanumeric_data() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_alphanumeric_data("AC-42".as_bytes());
        assert_eq!(
            eb.data,
            vec![0b00100000, 0b00101001, 0b11001110, 0b11100111, 0b00100001, 0b00000000]
        )
    }

    #[test]
    fn test_push_byte_data() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_byte_data("a".as_bytes());
        assert_eq!(eb.data, vec![0b01000000, 0b00010110, 0b00010000])
    }

    #[test]
    fn test_push_terminator() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_bits(1, 0b1);
        eb.push_terminator();
        assert_eq!(eb.data, vec![0b10000000]);
        assert_eq!(eb.bit_offset, 5);
    }

    #[test]
    fn test_push_padding_bits() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_bits(1, 0b1);
        eb.push_padding_bits();
        assert_eq!(eb.data, vec![0b10000000]);
        assert_eq!(eb.bit_offset, 0);
    }

    #[test]
    fn test_push_padding_codewords() {
        let version = Version::Normal(1);
        let ec_level = ECLevel::L;
        let mut eb = EncodedBlob::new(version, ec_level);
        eb.push_bits(1, 0b1);
        eb.push_padding_bits();
        eb.push_padding_codewords();
        let mut output = vec![0b10000000];
        output.extend(PADDING_CODEWORDS.iter().cycle().take(18));
        assert_eq!(eb.data, output);
    }
}

// Encoder
//------------------------------------------------------------------------------

// TODO: Write testcases
pub fn encode(data: &[u8], ec_level: ECLevel) -> QRResult<(Vec<u8>, usize, Version)> {
    let (version, segments) = find_optimal_version_and_segments(data, ec_level)?;
    let mut encoded_blob = EncodedBlob::new(version, ec_level);
    for seg in segments {
        encoded_blob.push_segment(seg);
    }
    let encoded_len = (encoded_blob.bit_len() + 7) / 8;
    encoded_blob.push_terminator();
    encoded_blob.pad_remaining_capacity();
    Ok((encoded_blob.data, encoded_len, encoded_blob.version))
}

// TODO: Write testcases
pub fn encode_with_version(
    data: &[u8],
    ec_level: ECLevel,
    version: Version,
) -> QRResult<(Vec<u8>, usize, Version)> {
    let capacity = version.bit_capacity(ec_level);
    let segments = compute_optimal_segments(data, version);
    let size: usize = segments.iter().map(|s| s.bit_len(version)).sum();
    if size > capacity {
        return Err(QRError::DataTooLong);
    }
    let mut encoded_blob = EncodedBlob::new(version, ec_level);
    for seg in segments {
        encoded_blob.push_segment(seg);
    }
    let encoded_len = (encoded_blob.bit_len() + 7) / 8;
    encoded_blob.pad_remaining_capacity();
    Ok((encoded_blob.data, encoded_len, encoded_blob.version))
}

fn find_optimal_version_and_segments(
    data: &[u8],
    ec_level: ECLevel,
) -> QRResult<(Version, Vec<Segment>)> {
    let mut segments = vec![];
    let mut size = 0;
    for v in 1..=40 {
        let version = Version::Normal(v);
        let capacity = version.bit_capacity(ec_level);
        if v == 1 || v == 10 || v == 27 {
            segments = compute_optimal_segments(data, version);
            size = segments.iter().map(|s| s.bit_len(version)).sum();
        }
        if size <= capacity {
            return Ok((version, segments));
        }
    }
    Err(QRError::DataTooLong)
}

// Dynamic programming to compute optimum mode segments
fn compute_optimal_segments(data: &[u8], version: Version) -> Vec<Segment> {
    debug_assert!(!data.is_empty(), "Empty data");

    let len = data.len();
    let mut prev_cost: [usize; 3] = [0; 3];
    MODES
        .iter()
        .enumerate()
        .for_each(|(i, &m)| prev_cost[i] = (4 + m.char_count_bit_len(version)) * 6);
    let mut cur_cost: [usize; 3] = [usize::MAX; 3];
    let mut min_path: Vec<Vec<usize>> = vec![vec![usize::MAX; 3]; len];
    for (i, b) in data.iter().enumerate() {
        for (j, to_mode) in MODES.iter().enumerate() {
            if !to_mode.contains(*b) {
                continue;
            }
            let encoded_char_size = match to_mode {
                Mode::Numeric => 20,
                Mode::Alphanumeric => 33,
                Mode::Byte => 48,
            };
            for (k, from_mode) in MODES.iter().enumerate() {
                if prev_cost[k] == usize::MAX {
                    continue;
                }
                let mut cost = 0;
                if to_mode != from_mode {
                    cost += (prev_cost[k] + 5) / 6 * 6;
                    cost += (4 + to_mode.char_count_bit_len(version)) * 6;
                } else {
                    cost += prev_cost[k];
                }
                cost += encoded_char_size;
                if cost < cur_cost[j] {
                    cur_cost[j] = cost;
                    min_path[i][j] = k;
                }
            }
        }
        swap(&mut prev_cost, &mut cur_cost);
        cur_cost.fill(usize::MAX);
    }

    let char_modes = trace_optimal_modes(min_path, prev_cost);
    build_segments(char_modes, data)
}

// Backtrack min_path and identify optimal char mode
// TODO: Write testcases
fn trace_optimal_modes(min_path: Vec<Vec<usize>>, prev_cost: [usize; 3]) -> Vec<Mode> {
    let len = min_path.len();
    let mut mode_index = 0;
    for i in 1..3 {
        if prev_cost[i] < prev_cost[mode_index] {
            mode_index = i;
        }
    }
    (0..len)
        .rev()
        .scan(mode_index, |mi, i| {
            let old_mi = *mi;
            *mi = min_path[i][*mi];
            Some(MODES[old_mi])
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

// Build segments from char modes
fn build_segments(char_modes: Vec<Mode>, data: &[u8]) -> Vec<Segment> {
    let len = data.len();
    let mut segs: Vec<Segment> = vec![];
    let mut seg_start = 0;
    let mut seg_mode = char_modes[0];
    for (i, &m) in char_modes.iter().enumerate().skip(1) {
        if seg_mode != m {
            segs.push(Segment::new(seg_mode, &data[seg_start..i]));
            seg_mode = m;
            seg_start = i;
        }
    }
    segs.push(Segment::new(seg_mode, &data[seg_start..len]));

    // WARN: remove
    println!("{:?}", segs);
    segs
}

#[cfg(test)]
mod encoder_tests {
    use super::{compute_optimal_segments, find_optimal_version_and_segments, Mode, Segment};
    use crate::{
        encode::build_segments,
        types::{ECLevel, Version},
    };

    #[test]
    fn test_build_segments() {
        let data = "aaaaa11111AAA";
        let mut char_modes = vec![Mode::Alphanumeric; 5];
        char_modes.extend([Mode::Numeric; 5]);
        char_modes.extend([Mode::Byte; 3]);
        let segs = build_segments(char_modes, data.as_bytes());
        let seg_1 = Segment::new(Mode::Alphanumeric, data[0..5].as_bytes());
        let seg_2 = Segment::new(Mode::Numeric, data[5..10].as_bytes());
        let seg_3 = Segment::new(Mode::Byte, data[10..].as_bytes());
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], seg_1);
        assert_eq!(segs[1], seg_2);
        assert_eq!(segs[2], seg_3);
    }

    #[test]
    fn test_compute_optimal_segments_1() {
        let data = "1111111";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Numeric, data[..].as_bytes());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], seg_1);
    }

    #[test]
    fn test_compute_optimal_segments_2() {
        let data = "AAAAA";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Alphanumeric, data[..].as_bytes());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], seg_1);
    }

    #[test]
    fn test_compute_optimal_segments_3() {
        let data = "aaaaa";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Byte, data[..].as_bytes());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], seg_1);
    }

    #[test]
    fn test_compute_optimal_segments_4() {
        let data = "1111111AAAA";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Numeric, data[0..7].as_bytes());
        let seg_2 = Segment::new(Mode::Alphanumeric, data[7..].as_bytes());
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], seg_1);
        assert_eq!(segs[1], seg_2);
    }

    #[test]
    fn test_compute_optimal_segments_5() {
        let data = "111111AAAA";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Alphanumeric, data[..].as_bytes());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], seg_1);
    }

    #[test]
    fn test_compute_optimal_segments_6() {
        let data = "aaa11111a";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Byte, data[..].as_bytes());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], seg_1);
    }

    #[test]
    fn test_compute_optimal_segments_7() {
        let data = "aaa111111a";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Byte, data[..3].as_bytes());
        let seg_2 = Segment::new(Mode::Numeric, data[3..9].as_bytes());
        let seg_3 = Segment::new(Mode::Byte, data[9..].as_bytes());
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], seg_1);
        assert_eq!(segs[1], seg_2);
        assert_eq!(segs[2], seg_3);
    }

    #[test]
    fn test_compute_optimal_segments_8() {
        let data = "aaa1111A";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Byte, data[..].as_bytes());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], seg_1);
    }

    #[test]
    fn test_compute_optimal_segments_9() {
        let data = "aaa1111AA";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Byte, data[..3].as_bytes());
        let seg_2 = Segment::new(Mode::Alphanumeric, data[3..].as_bytes());
        assert_eq!(segs.len(), 2);
        assert_eq!(segs[0], seg_1);
        assert_eq!(segs[1], seg_2);
    }

    #[test]
    fn test_compute_optimal_segments_10() {
        let data = "aaa1111111AA";
        let version = Version::Normal(1);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Byte, data[..3].as_bytes());
        let seg_2 = Segment::new(Mode::Numeric, data[3..10].as_bytes());
        let seg_3 = Segment::new(Mode::Alphanumeric, data[10..].as_bytes());
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0], seg_1);
        assert_eq!(segs[1], seg_2);
        assert_eq!(segs[2], seg_3);
    }

    #[test]
    fn test_compute_optimal_segments_11() {
        let data = "A11111111111111".repeat(23) + "A";
        let version = Version::Normal(10);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        let seg_1 = Segment::new(Mode::Alphanumeric, data[..].as_bytes());
        assert_eq!(segs.len(), 1);
        assert_eq!(segs[0], seg_1);
    }

    #[test]
    fn test_compute_optimal_segments_12() {
        let data = "A11111111111111".repeat(23);
        let version = Version::Normal(9);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        assert_eq!(segs.len(), 46);
        for (i, c) in data.as_bytes().chunks(15).enumerate() {
            let seg_1 = Segment::new(Mode::Alphanumeric, &c[..1]);
            assert_eq!(segs[i * 2], seg_1);
            let seg_2 = Segment::new(Mode::Numeric, &c[1..]);
            assert_eq!(segs[i * 2 + 1], seg_2);
        }
    }

    #[test]
    fn test_compute_optimal_segments_13() {
        let data = "Golden ratio φ = 1.6180339887498948482045868343656381177203091798057628621354486227052604628189024497072072041893911374......";
        let version = Version::Normal(9);
        let segs = compute_optimal_segments(data.as_bytes(), version);
        assert_eq!(segs.len(), 3);
        let seg_1 = Segment::new(Mode::Byte, data[..20].as_bytes());
        let seg_2 = Segment::new(Mode::Numeric, data[20..120].as_bytes());
        let seg_3 = Segment::new(Mode::Alphanumeric, data[120..126].as_bytes());
        assert_eq!(segs[0], seg_1);
        assert_eq!(segs[1], seg_2);
        assert_eq!(segs[2], seg_3);
    }

    #[test]
    fn test_find_optimal_version_and_segments_1() {
        let data = "aaaaa11111AAA";
        let ec_level = ECLevel::L;
        let (version, _) = find_optimal_version_and_segments(data.as_bytes(), ec_level).unwrap();
        assert_eq!(version, Version::Normal(1));
    }

    #[test]
    fn test_find_optimal_version_and_segments_2() {
        let data = "A11111111111111".repeat(2);
        let ec_level = ECLevel::L;
        let (version, _) = find_optimal_version_and_segments(data.as_bytes(), ec_level).unwrap();
        assert_eq!(version, Version::Normal(2));
    }

    #[test]
    fn test_find_optimal_version_and_segments_3() {
        let data = "A11111111111111".repeat(4);
        let ec_level = ECLevel::L;
        let (version, _) = find_optimal_version_and_segments(data.as_bytes(), ec_level).unwrap();
        assert_eq!(version, Version::Normal(3));
    }

    #[test]
    fn test_find_optimal_version_and_segments_4() {
        let data = "aAAAAAAAAAAA".repeat(5) + "a";
        let ec_level = ECLevel::L;
        let (version, _) = find_optimal_version_and_segments(data.as_bytes(), ec_level).unwrap();
        assert_eq!(version, Version::Normal(4));
    }

    #[test]
    fn test_find_optimal_version_and_segments_5() {
        let data = "aAAAAAAAAAAA".repeat(21) + "a";
        let ec_level = ECLevel::L;
        let (version, _) = find_optimal_version_and_segments(data.as_bytes(), ec_level).unwrap();
        assert_eq!(version, Version::Normal(10));
    }

    #[test]
    fn test_find_optimal_version_and_segments_max_capacity() {
        let data = "a".repeat(2953);
        let ec_level = ECLevel::L;
        let (version, _) = find_optimal_version_and_segments(data.as_bytes(), ec_level).unwrap();
        assert_eq!(version, Version::Normal(40));
    }

    #[test]
    #[should_panic]
    fn test_find_optimal_version_and_segments_panic() {
        let data = "a".repeat(2954);
        let ec_level = ECLevel::L;
        find_optimal_version_and_segments(data.as_bytes(), ec_level).unwrap();
    }
}

// Global constants
//------------------------------------------------------------------------------

static PADDING_CODEWORDS: [u8; 2] = [0b1110_1100, 0b0001_0001];

static MODES: [Mode; 3] = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte];
