use std::error::Error;

use qrism::reader::detect_hc_qr;

fn main() -> Result<(), Box<dyn Error>> {
    // Read an existing high capacity QR code from the assets directory
    let qr_path = "assets/example4.png";
    let img = image::open(qr_path)?;

    // Detect and decode high capacity QR codes in the image
    let mut res = detect_hc_qr(&img);

    if let Some(symbol) = res.symbols().first_mut() {
        let (metadata, decoded_message) = symbol.decode()?;
        println!("Successfully decoded QR code from: {}", qr_path);
        println!("Decoded message: {}", decoded_message);
        println!("QR metadata: {:?}", metadata);
    } else {
        println!("No QR code found in the image: {}", qr_path);
    }

    Ok(())
}
