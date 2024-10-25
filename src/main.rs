// TODO: Remember to remove unused_variables & dead_code
#![allow(clippy::items_after_test_module, unused_variables, dead_code)]

use std::error::Error;

use builder::QRBuilder;
use types::{ECLevel, Version};

mod builder;
mod encode;
mod error_correction;
pub mod mask;
pub mod qr;
pub mod types;

// TODO: Remove rqrr and clean up main function after testing
fn main() -> Result<(), Box<dyn Error>> {
    let data = "OK";

    let qr = QRBuilder::new(data.as_bytes())
        .version(Version::Normal(1))
        .ec_level(ECLevel::H)
        .build()
        .unwrap()
        .render_as_string(1);
    println!("{qr}");

    Ok(())
}
