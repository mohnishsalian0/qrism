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
use qrism::reader::binarize::BinaryImage;
use qrism::reader::detect;
use qrism::{ECLevel, Palette, Version};
use qrism::{MaskPattern, QRBuilder};
use rqrr::PreparedImage;

fn main() -> Result<(), Box<dyn Error>> {
    let data = "Winter is arriving";
    let ver = Version::Normal(21); // Size
    let ecl = ECLevel::L; // Error correction level
    let pal = Palette::Mono; // Color scheme: Monochromatic (traditional qr) or Polychromatic
    let mask = MaskPattern::new(5); // Mask pattern

    // QR Builder
    let qrb = QRBuilder::new(data.as_bytes())
        .version(ver) // if not provided, finds smallest version to fit the data
        .ec_level(ecl)
        .palette(pal)
        // .mask(mask) // If not provided, finds best mask based on score
        .build()
        .unwrap();

    let img = qrb.to_image(3);
    let path = Path::new("benches/dataset/decoding/version21.png");
    img.save(path).unwrap();

    // QR Reader
    // let path = Path::new("assets/test7.png");
    // let img = image::open(path).unwrap().to_rgb8();
    // let mut bin_img = BinaryImage::prepare(&img);
    // let mut symbols = detect(&mut bin_img);
    // assert!(symbols.len() > 0, "No symbol found");
    // let msg = symbols[0].decode().unwrap();
    // println!("\x1b[1;32mMessage:\x1b[0m");
    // println!("{msg}");

    // RQRR
    // let path = std::path::Path::new("benches/dataset/detection/monitor/image001.jpg");
    // let img = image::open(path).unwrap().to_luma8();
    // let mut img = PreparedImage::prepare(img);
    // let grids = img.detect_grids();
    // assert!(!grids.is_empty());
    // let msg = grids[0].decode().unwrap();
    // println!("Message: {msg:?}");

    use imageproc::distance_transform::Norm;
    use imageproc::morphology::{close, open};

    // Fill white noise in black regions
    let path = std::path::Path::new("assets/inp.png");
    let img = image::open(path).unwrap().to_luma8();
    let closed = close(&img, Norm::L1, 1);

    // Optionally remove white specks after
    let cleaned = open(&closed, Norm::L1, 1);
    cleaned.save("assets/cleaned.png").expect("Failed to save image");

    Ok(())
}
