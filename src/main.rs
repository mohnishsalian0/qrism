#![allow(
    clippy::items_after_test_module,
    unused_imports,
    unused_variables,
    dead_code,
    mixed_script_confusables
)]

use std::error::Error;
use std::path::Path;

use qr_pro_max::reader::QRReader;
use qr_pro_max::{ECLevel, Palette, Version};
use qr_pro_max::{MaskPattern, QRBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    let data = "ABCDEFGHIJKLMNOPQRTSUVWXYZABCDEFGHIJKLMNOPQRTSUVWXYZABCDEFGHIJKLMNOPQRTSUVWXYZABCDEFGHIJKLMNOPQRTSUVWXYZABCDEFGHIJKLMNOPQRTSUVWXYZABCDEFGHIJKLMNOPQRTSUVWXYZ";
    let version = Version::Normal(16);
    let ec_level = ECLevel::L;
    let pal = Palette::Poly;
    let mask_pattern = MaskPattern::new(1);

    // let qrb = QRBuilder::new(data.as_bytes())
    //     // .version(version)
    //     .ec_level(ec_level)
    //     .palette(pal)
    //     .mask(mask_pattern)
    //     .build()
    //     .unwrap();
    //
    // let path = Path::new("assets/test6.png");
    // let image = qrb.to_image(10);
    // image.save(path).expect("Failed to save image");

    let path = Path::new("assets/camera3.jpg");
    let img = image::open(path)?.to_rgb8();
    let extracted_data = QRReader::read_from_image(img).unwrap();
    println!("Extracted Data: {extracted_data}");

    // // Example with rqrr
    // let path = Path::new("assets/test5.png");
    // let img = image::open(path)?.to_luma8();
    // let mut img = rqrr::PreparedImage::prepare(img);
    // let grids = img.detect_grids();
    // assert_eq!(grids.len(), 1);
    // let (meta, content) = grids[0].decode().unwrap();
    // println!("{content}");

    Ok(())
}
