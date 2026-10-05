use std::error::Error;
use std::path::Path;

use qrism::{detect_hc_qr, detect_qr, ECLevel, Version};
use qrism::{MaskPattern, QRBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    let payload: String = std::fs::read_to_string("./qr_payload.txt")
        .unwrap()
        .lines()
        .next()
        .map(String::from)
        .unwrap();

    // Create a QR code
    for v in [1, 5, 10, 15, 25, 30, 40] {
        for ecl in [ECLevel::L, ECLevel::M, ECLevel::Q, ECLevel::H] {
            for size in [8, 12, 20] {
                // let data = "Lorem ipsum dolor sit amet";
                let ver = Version::Normal(v);
                let data_cap = ver.data_capacity(ecl, true);
                let qr = QRBuilder::new(&payload.as_bytes()[..(data_cap - 3).min(payload.len())])
                    .version(Version::Normal(v)) // If not provided, finds smallest version to fit the data
                    .ec_level(ecl) // Defaults to ECLevel::M
                    .high_capacity(true) // Defaults to false, use true for high capacity QR
                    // .mask(MaskPattern::new(1)) // If not provided, finds best mask based on penalty score
                    .build()?;

                // Save QR code as image
                let img = qr.to_image(size); // scale factor for output image size
                let size_label = match size {
                    8 => 'S',
                    12 => 'M',
                    20 => 'L',
                    _ => panic!(),
                };
                let p = format!("./assets/{}-{:?}-{}.png", v, ecl, size_label);
                let output_path = Path::new(&p);
                img.save(output_path)?;
                println!("QR code saved to: {}", output_path.display());
            }
        }
    }

    // Read the QR code back
    // let read_path = Path::new("./assets/qr_example.png");
    // let img = image::open(read_path)?;
    // let mut res = detect_qr(&img);
    //
    // if let Some(symbol) = res.symbols().first_mut() {
    //     let (metadata, decoded_message) = symbol.decode()?;
    //     println!("Decoded message: {}", decoded_message);
    //     println!("QR metadata: {:?}", metadata);
    // } else {
    //     println!("No QR code found in the image");
    // }
    //
    // // Read high capacity QR code
    // let read_path = Path::new("./benches/dataset/high_capacity/xs1.jpeg");
    // let img = image::open(read_path)?;
    // let mut res = detect_hc_qr(&img);
    //
    // if let Some(symbol) = res.symbols().first_mut() {
    //     let (metadata, decoded_message) = symbol.decode()?;
    //     println!("Decoded message: {}", decoded_message);
    //     println!("High capacity QR metadata: {:?}", metadata);
    // } else {
    //     println!("No high capacity QR code found in the image");
    // }

    Ok(())
}
