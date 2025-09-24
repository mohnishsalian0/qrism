use std::error::Error;
use std::path::Path;

use qrism::reader::binarize::BinaryImage;
use qrism::reader::detect;
use qrism::{ECLevel, Version};
use qrism::{MaskPattern, QRBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    // Create a QR code
    // let data = "Hello, qrism! This is a demonstration of QR code generation and reading.";
    // let qr = QRBuilder::new(data.as_bytes())
    //     .version(Version::Normal(5)) // If not provided, finds smallest version to fit the data
    //     .ec_level(ECLevel::M) // Defaults to ECLevel::M
    //     .high_capacity(false) // Defaults to false, use true for high capacity QR
    //     .mask(MaskPattern::new(1)) // If not provided, finds best mask based on penalty score
    //     .build()?;

    // Save QR code as image
    // let img = qr.to_image(4); // scale factor for output image size
    // let output_path = Path::new("./assets/example6.png");
    // img.save(output_path)?;
    // println!("QR code saved to: {}", output_path.display());

    // Read the QR code back
    let read_path = Path::new("./assets/example6.png");
    let rgb_img = image::open(read_path)?.to_rgb8();
    let mut binary_img = BinaryImage::prepare(&rgb_img);
    let mut symbols = detect(&mut binary_img);

    if let Some(symbol) = symbols.first_mut() {
        let (metadata, decoded_message) = symbol.decode()?;
        println!("Decoded message: {}", decoded_message);
        println!("QR metadata: {:?}", metadata);
    } else {
        println!("No QR code found in the image");
    }

    Ok(())
}
