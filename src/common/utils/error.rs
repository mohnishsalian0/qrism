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
    CastingFailed,
    PixelOutOfBounds,
    TooManyError,
    InvalidInfo,
    InvalidFormatInfo,
    InvalidVersionInfo,
    InvalidPaletteInfo,
    FinderMismatch,
    TimingMismatch,
    AlignmentMismatch,
    DivisionByZero,
    InvalidMode(u8),
    CorruptDataSegment,
    EndOfStream,
    InvalidUTF8Encoding,
    InvalidCharacterEncoding,
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
            Self::SymbolNotFound => "QR not found",
            Self::CastingFailed => "f64 to i32 casting failed",
            Self::PixelOutOfBounds => "Pixel is out of bounds",
            Self::TooManyError => "Too many errors",
            Self::InvalidInfo => "Invalid info",
            Self::InvalidFormatInfo => "Invalid format info detected",
            Self::InvalidVersionInfo => "Invalid version info detected",
            Self::InvalidPaletteInfo => "Invalid palette info detected",
            Self::FinderMismatch => "Finder color mismatch",
            Self::TimingMismatch => "Timing color mismatch",
            Self::AlignmentMismatch => "Alignment color mismatch",
            Self::DivisionByZero => "Division by zero in GF(256)",
            Self::InvalidMode(m) => &format!("Unexpected mode bits: {m}").to_string(),
            Self::CorruptDataSegment => "Truncated data segment",
            Self::EndOfStream => "End of stream reached",
            Self::InvalidUTF8Encoding => "Invalid UTF8 sequence",
            Self::InvalidCharacterEncoding => "Character sequence is neither utf8 nor shift jis",
        };
        f.write_str(msg)
    }
}

impl std::error::Error for QRError {}

pub type QRResult<T> = Result<T, QRError>;
