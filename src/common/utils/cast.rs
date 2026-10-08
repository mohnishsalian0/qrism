use num_traits::ToPrimitive;

use super::{QRError, QRResult};

pub fn f64_to_i32(num: &f64) -> QRResult<i32> {
    num.to_i32().ok_or(QRError::CastingFailed)
}
