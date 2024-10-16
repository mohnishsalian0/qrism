use std::cmp::{Ordering, PartialOrd};
use std::fmt::{Debug, Display, Error, Formatter};
use std::ops::{Deref, Not};

use crate::mask::MaskingPattern;

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

// Version
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Version {
    Micro(usize),
    Normal(usize),
}

impl Deref for Version {
    type Target = usize;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Micro(v) => v,
            Self::Normal(v) => v,
        }
    }
}

impl Version {
    pub const fn get_width(self) -> usize {
        debug_assert!(
            matches!(self, Self::Micro(1..=4) | Self::Normal(1..=40)),
            "Invalid version"
        );
        match self {
            Self::Micro(v) => v * 2 + 9,
            Self::Normal(v) => v * 4 + 17,
        }
    }

    pub fn get_alignment_pattern(self) -> &'static [i16] {
        debug_assert!(
            matches!(self, Self::Micro(1..=4) | Self::Normal(1..=40)),
            "Invalid version"
        );
        match self {
            Self::Micro(_) => &[],
            Self::Normal(v) => ALIGNMENT_PATTERN_POSITIONS[v - 1],
        }
    }

    pub fn get_version_info(self) -> u32 {
        debug_assert!(matches!(self, Self::Normal(7..=40)), "Invalid version");
        match self {
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
        invalid_version.get_alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_micro_version_high() {
        let invalid_version = Micro(5);
        invalid_version.get_alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_normal_version_low() {
        let invalid_version = Normal(0);
        invalid_version.get_alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_normal_version_high() {
        let invalid_version = Normal(41);
        invalid_version.get_alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_version_info_invalid_version_low() {
        let invalid_version = Normal(0);
        invalid_version.get_alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_version_info_invalid_version_high() {
        let invalid_version = Normal(41);
        invalid_version.get_alignment_pattern();
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
    Polychrome(u8),
}

impl Deref for Palette {
    type Target = u8;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Monochrome => &1,
            Self::Polychrome(p) => p,
        }
    }
}

impl Palette {
    pub fn get_palette_info(self) -> u32 {
        debug_assert!(0 < *self && *self < 17, "Invalid palette");

        match self {
            Self::Monochrome => 1,
            Self::Polychrome(p) => PALETTE_INFOS[p as usize],
        }
    }
}

// Color
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Color {
    Light,
    Dark,
    Hue(u32),
}

// TODO: Figure out how to handle hue
impl Not for Color {
    type Output = Self;
    fn not(self) -> Self::Output {
        match self {
            Self::Light => Self::Dark,
            Self::Dark => Self::Light,
            Self::Hue(h) => Self::Hue(!h),
        }
    }
}

// TODO: Figure out how to handle hue
impl Color {
    pub fn select<T: Debug>(&self, light: T, dark: T, hue: T) -> T {
        match self {
            Self::Light => light,
            Self::Dark => dark,
            Self::Hue(_) => hue,
        }
    }
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

// Format information
//------------------------------------------------------------------------------

pub fn get_format_info(version: Version, ec_level: ECLevel, mask_pattern: MaskingPattern) -> u32 {
    match version {
        Version::Micro(_) => todo!(),
        Version::Normal(_) => {
            let format_data = ((ec_level as usize) ^ 1) << 3 | (*mask_pattern as usize);
            FORMAT_INFOS_QR[format_data]
        }
    }
}

// Global constants
//------------------------------------------------------------------------------

static ALIGNMENT_PATTERN_POSITIONS: [&[i16]; 40] = [
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

static FORMAT_INFOS_QR: [u32; 32] = [
    0x5412, 0x5125, 0x5e7c, 0x5b4b, 0x45f9, 0x40ce, 0x4f97, 0x4aa0, 0x77c4, 0x72f3, 0x7daa, 0x789d,
    0x662f, 0x6318, 0x6c41, 0x6976, 0x1689, 0x13be, 0x1ce7, 0x19d0, 0x0762, 0x0255, 0x0d0c, 0x083b,
    0x355f, 0x3068, 0x3f31, 0x3a06, 0x24b4, 0x2183, 0x2eda, 0x2bed,
];

// TODO: Fill out palette info
static PALETTE_INFOS: [u32; 12] = [0xFFF; 12];
