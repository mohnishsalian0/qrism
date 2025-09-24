use std::error::Error;

use qrism::{ECLevel, MaskPattern, QRBuilder, Version};

fn main() -> Result<(), Box<dyn Error>> {
    let data = "This example shows all available configuration options for QR code generation.";
    
    let qr = QRBuilder::new(data.as_bytes())
        .version(Version::Normal(3))  // QR version (size) - if not provided, finds smallest version to fit data
        .ec_level(ECLevel::H)         // Error correction level - if not provided, defaults to ECLevel::M
        .high_capacity(false)         // High capacity mode - if not provided, defaults to false
        .mask(MaskPattern::new(2))    // Mask pattern - if not provided, finds best mask based on penalty score
        .build()?;

    // Convert to image and save
    let img = qr.to_image(6); // 6x scale factor for larger output
    img.save("configured_qr.png")?;
    
    println!("Configured QR code saved to: configured_qr.png");
    println!("QR metadata: {}", qr.metadata());
    
    Ok(())
}