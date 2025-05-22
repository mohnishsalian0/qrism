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
    SingularMatrix,
    PointAtInfinity,
    SymbolNotFound,
    TooManyError,
    InvalidInfo,
    InvalidFormatInfo,
    InvalidVersionInfo,
    FinderMismatch,
    TimingMismatch,
    AlignmentMismatch,
    InvalidUTF8Sequence,
}

impl Display for QRError {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        let msg = match *self {
            // QR builder
            Self::EmptyData => "Empty data",
            Self::DataTooLong => "Data too long",
            Self::CapacityOverflow => "Capacity overflow",
            Self::InvalidVersion => "Invalid version",
            Self::InvalidECLevel => "Invalid error correction level",
            Self::InvalidPalette => "Invalid color palette",
            Self::InvalidColor => "Invalid color",
            Self::InvalidChar => "Invalid character",
            Self::InvalidMaskingPattern => "Invalid masking pattern",

            // QR reader
            Self::SingularMatrix => "Cannot compute homography",
            Self::PointAtInfinity => "Projected point is at infinity",
            Self::SymbolNotFound => "Symbol not found",
            Self::TooManyError => "Too many errors to correct successfully",
            Self::InvalidInfo => "Invalid info",
            Self::InvalidFormatInfo => "Invalid format info detected",
            Self::InvalidVersionInfo => "Invalid version info detected",
            Self::FinderMismatch => "Finder color mismatch",
            Self::TimingMismatch => "Timing color mismatch",
            Self::AlignmentMismatch => "Alignment color mismatch",
            Self::InvalidUTF8Sequence => "Invalid UTF8 sequence",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for QRError {}

pub type QRResult<T> = Result<T, QRError>;
