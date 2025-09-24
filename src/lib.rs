//! # qrism
//!
//! A Rust library for generating and reading QR codes with Reed-Solomon error correction.
//! Supports traditional monochromatic QR codes with additional experimental multicolor QR
//! support for enhanced storage capacity.
//!
//! ## Features
//!
//! - **QR Code Generation**: Create QR codes with customizable versions, error correction levels, and palettes
//! - **QR Code Reading**: Detect and decode QR codes from images with robust error correction
//! - **Reed-Solomon Error Correction**: Built-in error correction with configurable levels (L, M, Q, H)
//! - **High Capacity QR Support**: Experimental polychromatic QR codes with 3x storage capacity
//! - **Image Processing**: Advanced binarization and geometric correction for reliable detection
//!
//! ## Quick Start
//!
//! ### Simple QR Code Generation
//!
//! ```rust
//! use qrism::QRBuilder;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Simplest usage - provide only data, all other settings are automatically chosen
//! let qr = QRBuilder::new(b"Hello, World!")
//!     .build()?;
//!
//! let img = qr.to_image(4);  // 4x scale factor
//! img.save("simple_qr.png")?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Full Configuration
//!
//! ```rust
//! use qrism::{QRBuilder, ECLevel, Palette, Version, MaskPattern};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let data = "Hello, World!";
//! let qr = QRBuilder::new(data.as_bytes())
//!     .version(Version::Normal(2))  // QR version (size) - if not provided, finds smallest version to fit data
//!     .ec_level(ECLevel::M)         // Error correction level - if not provided, defaults to ECLevel::M
//!     .palette(Palette::Mono)       // Color scheme - if not provided, defaults to Palette::Mono
//!     .mask(MaskPattern::new(3))    // Mask pattern - if not provided, finds best mask based on penalty score
//!     .build()?;
//!
//! let img = qr.to_image(4);  // 4x scale factor
//! img.save("configured_qr.png")?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Reading a QR Code
//!
//! ```rust,no_run
//! use qrism::reader::{binarize::BinaryImage, detect};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load and prepare image
//! let img = image::open("qr_code.png")?.to_rgb8();
//! let mut binary_img = BinaryImage::prepare(&img);
//!
//! // Detect and decode QR codes
//! let symbols = detect(&mut binary_img);
//! if let Some(symbol) = symbols.first() {
//!     let (metadata, message) = symbol.decode()?;
//!     println!("Decoded: {}", message);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## QR Code Components
//!
//! ### Versions
//! - **Micro QR**: Versions 1-4 for small data (experimental)
//! - **Normal QR**: Versions 1-40, with sizes from 21x21 to 177x177 modules
//!
//! ### Error Correction Levels
//! - **L (Low)**: ~7% error correction
//! - **M (Medium)**: ~15% error correction  
//! - **Q (Quartile)**: ~25% error correction
//! - **H (High)**: ~30% error correction
//!
//! ### Palettes
//! - **Mono**: Traditional black and white QR codes
//! - **Poly**: High capacity multicolor QR codes with 3x storage capacity
//!
//! ## High Capacity QR Codes
//!
//! High capacity QR codes are an extension of traditional QR codes that achieve **3x the storage capacity**
//! by leveraging color channels for data encoding. Unlike standard monochromatic QR codes that use only black and white modules,
//! high capacity QR codes utilize the full RGB color spectrum.
//!
//! ### How It Works
//!
//! The technology works by **multiplexing three separate QR codes** into a single visual code by
//! encoding one in each of the red, green and blue color channels.
//! Each color channel carries its own independent QR code with full Reed-Solomon error correction.
//! When decoded, the three separate data streams are combined to reconstruct the original data,
//! effectively tripling the storage capacity compared to traditional QR codes.
//!
//! ### Example Usage
//!
//! ```rust
//! use qrism::{QRBuilder, Palette};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a high capacity QR code with 3x storage
//! let large_data = "Large dataset that would not fit in a standard QR code...".repeat(10);
//! let qr = QRBuilder::new(large_data.as_bytes())
//!     .palette(Palette::Poly)  // Enable high capacity mode
//!     .build()?;
//!
//! let img = qr.to_image(4);
//! img.save("high_capacity_qr.png")?;
//! # Ok(())
//! # }
//! ```

#![allow(
    clippy::items_after_test_module,
    unused_variables,
    dead_code,
    mixed_script_confusables,
    clippy::suspicious_arithmetic_impl,
    clippy::suspicious_op_assign_impl
)]

pub mod builder;
pub(crate) mod common;
pub mod reader;

pub use builder::QRBuilder;
pub use common::mask::MaskPattern;
pub use common::metadata::{ECLevel, Palette, Version};
pub(crate) use common::*;
pub use reader::*;

#[cfg(test)]
pub(crate) use builder::Module;

#[cfg(feature = "benchmark")]
pub use common::utils::benchmark;
