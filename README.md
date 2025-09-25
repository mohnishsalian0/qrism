# qrism

[![Crates.io](https://img.shields.io/crates/v/qrism.svg)](https://crates.io/crates/qrism)
[![Documentation](https://docs.rs/qrism/badge.svg)](https://docs.rs/qrism)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

A Rust library for generating and reading QR codes with Reed-Solomon error correction. Supports traditional monochromatic QR codes with additional experimental multicolor QR support for **3x enhanced storage capacity**.

## Features

- **QR Code Generation**: Create QR codes with customizable versions, error correction levels, and capacity
- **QR Code Reading**: Detect and decode QR codes from images with robust error correction  
- **Reed-Solomon Error Correction**: Built-in error correction with configurable levels (L, M, Q, H)
- **High Capacity QR Support**: Experimental polychromatic QR codes with 3x storage capacity
- **Image Processing**: Advanced binarization and geometric correction for reliable detection

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
qrism = "0.1.0"
```

## Quick Start

### Simple QR Code Generation

```rust
use qrism::QRBuilder;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Simplest usage - provide only data, all other settings are automatically chosen
    let qr = QRBuilder::new(b"Hello, World!")
        .build()?;

    let img = qr.to_image(4);  // 4x scale factor
    img.save("simple_qr.png")?;
    Ok(())
}
```

### Reading a QR Code

```rust
use qrism::reader::detect_qr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load and prepare image
    let img = image::open("qr_code.png")?;

    // Detect and decode QR codes
    let mut res = detect_qr(&img);
    if let Some(symbol) = res.symbols().first_mut() {
        let (metadata, message) = symbol.decode()?;
        println!("Decoded: {}", message);
    }
    Ok(())
}
```

### High Capacity QR Codes (3x Storage)

High capacity QR codes achieve **3x the storage capacity** by leveraging color channels for data encoding. Unlike standard monochromatic QR codes that use only black and white modules, high capacity QR codes utilize the full RGB color spectrum by multiplexing three separate QR codes into a single visual code.

```rust
use qrism::QRBuilder;
use qrism::reader::detect_hc_qr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a high capacity QR code with 3x storage
    let large_data = "Large dataset that would not fit in a standard QR code...".repeat(10);
    let qr = QRBuilder::new(large_data.as_bytes())
        .high_capacity(true)  // Enable high capacity mode
        .build()?;

    let img = qr.to_image(4);
    img.save("high_capacity_qr.png")?;

    // Reading high capacity QR codes
    let img = image::open("high_capacity_qr.png")?;
    let mut res = detect_hc_qr(&img);
    if let Some(symbol) = res.symbols().first_mut() {
        let (metadata, message) = symbol.decode()?;
        println!("Decoded: {}", message);
    }
    
    Ok(())
}
```

## Error Correction Levels

- **L (Low)**: ~7% error correction
- **M (Medium)**: ~15% error correction (default)
- **Q (Quartile)**: ~25% error correction
- **H (High)**: ~30% error correction

## Examples

See the [`examples/`](examples/) directory for more comprehensive usage examples.

## License

This project is licensed under the MIT License - see the [LICENSE.txt](LICENSE.txt) file for details.

## Attribution

Test images used from the ZXing project (https://github.com/zxing/zxing), licensed under the Apache License 2.0. Attribution is provided in accordance with the license.
