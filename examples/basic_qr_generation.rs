use std::error::Error;

use qrism::QRBuilder;

fn main() -> Result<(), Box<dyn Error>> {
    // Simplest usage - provide only data, all other settings are automatically chosen
    let qr = QRBuilder::new(b"Hello, World!").build()?;

    // Convert to image and save
    let img = qr.to_image(4); // 4x scale factor
    img.save("./assets/simple_qr.png")?;

    println!("Simple QR code saved to: assets/simple_qr.png");
    Ok(())
}

