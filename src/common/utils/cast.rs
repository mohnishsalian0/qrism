use num_traits::ToPrimitive;

use super::{QRError, QRResult};

pub fn f64_to_u32(num: &f64) -> QRResult<u32> {
    num.round().to_u32().ok_or(QRError::CastingFailed)
}
