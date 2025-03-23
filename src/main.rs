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
    let data = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Nam rhoncus tempor erat, nec luctus leo pharetra a. Ut dignissim lacus ut iaculis vehicula. Phasellus nec consequat est, vel lobortis nisl. Proin id imperdiet eros, vitae pharetra odio. Curabitur blandit id ipsum a efficitur. Ut aliquam mollis varius. Quisque suscipit aliquet augue malesuada lacinia. Vivamus non quam lectus. Duis eu purus eget mi egestas vulputate id sit amet mauris. Donec ut enim sed lorem ultricies egestas. Nam feugiat ipsum eu nunc gravida, nec luctus ipsum viverra. Pellentesque at massa non nulla consectetur eleifend vitae a ante. Suspendisse aliquam condimentum eros, et pulvinar tellus maximus a. Aenean euismod accumsan dolor commodo luctus. Praesent in nibh nunc. In dictum ante ut massa fringilla, sed hendrerit lorem consequat.";
    let version = Version::Normal(1);
    let ec_level = ECLevel::L;
    let mask_pattern = MaskPattern::new(1);

    let qrb = QRBuilder::new(data.as_bytes())
        // .version(version)
        .ec_level(ec_level)
        .palette(Palette::Poly)
        .mask(mask_pattern)
        .build()
        .unwrap();
    let version = qrb.version();

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
