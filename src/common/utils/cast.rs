pub fn f64_to_i32(num: &f64) -> i32 {
    let num = *num;

    assert!(num <= i32::MAX as f64);
    assert!(num >= i32::MIN as f64);

    num as i32
}
