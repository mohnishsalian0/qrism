// TODO: Remember to remove unused_variables & dead_code
// #![allow(clippy::items_after_test_module, unused_variables, dead_code)]
#![allow(clippy::items_after_test_module)]

use std::error::Error;

use builder::QRBuilder;
use types::{ECLevel, Version};

mod builder;
mod encode;
mod error_correction;
pub mod mask;
pub mod qr;
pub mod types;

fn main() -> Result<(), Box<dyn Error>> {
    let data = "Hello, world! 🌏";

    let qr = QRBuilder::new(data.as_bytes())
        .version(Version::Normal(3))
        .ec_level(ECLevel::H)
        .build()
        .unwrap()
        .render_as_string(1);
    println!("{qr}");

    Ok(())
}
