use std::error::Error;

use qrism::{ECLevel, QRBuilder, Version};

fn main() -> Result<(), Box<dyn Error>> {
    let data = "This is an experimental multicolor QR code with enhanced storage capacity!";
    
    // Create a high capacity QR code
    let qr = QRBuilder::new(data.as_bytes())
        .version(Version::Normal(3))
        .ec_level(ECLevel::M)
        .high_capacity(true)  // This enables high capacity mode
        .build()?;

    // Convert to image and save
    let img = qr.to_image(5); // 5x scale factor
    img.save("multicolor_qr.png")?;
    
    println!("High capacity QR code saved to: multicolor_qr.png");
    println!("QR metadata: {}", qr.metadata());
    println!("Note: This is experimental high capacity QR support with 3x storage");
    
    Ok(())
}