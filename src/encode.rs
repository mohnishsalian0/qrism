use crate::types::{ECLevel, QRError, QRResult, Version};
use std::cmp::Ordering;

// Mode
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Mode {
    Numeric = 0b0001,
    Alphanumeric = 0b0010,
    Byte = 0b0100,
}

impl Mode {
    pub fn char_count_bits_len(&self, version: Version) -> QRResult<usize> {
        let bits_len = match version {
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
            Version::Normal(27..=40) => match *self {
                Self::Numeric => 14,
                Self::Alphanumeric => 13,
                Self::Byte => 16,
            },
            _ => return Err(QRError::InvalidVersion),
        };
        Ok(bits_len)
    }

    pub fn data_bits_len(&self, raw_data_len: usize) -> usize {
        match *self {
            Self::Numeric => (raw_data_len * 10 + 2) / 3,
            Self::Alphanumeric => (raw_data_len * 11 + 1) / 2,
            Self::Byte => raw_data_len * 8,
        }
    }
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

#[cfg(test)]
mod mode_tests {
    use super::Mode::*;
    use super::QRError::*;
    use super::Version::*;

    #[test]
    fn test_char_count_bits_len_invalid_version() {
        let result = Numeric.char_count_bits_len(Normal(0));
        assert_eq!(result, Err(InvalidVersion));
        let result = Alphanumeric.char_count_bits_len(Normal(41));
        assert_eq!(result, Err(InvalidVersion));
        let result = Alphanumeric.char_count_bits_len(Normal(usize::MAX));
        assert_eq!(result, Err(InvalidVersion));
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
}

// Segment
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Segment<'a> {
    mode: Mode,
    char_count: usize,
    data: &'a [u8], // Reference to raw data
}

impl<'a> Segment<'a> {
    fn get_mode(&self) -> Mode {
        self.mode
    }

    fn get_char_count(&self) -> usize {
        self.char_count
    }

    fn get_encoded_len(&self, version: Version) -> usize {
        todo!()
    }
}

// Encoded data
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct EncodedData {
    data: Vec<u8>,
    remainder_bits: u8,
    version: Version,
}

impl EncodedData {
    fn push_mode(&mut self, mode: Mode) {
        todo!()
    }

    fn push_char_count(&mut self, count: usize) {
        todo!()
    }

    fn push_header(&mut self, mode: Mode, count: usize) {
        todo!()
    }

    fn push_bits(&mut self, bits: u16) {
        todo!()
    }

    fn push_numeric_data(&mut self, data: &[u8]) {
        todo!()
    }

    fn push_alphanumeric_data(&mut self, data: &[u8]) {
        todo!()
    }

    fn push_byte_data(&mut self, data: &[u8]) {
        todo!()
    }

    fn push_segment(&mut self, seg: Segment) {
        todo!()
    }

    fn push_terminator(&mut self) {
        todo!()
    }
}

// Encoder
//------------------------------------------------------------------------------

fn compute_optimal_segments(data: &[u8], version: Version) -> Vec<Segment> {
    todo!()
}

fn find_best_version(data: &[u8], ec_level: ECLevel) -> Version {
    todo!()
}

pub fn encode(data: &[u8], ec_level: ECLevel) -> Result<EncodedData, QRError> {
    todo!()
}

pub fn encode_with_version(data: &[u8], ec_level: ECLevel) -> Result<EncodedData, QRError> {
    todo!()
}

// Global constants
//------------------------------------------------------------------------------

// Bit capacity per error level per version
static VERSION_BIT_CAPACITY: [[usize; 4]; 40] = [
    [152, 128, 104, 72],
    [272, 224, 176, 128],
    [440, 352, 272, 208],
    [640, 512, 384, 288],
    [864, 688, 496, 368],
    [1088, 864, 608, 480],
    [1248, 992, 704, 528],
    [1552, 1232, 880, 688],
    [1856, 1456, 1056, 800],
    [2192, 1728, 1232, 976],
    [2592, 2032, 1440, 1120],
    [2960, 2320, 1648, 1264],
    [3424, 2672, 1952, 1440],
    [3688, 2920, 2088, 1576],
    [4184, 3320, 2360, 1784],
    [4712, 3624, 2600, 2024],
    [5176, 4056, 2936, 2264],
    [5768, 4504, 3176, 2504],
    [6360, 5016, 3560, 2728],
    [6888, 5352, 3880, 3080],
    [7456, 5712, 4096, 3248],
    [8048, 6256, 4544, 3536],
    [8752, 6880, 4912, 3712],
    [9392, 7312, 5312, 4112],
    [10208, 8000, 5744, 4304],
    [10960, 8496, 6032, 4768],
    [11744, 9024, 6464, 5024],
    [12248, 9544, 6968, 5288],
    [13048, 10136, 7288, 5608],
    [13880, 10984, 7880, 5960],
    [14744, 11640, 8264, 6344],
    [15640, 12328, 8920, 6760],
    [16568, 13048, 9368, 7208],
    [17528, 13800, 9848, 7688],
    [18448, 14496, 10288, 7888],
    [19472, 15312, 10832, 8432],
    [20528, 15936, 11408, 8768],
    [21616, 16816, 12016, 9136],
    [22496, 17728, 12656, 9776],
    [23648, 18672, 13328, 10208],
];
