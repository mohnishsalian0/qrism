use std::fmt::{Debug, Display, Error, Formatter};

// Error
//------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum QRError {
    // QR builder
    EmptyData,
    DataTooLong,
    CapacityOverflow,
    InvalidVersion,
    InvalidECLevel,
    InvalidPalette,
    InvalidColor,
    InvalidChar,
    InvalidMaskingPattern,

    // QR reader
    ErrorDetected([u8; 64]),
    InvalidInfo,
    InvalidFormatInfo,
    InvalidVersionInfo,
}

impl Display for QRError {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        let msg = match *self {
            Self::EmptyData => "empty data",
            Self::DataTooLong => "data too long",
            Self::CapacityOverflow => "capacity overflow",
            Self::InvalidVersion => "invalid version",
            Self::InvalidECLevel => "invalid error correction level",
            Self::InvalidPalette => "invalid color palette",
            Self::InvalidColor => "invalid color",
            Self::InvalidChar => "invalid character",
            Self::InvalidMaskingPattern => "invalid masking pattern",
            Self::ErrorDetected(_) => "Error detected in data",
            Self::InvalidInfo => "Invalid info",
            Self::InvalidFormatInfo => "Invalid format info detected",
            Self::InvalidVersionInfo => "Invalid version info detected",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for QRError {}

pub type QRResult<T> = Result<T, QRError>;
