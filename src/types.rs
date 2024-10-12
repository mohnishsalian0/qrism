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
    Micro(i16),
    Normal(i16),
}

impl Version {
    pub const fn width(self) -> i16 {
        match self {
            Self::Micro(v) => v * 2 + 9,
            Self::Normal(v) => v * 4 + 17,
        }
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
    Multicolor(i16),
}

// Mode
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum Mode {
    Numeric,
    Alphanumeric,
    Byte,
    Kanji,
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
