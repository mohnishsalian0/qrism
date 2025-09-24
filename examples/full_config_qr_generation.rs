use std::error::Error;

use qrism::{ECLevel, MaskPattern, QRBuilder, Version};

fn main() -> Result<(), Box<dyn Error>> {
    let data = "This example shows all available configuration options for QR code generation.";

    let qr = QRBuilder::new(data.as_bytes())
        .version(Version::Normal(5)) // QR version (size) - if not provided, finds smallest version to fit data
        .ec_level(ECLevel::M) // Error correction level - if not provided, defaults to ECLevel::M
        .high_capacity(false) // High capacity mode - if not provided, defaults to false
        .mask(MaskPattern::new(2)) // Mask pattern - if not provided, finds best mask based on penalty score
        .build()?;

    // Convert to image and save
    let img = qr.to_image(5); // 5x scale factor for larger output
    img.save("./assets/configured_qr.png")?;

    println!("Configured QR code saved to: assets/configured_qr.png");
    println!("QR metadata: {}", qr.metadata());

    Ok(())
}
