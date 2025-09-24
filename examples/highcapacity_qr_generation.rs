use std::error::Error;

use qrism::{ECLevel, Palette, QRBuilder, Version};

fn main() -> Result<(), Box<dyn Error>> {
    let data = "This is an experimental multicolor QR code with enhanced storage capacity!";
    
    // Create a multicolor (polychromatic) QR code
    let qr = QRBuilder::new(data.as_bytes())
        .version(Version::Normal(3))
        .ec_level(ECLevel::M)
        .palette(Palette::Poly)  // This enables multicolor mode
        .build()?;

    // Convert to image and save
    let img = qr.to_image(5); // 5x scale factor
    img.save("multicolor_qr.png")?;
    
    println!("Multicolor QR code saved to: multicolor_qr.png");
    println!("QR metadata: {}", qr.metadata());
    println!("Note: This is experimental multicolor QR support for enhanced capacity");
    
    Ok(())
}