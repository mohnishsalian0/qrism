use std::error::Error;

use qrism::reader::{binarize::BinaryImage, detect};

fn main() -> Result<(), Box<dyn Error>> {
    // Read an existing QR code from the assets directory
    let qr_path = "assets/example4.png";
    let rgb_img = image::open(qr_path)?.to_rgb8();
    let mut binary_img = BinaryImage::prepare(&rgb_img);

    // Detect and decode QR codes in the image
    let mut symbols = detect(&mut binary_img);

    if let Some(symbol) = symbols.first_mut() {
        let (metadata, decoded_message) = symbol.decode()?;
        println!("Successfully decoded QR code from: {}", qr_path);
        println!("Decoded message: {}", decoded_message);
        println!("QR metadata: {:?}", metadata);
    } else {
        println!("No QR code found in the image: {}", qr_path);
    }

    Ok(())
}
