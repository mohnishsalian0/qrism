use core::panic;
use std::cmp::PartialOrd;
use std::fmt::{Debug, Display};
use std::ops::{Deref, Not};

use image::{Luma, Rgb};

use super::{codec::Mode, mask::MaskPattern};

// Metadata
//------------------------------------------------------------------------------

#[derive(Debug, Copy, Clone)]
pub struct Metadata {
    ver: Option<Version>,
    ecl: Option<ECLevel>,
    mask: Option<MaskPattern>,
}

impl Metadata {
    pub fn new(ver: Option<Version>, ecl: Option<ECLevel>, mask: Option<MaskPattern>) -> Self {
        Self { ver, ecl, mask }
    }
}

impl Display for Metadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ver = match &self.ver {
            Some(v) => format!("{:?}", v),
            None => "None".to_string(),
        };
        let ec = match &self.ecl {
            Some(e) => format!("{:?}", e),
            None => "None".to_string(),
        };
        let mask = match &self.mask {
            Some(m) => format!("{:?}", m),
            None => "None".to_string(),
        };
        write!(f, "Metadata: Version: {}, EC Level: {}, Masking Pattern: {} ", ver, ec, mask)
    }
}

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
    pub fn from_grid_size(grid_size: usize) -> Option<Self> {
        if !(21..=177).contains(&grid_size) {
            return None;
        };
        Some(Version::Normal((grid_size - 17) / 4))
    }

    pub const fn width(self) -> usize {
        debug_assert!(matches!(self, Self::Micro(1..=4) | Self::Normal(1..=40)), "Invalid version");
        match self {
            Self::Micro(v) => v * 2 + 9,
            Self::Normal(v) => v * 4 + 17,
        }
    }

    pub fn alignment_pattern(self) -> &'static [i32] {
        debug_assert!(matches!(self, Self::Micro(1..=4) | Self::Normal(1..=40)), "Invalid version");
        match self {
            Self::Micro(_) => &[],
            Self::Normal(v) => ALIGNMENT_PATTERN_POSITIONS[v - 1],
        }
    }

    pub fn mode_bits(self) -> usize {
        match self {
            Version::Micro(v) => v - 1,
            Version::Normal(_) => 4,
        }
    }

    pub fn char_cnt_bits(&self, mode: Mode) -> usize {
        debug_assert!(
            matches!(self, Version::Micro(1..=4) | Version::Normal(1..=40)),
            "Invalid version"
        );

        match self {
            Version::Micro(v) => match mode {
                Mode::Numeric => *v + 2,
                Mode::Alphanumeric => *v + 1,
                Mode::Byte => *v + 1,
                Mode::Kanji => *v,
                Mode::Eci | Mode::Terminator => 0,
            },
            Version::Normal(1..=9) => match mode {
                Mode::Numeric => 10,
                Mode::Alphanumeric => 9,
                Mode::Byte => 8,
                Mode::Kanji => 8,
                Mode::Eci | Mode::Terminator => 0,
            },
            Version::Normal(10..=26) => match mode {
                Mode::Numeric => 12,
                Mode::Alphanumeric => 11,
                Mode::Byte => 16,
                Mode::Kanji => 10,
                Mode::Eci | Mode::Terminator => 0,
            },
            Version::Normal(_) => match mode {
                Mode::Numeric => 14,
                Mode::Alphanumeric => 13,
                Mode::Byte => 16,
                Mode::Kanji => 12,
                Mode::Eci | Mode::Terminator => 0,
            },
        }
    }

    pub fn data_bit_capacity(self, ecl: ECLevel, hi_cap: bool) -> usize {
        let mut bc = match self {
            Version::Micro(v) => VERSION_DATA_BIT_CAPACITY[39 + v][ecl as usize],
            Version::Normal(v) => VERSION_DATA_BIT_CAPACITY[v - 1][ecl as usize],
        };
        if hi_cap {
            bc *= 3;
        }
        bc
    }

    pub fn data_capacity(self, ecl: ECLevel, hi_cap: bool) -> usize {
        let mut bc = match self {
            Version::Micro(v) => VERSION_DATA_BIT_CAPACITY[39 + v][ecl as usize],
            Version::Normal(v) => VERSION_DATA_BIT_CAPACITY[v - 1][ecl as usize],
        };
        if hi_cap {
            bc *= 3;
        }
        bc >> 3
    }

    pub fn total_codewords(self, hi_cap: bool) -> usize {
        let mut tc = match self {
            Version::Micro(v) => VERSION_TOTAL_CODEWORDS[39 + v],
            Version::Normal(v) => VERSION_TOTAL_CODEWORDS[v - 1],
        };
        if hi_cap {
            tc *= 3;
        }
        tc
    }

    pub fn channel_data_capacity(self, ecl: ECLevel) -> usize {
        let bc = match self {
            Version::Micro(v) => VERSION_DATA_BIT_CAPACITY[39 + v][ecl as usize],
            Version::Normal(v) => VERSION_DATA_BIT_CAPACITY[v - 1][ecl as usize],
        };
        bc >> 3
    }

    pub fn channel_codewords(self) -> usize {
        match self {
            Version::Micro(v) => VERSION_TOTAL_CODEWORDS[39 + v],
            Version::Normal(v) => VERSION_TOTAL_CODEWORDS[v - 1],
        }
    }

    pub fn data_codewords_per_block(self, ecl: ECLevel) -> (usize, usize, usize, usize) {
        match self {
            Version::Micro(v) => DATA_CODEWORDS_PER_BLOCK[39 + v][ecl as usize],
            Version::Normal(v) => DATA_CODEWORDS_PER_BLOCK[v - 1][ecl as usize],
        }
    }

    pub fn ecc_per_block(self, ecl: ECLevel) -> usize {
        match self {
            Version::Micro(v) => ECC_PER_BLOCK[39 + v][ecl as usize],
            Version::Normal(v) => ECC_PER_BLOCK[v - 1][ecl as usize],
        }
    }

    pub fn remainder_bits(self) -> usize {
        match self {
            Version::Micro(_) | Version::Normal(1) => 0,
            Version::Normal(2..=6) => 7,
            Version::Normal(7..=13) => 0,
            Version::Normal(14..=20) => 3,
            Version::Normal(21..=27) => 4,
            Version::Normal(28..=34) => 3,
            Version::Normal(35..=40) => 0,
            _ => unreachable!("Invalid version"),
        }
    }

    pub fn info(self) -> u32 {
        debug_assert!(matches!(self, Version::Normal(7..=40)), "Invalid version");
        match self {
            Version::Normal(v) => VERSION_INFOS[v - 7],
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod version_tests {
    use super::Mode;
    use super::Version::*;

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_micro_version_low() {
        let bad_ver = Micro(0);
        bad_ver.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_micro_version_high() {
        let bad_ver = Micro(5);
        bad_ver.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_normal_version_low() {
        let bad_ver = Normal(0);
        bad_ver.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_width_invalid_normal_version_high() {
        let bad_ver = Normal(41);
        bad_ver.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_version_info_invalid_version_low() {
        let bad_ver = Normal(0);
        bad_ver.alignment_pattern();
    }

    #[test]
    #[should_panic(expected = "Invalid version")]
    fn test_version_info_invalid_version_high() {
        let bad_ver = Normal(41);
        bad_ver.alignment_pattern();
    }

    #[test]
    fn test_char_cnt_bits() {
        assert_eq!(Normal(1).char_cnt_bits(Mode::Numeric), 10);
        assert_eq!(Normal(9).char_cnt_bits(Mode::Numeric), 10);
        assert_eq!(Normal(10).char_cnt_bits(Mode::Numeric), 12);
        assert_eq!(Normal(26).char_cnt_bits(Mode::Numeric), 12);
        assert_eq!(Normal(27).char_cnt_bits(Mode::Numeric), 14);
        assert_eq!(Normal(40).char_cnt_bits(Mode::Numeric), 14);
        assert_eq!(Normal(1).char_cnt_bits(Mode::Alphanumeric), 9);
        assert_eq!(Normal(9).char_cnt_bits(Mode::Alphanumeric), 9);
        assert_eq!(Normal(10).char_cnt_bits(Mode::Alphanumeric), 11);
        assert_eq!(Normal(26).char_cnt_bits(Mode::Alphanumeric), 11);
        assert_eq!(Normal(27).char_cnt_bits(Mode::Alphanumeric), 13);
        assert_eq!(Normal(40).char_cnt_bits(Mode::Alphanumeric), 13);
        assert_eq!(Normal(1).char_cnt_bits(Mode::Byte), 8);
        assert_eq!(Normal(9).char_cnt_bits(Mode::Byte), 8);
        assert_eq!(Normal(10).char_cnt_bits(Mode::Byte), 16);
        assert_eq!(Normal(26).char_cnt_bits(Mode::Byte), 16);
        assert_eq!(Normal(27).char_cnt_bits(Mode::Byte), 16);
        assert_eq!(Normal(40).char_cnt_bits(Mode::Byte), 16);
    }

    #[test]
    #[should_panic]
    fn test_char_cnt_bits_invalid_version_low() {
        Normal(0).char_cnt_bits(Mode::Numeric);
    }

    #[test]
    #[should_panic]
    fn test_char_cnt_bits_invalid_version_high() {
        Normal(41).char_cnt_bits(Mode::Alphanumeric);
    }

    #[test]
    #[should_panic]
    fn test_char_cnt_bits_invalid_version_max() {
        Normal(usize::MAX).char_cnt_bits(Mode::Alphanumeric);
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

impl From<u8> for ECLevel {
    fn from(value: u8) -> Self {
        match value {
            0 => ECLevel::L,
            1 => ECLevel::M,
            2 => ECLevel::Q,
            3 => ECLevel::H,
            _ => panic!("Invalid u8 for ec level: {value}"),
        }
    }
}


// Color
//------------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Color {
    Black = 0b000,
    Red = 0b100,
    Green = 0b010,
    Blue = 0b001,
    Yellow = 0b110,  // Red + Green
    Magenta = 0b101, // Red + Blue
    Cyan = 0b011,    // Green + Blue
    White = 0b111,
}

impl TryFrom<u8> for Color {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value & 0b111 {
            0b000 => Ok(Color::Black),
            0b100 => Ok(Color::Red),
            0b010 => Ok(Color::Green),
            0b001 => Ok(Color::Blue),
            0b110 => Ok(Color::Yellow),
            0b101 => Ok(Color::Magenta),
            0b011 => Ok(Color::Cyan),
            0b111 => Ok(Color::White),
            _ => Err(()),
        }
    }
}

impl From<bool> for Color {
    fn from(value: bool) -> Self {
        if value {
            Color::White
        } else {
            Color::Black
        }
    }
}

impl Not for Color {
    type Output = Self;
    fn not(self) -> Self::Output {
        let bit = !(self as u8);
        Color::try_from(bit).unwrap()
    }
}

impl From<Color> for u32 {
    fn from(value: Color) -> Self {
        match value {
            Color::White => 0,
            Color::Black => 1,
            Color::Red | Color::Yellow | Color::Magenta => 0,
            _ => 1,
        }
    }
}

impl From<Color> for Rgb<u8> {
    fn from(value: Color) -> Self {
        match value {
            Color::Black => Rgb([0, 0, 0]),
            Color::Red => Rgb([255, 0, 0]),
            Color::Green => Rgb([0, 255, 0]),
            Color::Blue => Rgb([0, 0, 255]),
            Color::Yellow => Rgb([255, 255, 0]),
            Color::Magenta => Rgb([255, 0, 255]),
            Color::Cyan => Rgb([0, 255, 255]),
            Color::White => Rgb([255, 255, 255]),
        }
    }
}

impl TryFrom<Color> for Luma<u8> {
    type Error = ();

    fn try_from(value: Color) -> Result<Self, Self::Error> {
        match value {
            Color::Black => Ok(Luma([0])),
            Color::White => Ok(Luma([255])),
            _ => Err(()),
        }
    }
}

impl Color {
    pub fn select<T: Debug>(&self, light: T, dark: T) -> T {
        match self {
            Self::White => light,
            Self::Black => dark,
            _ => todo!(),
        }
    }
}

// Format information
//------------------------------------------------------------------------------

pub fn generate_format_info_qr(ecl: ECLevel, mask: MaskPattern) -> u32 {
    let format_data = ((ecl as usize) ^ 1) << 3 | (*mask as usize);
    FORMAT_INFOS_QR[format_data]
}

pub fn parse_format_info_qr(info: u32) -> (ECLevel, MaskPattern) {
    let ecl = ECLevel::from(((info >> 13) ^ 1) as u8);
    let mask = MaskPattern::new(((info >> 10) & 7) as u8);
    (ecl, mask)
}

// Global constants
//------------------------------------------------------------------------------

static ALIGNMENT_PATTERN_POSITIONS: [&[i32]; 40] = [
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

// Data bit capacity per error level per version
static VERSION_DATA_BIT_CAPACITY: [[usize; 4]; 44] = [
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
    // Micro versions
    [20, 0, 0, 0],
    [40, 32, 0, 0],
    [84, 68, 0, 0],
    [128, 112, 80, 0],
];

static VERSION_TOTAL_CODEWORDS: [usize; 44] = [
    26, 44, 70, 100, 134, 172, 196, 242, 292, 346, 404, 466, 532, 581, 655, 733, 815, 901, 991,
    1085, 1156, 1258, 1364, 1474, 1588, 1706, 1828, 1921, 2051, 2185, 2323, 2465, 2611, 2761, 2876,
    3034, 3196, 3362, 3532, 3706, //Micro versions
    5, 10, 17, 24,
];

static ECC_PER_BLOCK: [[usize; 4]; 44] = [
    // Normal versions.
    [7, 10, 13, 17],
    [10, 16, 22, 28],
    [15, 26, 18, 22],
    [20, 18, 26, 16],
    [26, 24, 18, 22],
    [18, 16, 24, 28],
    [20, 18, 18, 26],
    [24, 22, 22, 26],
    [30, 22, 20, 24],
    [18, 26, 24, 28],
    [20, 30, 28, 24],
    [24, 22, 26, 28],
    [26, 22, 24, 22],
    [30, 24, 20, 24],
    [22, 24, 30, 24],
    [24, 28, 24, 30],
    [28, 28, 28, 28],
    [30, 26, 28, 28],
    [28, 26, 26, 26],
    [28, 26, 30, 28],
    [28, 26, 28, 30],
    [28, 28, 30, 24],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [26, 28, 30, 30],
    [28, 28, 28, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    [30, 28, 30, 30],
    // Micro versions.
    [2, 0, 0, 0],
    [5, 6, 0, 0],
    [6, 8, 0, 0],
    [8, 10, 14, 0],
];

static DATA_CODEWORDS_PER_BLOCK: [[(usize, usize, usize, usize); 4]; 44] = [
    // Normal versions.
    [(19, 1, 0, 0), (16, 1, 0, 0), (13, 1, 0, 0), (9, 1, 0, 0)],
    [(34, 1, 0, 0), (28, 1, 0, 0), (22, 1, 0, 0), (16, 1, 0, 0)],
    [(55, 1, 0, 0), (44, 1, 0, 0), (17, 2, 0, 0), (13, 2, 0, 0)],
    [(80, 1, 0, 0), (32, 2, 0, 0), (24, 2, 0, 0), (9, 4, 0, 0)],
    [(108, 1, 0, 0), (43, 2, 0, 0), (15, 2, 16, 2), (11, 2, 12, 2)],
    [(68, 2, 0, 0), (27, 4, 0, 0), (19, 4, 0, 0), (15, 4, 0, 0)],
    [(78, 2, 0, 0), (31, 4, 0, 0), (14, 2, 15, 4), (13, 4, 14, 1)],
    [(97, 2, 0, 0), (38, 2, 39, 2), (18, 4, 19, 2), (14, 4, 15, 2)],
    [(116, 2, 0, 0), (36, 3, 37, 2), (16, 4, 17, 4), (12, 4, 13, 4)],
    [(68, 2, 69, 2), (43, 4, 44, 1), (19, 6, 20, 2), (15, 6, 16, 2)],
    [(81, 4, 0, 0), (50, 1, 51, 4), (22, 4, 23, 4), (12, 3, 13, 8)],
    [(92, 2, 93, 2), (36, 6, 37, 2), (20, 4, 21, 6), (14, 7, 15, 4)],
    [(107, 4, 0, 0), (37, 8, 38, 1), (20, 8, 21, 4), (11, 12, 12, 4)],
    [(115, 3, 116, 1), (40, 4, 41, 5), (16, 11, 17, 5), (12, 11, 13, 5)],
    [(87, 5, 88, 1), (41, 5, 42, 5), (24, 5, 25, 7), (12, 11, 13, 7)],
    [(98, 5, 99, 1), (45, 7, 46, 3), (19, 15, 20, 2), (15, 3, 16, 13)],
    [(107, 1, 108, 5), (46, 10, 47, 1), (22, 1, 23, 15), (14, 2, 15, 17)],
    [(120, 5, 121, 1), (43, 9, 44, 4), (22, 17, 23, 1), (14, 2, 15, 19)],
    [(113, 3, 114, 4), (44, 3, 45, 11), (21, 17, 22, 4), (13, 9, 14, 16)],
    [(107, 3, 108, 5), (41, 3, 42, 13), (24, 15, 25, 5), (15, 15, 16, 10)],
    [(116, 4, 117, 4), (42, 17, 0, 0), (22, 17, 23, 6), (16, 19, 17, 6)],
    [(111, 2, 112, 7), (46, 17, 0, 0), (24, 7, 25, 16), (13, 34, 0, 0)],
    [(121, 4, 122, 5), (47, 4, 48, 14), (24, 11, 25, 14), (15, 16, 16, 14)],
    [(117, 6, 118, 4), (45, 6, 46, 14), (24, 11, 25, 16), (16, 30, 17, 2)],
    [(106, 8, 107, 4), (47, 8, 48, 13), (24, 7, 25, 22), (15, 22, 16, 13)],
    [(114, 10, 115, 2), (46, 19, 47, 4), (22, 28, 23, 6), (16, 33, 17, 4)],
    [(122, 8, 123, 4), (45, 22, 46, 3), (23, 8, 24, 26), (15, 12, 16, 28)],
    [(117, 3, 118, 10), (45, 3, 46, 23), (24, 4, 25, 31), (15, 11, 16, 31)],
    [(116, 7, 117, 7), (45, 21, 46, 7), (23, 1, 24, 37), (15, 19, 16, 26)],
    [(115, 5, 116, 10), (47, 19, 48, 10), (24, 15, 25, 25), (15, 23, 16, 25)],
    [(115, 13, 116, 3), (46, 2, 47, 29), (24, 42, 25, 1), (15, 23, 16, 28)],
    [(115, 17, 0, 0), (46, 10, 47, 23), (24, 10, 25, 35), (15, 19, 16, 35)],
    [(115, 17, 116, 1), (46, 14, 47, 21), (24, 29, 25, 19), (15, 11, 16, 46)],
    [(115, 13, 116, 6), (46, 14, 47, 23), (24, 44, 25, 7), (16, 59, 17, 1)],
    [(121, 12, 122, 7), (47, 12, 48, 26), (24, 39, 25, 14), (15, 22, 16, 41)],
    [(121, 6, 122, 14), (47, 6, 48, 34), (24, 46, 25, 10), (15, 2, 16, 64)],
    [(122, 17, 123, 4), (46, 29, 47, 14), (24, 49, 25, 10), (15, 24, 16, 46)],
    [(122, 4, 123, 18), (46, 13, 47, 32), (24, 48, 25, 14), (15, 42, 16, 32)],
    [(117, 20, 118, 4), (47, 40, 48, 7), (24, 43, 25, 22), (15, 10, 16, 67)],
    [(118, 19, 119, 6), (47, 18, 48, 31), (24, 34, 25, 34), (15, 20, 16, 61)],
    // Micro versions.
    [(3, 1, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)], // M1
    [(5, 1, 0, 0), (4, 1, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)], // M2
    [(11, 1, 0, 0), (9, 1, 0, 0), (0, 0, 0, 0), (0, 0, 0, 0)], // M3
    [(16, 1, 0, 0), (14, 1, 0, 0), (10, 1, 0, 0), (0, 0, 0, 0)], // M4
];

pub static FORMAT_INFO_BIT_LEN: usize = 15;
pub static FORMAT_ERROR_CAPACITY: u32 = 3;

pub static FORMAT_MASK: u32 = 0b101010000010010;

pub static FORMAT_INFOS_QR: [u32; 32] = [
    0x5412, 0x5125, 0x5e7c, 0x5b4b, 0x45f9, 0x40ce, 0x4f97, 0x4aa0, 0x77c4, 0x72f3, 0x7daa, 0x789d,
    0x662f, 0x6318, 0x6c41, 0x6976, 0x1689, 0x13be, 0x1ce7, 0x19d0, 0x0762, 0x0255, 0x0d0c, 0x083b,
    0x355f, 0x3068, 0x3f31, 0x3a06, 0x24b4, 0x2183, 0x2eda, 0x2bed,
];

pub static FORMAT_INFO_COORDS_QR_MAIN: [(i32, i32); 15] = [
    (0, 8),
    (1, 8),
    (2, 8),
    (3, 8),
    (4, 8),
    (5, 8),
    (7, 8),
    (8, 8),
    (8, 7),
    (8, 5),
    (8, 4),
    (8, 3),
    (8, 2),
    (8, 1),
    (8, 0),
];

pub static FORMAT_INFO_COORDS_QR_SIDE: [(i32, i32); 15] = [
    (8, -1),
    (8, -2),
    (8, -3),
    (8, -4),
    (8, -5),
    (8, -6),
    (8, -7),
    (-8, 8),
    (-7, 8),
    (-6, 8),
    (-5, 8),
    (-4, 8),
    (-3, 8),
    (-2, 8),
    (-1, 8),
];

pub static VERSION_INFO_BIT_LEN: usize = 18;
pub static VERSION_ERROR_BIT_LEN: usize = 12;
pub static VERSION_ERROR_CAPACITY: u32 = 3;

pub static VERSION_INFOS: [u32; 34] = [
    0x07c94, 0x085bc, 0x09a99, 0x0a4d3, 0x0bbf6, 0x0c762, 0x0d847, 0x0e60d, 0x0f928, 0x10b78,
    0x1145d, 0x12a17, 0x13532, 0x149a6, 0x15683, 0x168c9, 0x177ec, 0x18ec4, 0x191e1, 0x1afab,
    0x1b08e, 0x1cc1a, 0x1d33f, 0x1ed75, 0x1f250, 0x209d5, 0x216f0, 0x228ba, 0x2379f, 0x24b0b,
    0x2542e, 0x26a64, 0x27541, 0x28c69,
];

pub static VERSION_INFO_COORDS_BL: [(i32, i32); 18] = [
    (5, -9),
    (5, -10),
    (5, -11),
    (4, -9),
    (4, -10),
    (4, -11),
    (3, -9),
    (3, -10),
    (3, -11),
    (2, -9),
    (2, -10),
    (2, -11),
    (1, -9),
    (1, -10),
    (1, -11),
    (0, -9),
    (0, -10),
    (0, -11),
];

pub static VERSION_INFO_COORDS_TR: [(i32, i32); 18] = [
    (-9, 5),
    (-10, 5),
    (-11, 5),
    (-9, 4),
    (-10, 4),
    (-11, 4),
    (-9, 3),
    (-10, 3),
    (-11, 3),
    (-9, 2),
    (-10, 2),
    (-11, 2),
    (-9, 1),
    (-10, 1),
    (-11, 1),
    (-9, 0),
    (-10, 0),
    (-11, 0),
];

pub const MAX_QR_SIZE: usize = 40960;
