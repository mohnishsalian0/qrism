#![allow(
    clippy::items_after_test_module,
    unused_imports,
    unused_variables,
    dead_code,
    mixed_script_confusables
)]

use std::error::Error;
use std::path::Path;

use image::{GrayImage, Luma, RgbImage};
use qrism::reader::QRReader;
use qrism::{ECLevel, Palette, Version};
use qrism::{MaskPattern, QRBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    let data = "Well, hello there!";
    let version = Version::Normal(16);
    let ec_level = ECLevel::L;
    let pal = Palette::Poly;
    let mask_pattern = MaskPattern::new(1);

    let qrb = QRBuilder::new(data.as_bytes())
        // .version(version)
        .ec_level(ec_level)
        .palette(pal)
        .mask(mask_pattern)
        .build()
        .unwrap();

    let path = Path::new("assets/test.png");
    let img = qrb.to_image(10);
    // image.save(path).expect("Failed to save image");

    // let path = Path::new("assets/test.jpg");
    // let img = image::open(path)?.to_rgb8();
    let extracted_data = QRReader::read(img).unwrap();
    println!("\x1b[1;32mMessage:\x1b[0m");
    println!("{extracted_data}");

    Ok(())
}
