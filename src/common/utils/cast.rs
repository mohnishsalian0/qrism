use super::{QRError, QRResult};

pub fn f64_to_i32(num: &f64) -> QRResult<i32> {
    let num = *num;

    if num < i32::MIN as f64 || num > i32::MAX as f64 {
        return Err(QRError::CastingFailed);
    }

    Ok(num as i32)
}
