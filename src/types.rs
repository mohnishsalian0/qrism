use std::cmp::{Ordering, PartialOrd};
use std::fmt::{Display, Error, Formatter};

// Error
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum QRError {
    DataTooLong,
    InvalidVersion,
    InvalidECLevel,
    InvalidPalette,
    InvalidColor,
    InvalidChar,
    InvalidMaskingPattern,
}

impl Display for QRError {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        let msg = match *self {
            Self::DataTooLong => "data too long",
            Self::InvalidVersion => "invalid version",
            Self::InvalidECLevel => "invalid error correction level",
            Self::InvalidPalette => "invalid color palette",
            Self::InvalidColor => "invalid color",
            Self::InvalidChar => "invalid character",
            Self::InvalidMaskingPattern => "invalid masking pattern",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for QRError {}

pub type QRResult<T> = Result<T, QRError>;

// Color
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Color {
    Mono(bool),
    Multi(u8, u8, u8),
}

// Version
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Version {
    Micro(usize),
    Normal(usize),
}

impl Version {
    pub const fn width(&self) -> usize {
        debug_assert!(
            matches!(*self, Self::Micro(1..=4) | Self::Normal(1..=40)),
            "Invalid version"
        );
        match *self {
            Self::Micro(v) => v * 2 + 9,
            Self::Normal(v) => v * 4 + 17,
        }
    }

    pub fn alignment_pattern(&self) -> &[usize] {
        debug_assert!(
            matches!(*self, Self::Micro(1..=4) | Self::Normal(1..=40)),
            "Invalid version"
        );
        match *self {
            Self::Micro(_) => &[],
            Self::Normal(v) => ALIGNMENT_PATTERN_POSITIONS[v],
        }
    }

    pub fn version_info(&self) -> u32 {
        debug_assert!(matches!(*self, Self::Normal(7..=40)), "Invalid version");
        match *self {
            Self::Normal(v) => VERSION_INFOS[v - 7],
            _ => unreachable!(),
        }
    }
}
#[cfg(test)]
mod version_tests {
    use super::Version::*;

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_micro_version_low() {
        let invalid_version = Micro(0);
        invalid_version.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_micro_version_high() {
        let invalid_version = Micro(5);
        invalid_version.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_normal_version_low() {
        let invalid_version = Normal(0);
        invalid_version.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_normal_version_high() {
        let invalid_version = Normal(41);
        invalid_version.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_version_info_invalid_version_low() {
        let invalid_version = Normal(0);
        invalid_version.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_version_info_invalid_version_high() {
        let invalid_version = Normal(41);
        invalid_version.alignment_pattern();
    }
}

// Error correction level
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone, PartialOrd, Ord)]
pub enum ECLevel {
    L = 0,
    M = 1,
    Q = 2,
    H = 3,
}

// Palette
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Palette {
    Monochrome,
    Multicolor(u16),
}

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

// Masking pattern
//------------------------------------------------------------------------------

pub struct MaskingPattern(u8);

mod mask_functions {
    pub fn checkerboard(r: i16, c: i16) -> bool {
        (r + c) & 1 == 0
    }

    pub fn horizontal_lines(r: i16, _: i16) -> bool {
        r & 1 == 0
    }

    pub fn vertical_lines(_: i16, c: i16) -> bool {
        c % 3 == 0
    }

    pub fn diagonal_lines(r: i16, c: i16) -> bool {
        (r + c) % 3 == 0
    }

    pub fn large_checkerboard(r: i16, c: i16) -> bool {
        ((r >> 1) + (c / 3)) & 1 == 0
    }

    pub fn fields(r: i16, c: i16) -> bool {
        ((r * c) & 1) + ((r * c) % 3) == 0
    }

    pub fn diamonds(r: i16, c: i16) -> bool {
        (((r * c) & 1) + ((r * c) % 3)) & 1 == 0
    }

    pub fn meadow(r: i16, c: i16) -> bool {
        (((r + c) & 1) + ((r * c) % 3)) & 1 == 0
    }
}

pub fn get_mask_functions(pattern: MaskingPattern) -> QRResult<fn(i16, i16) -> bool> {
    let MaskingPattern(pattern) = pattern;
    let mask_function = match pattern {
        0b000 => mask_functions::checkerboard,
        0b001 => mask_functions::horizontal_lines,
        0b010 => mask_functions::vertical_lines,
        0b011 => mask_functions::diagonal_lines,
        0b100 => mask_functions::large_checkerboard,
        0b101 => mask_functions::fields,
        0b110 => mask_functions::diamonds,
        0b111 => mask_functions::meadow,
        _ => return Err(QRError::InvalidMaskingPattern),
    };
    Ok(mask_function)
}

// Format information
//------------------------------------------------------------------------------

pub fn format_info_qr(ec_level: ECLevel, mask_pattern: MaskingPattern) -> u16 {
    let MaskingPattern(m) = mask_pattern;
    let format_data = ((ec_level as usize) ^ 1) << 3 | (m as usize);
    FORMAT_INFOS_QR[format_data]
}

// Global constants
//------------------------------------------------------------------------------

static ALIGNMENT_PATTERN_POSITIONS: [&[usize]; 40] = [
    &[],
    &[6, 18],
    &[6, 22],
    &[6, 26],
    &[6, 30],
    &[6, 34],
    &[6, 22, 38],
    &[6, 24, 42],
    &[6, 26, 46],
    &[6, 28, 50],
    &[6, 30, 54],
    &[6, 32, 58],
    &[6, 34, 62],
    &[6, 26, 46, 66],
    &[6, 26, 48, 70],
    &[6, 26, 50, 74],
    &[6, 30, 54, 78],
    &[6, 30, 56, 82],
    &[6, 30, 58, 86],
    &[6, 34, 62, 90],
    &[6, 28, 50, 72, 94],
    &[6, 26, 50, 74, 98],
    &[6, 30, 54, 78, 102],
    &[6, 28, 54, 80, 106],
    &[6, 32, 58, 84, 110],
    &[6, 30, 58, 86, 114],
    &[6, 34, 62, 90, 118],
    &[6, 26, 50, 74, 98, 122],
    &[6, 30, 54, 78, 102, 126],
    &[6, 26, 52, 78, 104, 130],
    &[6, 30, 56, 82, 108, 134],
    &[6, 34, 60, 86, 112, 138],
    &[6, 30, 58, 86, 114, 142],
    &[6, 34, 62, 90, 118, 146],
    &[6, 30, 54, 78, 102, 126, 150],
    &[6, 24, 50, 76, 102, 128, 154],
    &[6, 28, 54, 80, 106, 132, 158],
    &[6, 32, 58, 84, 110, 136, 162],
    &[6, 26, 54, 82, 110, 138, 166],
    &[6, 30, 58, 86, 114, 142, 170],
];

static VERSION_INFOS: [u32; 34] = [
    0x07c94, 0x085bc, 0x09a99, 0x0a4d3, 0x0bbf6, 0x0c762, 0x0d847, 0x0e60d, 0x0f928, 0x10b78,
    0x1145d, 0x12a17, 0x13532, 0x149a6, 0x15683, 0x168c9, 0x177ec, 0x18ec4, 0x191e1, 0x1afab,
    0x1b08e, 0x1cc1a, 0x1d33f, 0x1ed75, 0x1f250, 0x209d5, 0x216f0, 0x228ba, 0x2379f, 0x24b0b,
    0x2542e, 0x26a64, 0x27541, 0x28c69,
];

static FORMAT_INFOS_QR: [u16; 32] = [
    0x5412, 0x5125, 0x5e7c, 0x5b4b, 0x45f9, 0x40ce, 0x4f97, 0x4aa0, 0x77c4, 0x72f3, 0x7daa, 0x789d,
    0x662f, 0x6318, 0x6c41, 0x6976, 0x1689, 0x13be, 0x1ce7, 0x19d0, 0x0762, 0x0255, 0x0d0c, 0x083b,
    0x355f, 0x3068, 0x3f31, 0x3a06, 0x24b4, 0x2183, 0x2eda, 0x2bed,
];
