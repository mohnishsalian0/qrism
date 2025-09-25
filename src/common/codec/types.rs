use std::cmp::Ordering;

// Mode
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Mode {
    Numeric = 0b0001,
    Alphanumeric = 0b0010,
    Byte = 0b0100,
    Kanji = 0b1000,
    Eci = 0b0111,
    Terminator = 0b0000,
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

    #[inline]
    fn byte(&self, mode_digit: u8) -> u8 {
        match self {
            Self::Numeric => match mode_digit {
                md @ 0..=9 => md + b'0',
                _ => unreachable!("Invalid numeric digit {mode_digit}"),
            },
            Self::Alphanumeric => match mode_digit {
                md @ 0..=9 => md + b'0',
                md @ 10..=35 => md - 10 + b'A',
                36 => b' ',
                37 => b'$',
                38 => b'%',
                39 => b'*',
                40 => b'+',
                41 => b'-',
                42 => b'.',
                43 => b'/',
                44 => b':',
                _ => unreachable!("Invalid alphanumeric digit {mode_digit}"),
            },
            Self::Byte => mode_digit,
            Self::Kanji => todo!(),
            Self::Eci => unreachable!("ECI mode doesn't have characters"),
            Self::Terminator => unreachable!("Terminator mode doesn't have characters"),
        }
    }

    pub fn encode_chunk(&self, data: &[u8]) -> u16 {
        let len = data.len();
        match self {
            Self::Numeric => {
                debug_assert!(len <= 3, "Data is too long for numeric conver: {len}");
                data.iter().fold(0_u16, |n, b| n * 10 + Self::numeric_digit(*b))
            }
            Self::Alphanumeric => {
                debug_assert!(len <= 2, "Data is too long for alphanumeric conver: {len}");
                data.iter().fold(0_u16, |n, b| n * 45 + Self::alphanumeric_digit(*b))
            }
            Self::Byte => {
                debug_assert!(len == 1, "Data is too long for byte conver: {len}");
                data[0] as u16
            }
            Self::Kanji => todo!(),
            Self::Eci => unreachable!("Cannot encode in ECI mode"),
            Self::Terminator => unreachable!("Cannot encode in terminator mode"),
        }
    }

    pub fn decode_chunk(&self, data: u16, bit_len: usize) -> Vec<u8> {
        match self {
            Self::Numeric => Self::decode_numeric_chunk(data, bit_len),
            Self::Alphanumeric => Self::decode_alphanumeric_chunk(data, bit_len),
            Self::Byte => {
                debug_assert!(bit_len == 8, "Invalid byte encoded length: {bit_len}");

                vec![data as u8]
            }
            Self::Kanji => Self::decode_kanji_chunk(data),
            Self::Eci => unreachable!("Cannot decode in ECI mode"),
            Self::Terminator => unreachable!("Cannot decode in terminator mode"),
        }
    }

    fn decode_numeric_chunk(mut data: u16, bit_len: usize) -> Vec<u8> {
        debug_assert!(
            bit_len == 10 || bit_len == 7 || bit_len == 4,
            "Invalid numeric encoded length: {bit_len}"
        );

        let len = bit_len / 3;
        let mut res = vec![0; len];
        for i in 0..len {
            res[len - 1 - i] = Mode::Numeric.byte((data % 10) as u8);
            data /= 10;
        }
        res
    }

    fn decode_alphanumeric_chunk(mut data: u16, bit_len: usize) -> Vec<u8> {
        debug_assert!(
            bit_len == 11 || bit_len == 6,
            "Invalid alphanumeric encoded length: {bit_len}"
        );

        let len = bit_len / 5;
        let mut res = vec![0; len];
        for i in 0..len {
            res[len - 1 - i] = Mode::Alphanumeric.byte((data % 45) as u8);
            data /= 45;
        }
        res
    }

    fn decode_kanji_chunk(data: u16) -> Vec<u8> {
        let msbyte = data / 0xc0;
        let lsbyte = data % 0xc0;
        let temp = ((msbyte << 8) | lsbyte) + 0x8140;
        let sjw = if temp <= 0x9ffc { temp } else { temp + 0x4000 };

        vec![(sjw >> 8) as u8, (sjw & 0xff) as u8]
    }

    pub fn contains(&self, byte: u8) -> bool {
        match self {
            Self::Numeric => byte.is_ascii_digit(),
            Self::Alphanumeric => {
                matches!(byte, b'0'..=b'9' | b'A'..=b'Z' | b' ' | b'$' | b'%' | b'*' | b'+' | b'-' | b'.' | b'/' | b':')
            }
            Self::Byte => true,
            Self::Kanji => todo!(),
            Self::Eci | Self::Terminator => false,
        }
    }

    pub fn encoded_len(&self, len: usize) -> usize {
        match *self {
            Self::Numeric => (len * 10).div_ceil(3),
            Self::Alphanumeric => (len * 11).div_ceil(2),
            Self::Byte => len * 8,
            Self::Kanji => (len / 2) * 13,
            Self::Eci => len,
            Self::Terminator => unreachable!("Cannot encode in terminator mode"),
        }
    }
}

#[cfg(test)]
mod mode_tests {

    use super::Mode;
    use super::Mode::*;

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
    fn test_numeric_to_byte() {
        assert_eq!(Numeric.byte(0), b'0');
        assert_eq!(Numeric.byte(9), b'9');
    }

    #[test]
    #[should_panic]
    fn test_invalid_numeric_digit_to_byte() {
        Numeric.byte(b'A');
    }

    #[test]
    fn test_alphanumeric_to_byte() {
        assert_eq!(Alphanumeric.byte(0), b'0');
        assert_eq!(Alphanumeric.byte(9), b'9');
        assert_eq!(Alphanumeric.byte(10), b'A');
        assert_eq!(Alphanumeric.byte(35), b'Z');
        assert_eq!(Alphanumeric.byte(36), b' ');
        assert_eq!(Alphanumeric.byte(44), b':');
    }

    #[test]
    #[should_panic]
    fn test_invalid_alphanumeric_digit_to_byte() {
        Alphanumeric.byte(b'a');
    }

    #[test]
    fn test_numeric_encoding() {
        assert_eq!(Numeric.encode_chunk("012".as_bytes()), 0b0000001100);
        assert_eq!(Numeric.encode_chunk("345".as_bytes()), 0b0101011001);
        assert_eq!(Numeric.encode_chunk("901".as_bytes()), 0b1110000101);
        assert_eq!(Numeric.encode_chunk("67".as_bytes()), 0b1000011);
        assert_eq!(Numeric.encode_chunk("8".as_bytes()), 0b1000);
    }

    #[test]
    #[should_panic]
    fn test_invalid_numeric_encoding() {
        Numeric.encode_chunk("1234".as_bytes());
    }

    #[test]
    fn test_numeric_decoding() {
        let data = "012".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 10), data);
        let data = "345".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 10), data);
        let data = "901".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 10), data);
        let data = "67".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 7), data);
        let data = "8".as_bytes();
        let encoded_data = Numeric.encode_chunk(data);
        assert_eq!(Numeric.decode_chunk(encoded_data, 4), data);
    }

    #[test]
    fn test_alphanumeric_encoding() {
        assert_eq!(Alphanumeric.encode_chunk("AC".as_bytes()), 0b00111001110);
        assert_eq!(Alphanumeric.encode_chunk("-4".as_bytes()), 0b11100111001);
        assert_eq!(Alphanumeric.encode_chunk("2".as_bytes()), 0b000010);
    }

    #[test]
    #[should_panic]
    fn test_invalid_alphanumeric_encoding() {
        Alphanumeric.encode_chunk("1234".as_bytes());
    }

    #[test]
    fn test_alphanumeric_decoding() {
        let data = "AC".as_bytes();
        let encoded_data = Alphanumeric.encode_chunk(data);
        assert_eq!(Alphanumeric.decode_chunk(encoded_data, 11), data);
        let data = "-4".as_bytes();
        let encoded_data = Alphanumeric.encode_chunk(data);
        assert_eq!(Alphanumeric.decode_chunk(encoded_data, 11), data);
        let data = "2".as_bytes();
        let encoded_data = Alphanumeric.encode_chunk(data);
        assert_eq!(Alphanumeric.decode_chunk(encoded_data, 6), data);
    }

    #[test]
    fn test_is_numeric() {
        assert!(Numeric.contains(b'0'));
        assert!(Numeric.contains(b'9'));
        assert!(!Numeric.contains(b'A'));
        assert!(!Numeric.contains(b'Z'));
        assert!(!Numeric.contains(b' '));
        assert!(!Numeric.contains(b':'));
    }

    #[test]
    fn test_is_alphanumeric() {
        assert!(Alphanumeric.contains(b'0'));
        assert!(Alphanumeric.contains(b'9'));
        assert!(Alphanumeric.contains(b'A'));
        assert!(Alphanumeric.contains(b'Z'));
        assert!(Alphanumeric.contains(b' '));
        assert!(Alphanumeric.contains(b':'));
        assert!(!Alphanumeric.contains(b'@'));
        assert!(!Alphanumeric.contains(b'('));
    }

    #[test]
    fn test_encoded_len() {
        assert_eq!(Numeric.encoded_len(3), 10);
        assert_eq!(Numeric.encoded_len(2), 7);
        assert_eq!(Numeric.encoded_len(1), 4);
        assert_eq!(Alphanumeric.encoded_len(2), 11);
        assert_eq!(Alphanumeric.encoded_len(1), 6);
        assert_eq!(Byte.encoded_len(1), 8);
    }
}

// Segment
//------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment<'a> {
    pub mode: Mode,
    pub mode_bits: usize, // Bit len of mode
    pub len_bits: usize,  // Bit len of char count
    pub data: &'a [u8],   // Reference to raw data
}

impl<'a> Segment<'a> {
    pub fn new(mode: Mode, mode_bits: usize, len_bits: usize, data: &'a [u8]) -> Self {
        Self { mode, mode_bits, len_bits, data }
    }

    pub fn bit_len(&self) -> usize {
        let encoded_bits = self.mode.encoded_len(self.data.len());
        self.mode_bits + self.len_bits + encoded_bits
    }
}

#[cfg(test)]
mod segment_tests {
    use super::{Mode, Segment};
    use crate::Version;

    #[test]
    fn test_bit_len_numeric_mode_1() {
        let ver = Version::Normal(1);
        let mode = Mode::Numeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "123".as_bytes());
        assert_eq!(seg.bit_len(), 24);
        let seg = Segment::new(mode, mode_bits, len_bits, "45".as_bytes());
        assert_eq!(seg.bit_len(), 21);
        let seg = Segment::new(mode, mode_bits, len_bits, "6".as_bytes());
        assert_eq!(seg.bit_len(), 18);
    }

    #[test]
    fn test_bit_len_numeric_mode_10() {
        let ver = Version::Normal(10);
        let mode = Mode::Numeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "123".as_bytes());
        assert_eq!(seg.bit_len(), 26);
        let seg = Segment::new(mode, mode_bits, len_bits, "45".as_bytes());
        assert_eq!(seg.bit_len(), 23);
        let seg = Segment::new(mode, mode_bits, len_bits, "6".as_bytes());
        assert_eq!(seg.bit_len(), 20);
    }

    #[test]
    fn test_bit_len_numeric_mode_27() {
        let ver = Version::Normal(27);
        let mode = Mode::Numeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "123".as_bytes());
        assert_eq!(seg.bit_len(), 28);
        let seg = Segment::new(mode, mode_bits, len_bits, "45".as_bytes());
        assert_eq!(seg.bit_len(), 25);
        let seg = Segment::new(mode, mode_bits, len_bits, "6".as_bytes());
        assert_eq!(seg.bit_len(), 22);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_1() {
        let ver = Version::Normal(1);
        let mode = Mode::Alphanumeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "AZ".as_bytes());
        assert_eq!(seg.bit_len(), 24);
        let seg = Segment::new(mode, mode_bits, len_bits, "-".as_bytes());
        assert_eq!(seg.bit_len(), 19);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_10() {
        let ver = Version::Normal(10);
        let mode = Mode::Alphanumeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "AZ".as_bytes());
        assert_eq!(seg.bit_len(), 26);
        let seg = Segment::new(mode, mode_bits, len_bits, "-".as_bytes());
        assert_eq!(seg.bit_len(), 21);
    }

    #[test]
    fn test_bit_len_alphanumeric_mode_27() {
        let ver = Version::Normal(27);
        let mode = Mode::Alphanumeric;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "AZ".as_bytes());
        assert_eq!(seg.bit_len(), 28);
        let seg = Segment::new(mode, mode_bits, len_bits, "-".as_bytes());
        assert_eq!(seg.bit_len(), 23);
    }

    #[test]
    fn test_bit_len_byte_mode_1() {
        let ver = Version::Normal(1);
        let mode = Mode::Byte;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "a".as_bytes());
        assert_eq!(seg.bit_len(), 20);
    }

    #[test]
    fn test_bit_len_byte_mode_10() {
        let ver = Version::Normal(10);
        let mode = Mode::Byte;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "ab".as_bytes());
        assert_eq!(seg.bit_len(), 36);
    }

    #[test]
    fn test_bit_len_byte_mode_27() {
        let ver = Version::Normal(27);
        let mode = Mode::Byte;
        let mode_bits = ver.mode_bits();
        let len_bits = ver.char_cnt_bits(mode);
        let seg = Segment::new(mode, mode_bits, len_bits, "abc".as_bytes());
        assert_eq!(seg.bit_len(), 44);
    }
}

// Global constants
//------------------------------------------------------------------------------

pub static PADDING_CODEWORDS: [u8; 2] = [0b1110_1100, 0b0001_0001];

pub static MODES: [Mode; 3] = [Mode::Numeric, Mode::Alphanumeric, Mode::Byte];
