use std::error::Error;
use std::path::Path;

use qr_pro_max::reader::QRReader;
use qr_pro_max::QRBuilder;
use qr_pro_max::{ECLevel, Palette, Version};

fn main() -> Result<(), Box<dyn Error>> {
    let data = "Hello, world!";
    let version = Version::Normal(3);
    let ec_level = ECLevel::L;

    let qrb = QRBuilder::new(data.as_bytes())
        .version(Version::Normal(3))
        .ec_level(ECLevel::L)
        .palette(Palette::Poly)
        .build()
        .unwrap();

    let path = Path::new("assets/output.png");
    let image = qrb.render_color(10);
    image.save(path).expect("Failed to save image");

    // let extracted_data = QRReader::read_from_str(&qrb.to_str(1), version).unwrap();
    // println!("Extracted Data: {extracted_data}");

    // FIXME: Remove
    // let path = "assets/test_image_1.png";
    // let img = image::open(path)?.to_luma8();
    // let mut img = rqrr::PreparedImage::prepare(img);
    // let grids = img.detect_grids();
    // assert_eq!(grids.len(), 1);
    // let (meta, content) = grids[0].decode().unwrap();
    // println!("{content}");

    Ok(())
}
