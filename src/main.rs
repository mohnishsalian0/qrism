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
use rqrr::PreparedImage;

fn main() -> Result<(), Box<dyn Error>> {
    let data = "Well, hello there!";
    let ver = Version::Normal(16); // Size
    let ecl = ECLevel::L; // Error correction level
    let pal = Palette::Mono; // Color scheme: Monochromatic (traditional qr) or Polychromatic
    let mask = MaskPattern::new(1); // Mask pattern

    // QR Builder
    let qrb = QRBuilder::new(data.as_bytes())
        // .version(ver) // if not provided, finds smallest version to fit the data
        .ec_level(ecl)
        .palette(pal)
        // .mask(mask) // If not provided, finds best mask based on score
        .build()
        .unwrap();

    let img = qrb.to_image(10);
    let path = Path::new("D:/Rust/images/test1.png");
    img.save(path).unwrap();

    // QR Reader
    // let path = Path::new("assets/test1.png");
    // let img = image::open(path).unwrap().to_rgb8();
    // let msg = QRReader::read(img).unwrap();
    // println!("\x1b[1;32mMessage:\x1b[0m");
    // println!("{msg}");

    // let path = Path::new("assets/test1.png");
    // let img = image::open(path).unwrap().to_luma8();
    // let mut img = PreparedImage::prepare(img);
    // let grids = img.detect_grids();
    // assert!(!grids.is_empty());
    // let msg = grids[0].decode().unwrap();
    // println!("Message: {msg:?}");

    Ok(())
}
