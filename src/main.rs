use std::error::Error;
use std::path::Path;

use qr_pro_max::reader::QRReader;
use qr_pro_max::{ECLevel, Palette, Version};
use qr_pro_max::{MaskPattern, QRBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    let data = "Hello, world! HeHello, world! HeHello, world!";
    let version = Version::Normal(1);
    let ec_level = ECLevel::L;
    let mask_pattern = MaskPattern::new(1);

    let qrb = QRBuilder::new(data.as_bytes())
        .version(version)
        .ec_level(ec_level)
        .palette(Palette::Poly)
        .mask(mask_pattern)
        .build()
        .unwrap();

    let path = Path::new("assets/output.png");
    let image = qrb.render_color(10);
    image.save(path).expect("Failed to save image");

    let img = image::open(path)?.to_rgb8();
    let extracted_data = QRReader::read(&img, version).unwrap();
    println!("Extracted Data: {extracted_data}");

    // Example with rqrr
    // let img = image::open(path)?.to_luma8();
    // let mut img = rqrr::PreparedImage::prepare(img);
    // let grids = img.detect_grids();
    // assert_eq!(grids.len(), 1);
    // let (meta, content) = grids[0].decode().unwrap();
    // println!("{content}");

    Ok(())
}
