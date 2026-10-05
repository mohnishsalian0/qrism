//! # qrism
//!
//! A Rust library for generating and reading QR codes with Reed-Solomon error correction.
//! Supports traditional monochromatic QR codes with additional experimental multicolor QR
//! support for enhanced storage capacity.
//!
//! ## Features
//!
//! - **QR Code Generation**: Create QR codes with customizable versions, error correction levels, and capacity
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
//! use qrism::{QRBuilder, ECLevel, Version, MaskPattern};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let data = "Hello, World!";
//! let qr = QRBuilder::new(data.as_bytes())
//!     .version(Version::Normal(2))  // QR version (size) - if not provided, finds smallest version to fit data
//!     .ec_level(ECLevel::M)         // Error correction level - if not provided, defaults to ECLevel::M
//!     .high_capacity(false)         // Standard capacity mode - if not provided, defaults to false
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
//! use qrism::reader::detect_qr;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load and prepare image
//! let img = image::open("qr_code.png")?;
//!
//! // Detect and decode QR codes
//! let mut res = detect_qr(&img);
//! if let Some(symbol) = res.symbols().first_mut() {
//!     let (metadata, message) = symbol.decode()?;
//!     println!("Decoded: {}", message);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Reading a high capacity multi color QR
//!
//! ```rust,no_run
//! use qrism::reader::detect_hc_qr;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Load and prepare image
//! let img = image::open("qr_code.png")?;
//!
//! // Detect and decode QR codes
//! let mut res = detect_hc_qr(&img);
//! if let Some(symbol) = res.symbols().first_mut() {
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
//! use qrism::QRBuilder;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create a high capacity QR code with 3x storage
//! let large_data = "Large dataset that would not fit in a standard QR code...".repeat(10);
//! let qr = QRBuilder::new(large_data.as_bytes())
//!     .high_capacity(true)  // Enable high capacity mode
//!     .build()?;
//!
//! let img = qr.to_image(4);
//! img.save("high_capacity_qr.png")?;
//! # Ok(())
//! # }
//! ```

#![allow(
    clippy::items_after_test_module,
    // FIXME: Uncomment below
    // unused_variables,
    // dead_code,
    mixed_script_confusables,
    clippy::suspicious_arithmetic_impl,
    clippy::suspicious_op_assign_impl
)]

pub mod builder;
pub(crate) mod common;
pub mod reader;

pub use builder::QRBuilder;
pub use common::mask::MaskPattern;
pub use common::metadata::{Color, ECLevel, Version};
pub(crate) use common::*;
pub use reader::*;

#[cfg(test)]
pub(crate) use builder::Module;

/// Benchmark-only access to crate-private pieces the colour bench needs: grid format reads and
/// a per-block high-capacity decode. Kept behind a thin wrapper so internal types stay free to
/// change.
#[cfg(feature = "benchmark")]
pub mod bench_hooks {
    use crate::common::codec::decode as codec_decode;
    use crate::common::ec::rectify_info;
    use crate::common::metadata::{
        parse_format_info_qr, FORMAT_ERROR_CAPACITY, FORMAT_INFOS_QR, FORMAT_INFO_COORDS_QR_MAIN,
        FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK,
    };
    use crate::common::utils::{BitStream, EncRegionIter};
    use crate::reader::symbol::deinterleave;
    use crate::{Color, ECLevel, MaskPattern, Version};

    /// Reads the format info off a module grid indexed `[y][x]`, the way `Symbol` reads it
    /// off an image: any non-white module is dark. Tries the main copy, then the side copy.
    pub fn read_grid_format(grid: &[Vec<Color>]) -> Option<(ECLevel, MaskPattern)> {
        let w = grid.len() as i32;
        let number = |coords: &[(i32, i32)]| {
            coords.iter().fold(0u32, |num, &(x, y)| {
                let (x, y) = (x.rem_euclid(w) as usize, y.rem_euclid(w) as usize);
                (num << 1) | (grid[y][x] != Color::White) as u32
            })
        };
        [&FORMAT_INFO_COORDS_QR_MAIN, &FORMAT_INFO_COORDS_QR_SIDE].into_iter().find_map(|c| {
            let (format, _) =
                rectify_info(number(c), &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY).ok()?;
            Some(parse_format_info_qr(format ^ FORMAT_MASK))
        })
    }

    /// Where a high-capacity grid decode gave out, once its format was known.
    pub enum Payload {
        /// At least one Reed-Solomon block, in some channel, could not be corrected.
        RsFailed,
        /// Every block corrected, but the codec rejected the bit stream.
        CodecFailed,
        Decoded(String),
    }

    /// A high-capacity decode broken out per channel and per Reed-Solomon block, which
    /// `Symbol::decode` folds into a single pass/fail. Channels are indexed R, G, B.
    pub struct GridDecode {
        /// Each block's codewords as received — data then parity, deinterleaved — before
        /// correction. Diffing these against a clean render's counts codeword errors exactly.
        pub received: [Vec<Vec<u8>>; 3],
        /// Whether each block corrected.
        pub block_ok: [Vec<bool>; 3],
        pub ec_len: usize,
        pub payload: Payload,
    }

    /// Decodes a high-capacity module grid with a known EC level and mask, mirroring
    /// `Symbol::decode` but keeping every block's outcome rather than stopping at the first
    /// failure.
    pub fn decode_hc_grid(
        grid: &[Vec<Color>],
        ver: Version,
        ecl: ECLevel,
        mask: MaskPattern,
    ) -> GridDecode {
        let w = grid.len() as i32;
        let mask_fn = mask.mask_functions();
        let chan_cap = ver.channel_codewords();
        let chan_bits = chan_cap << 3;
        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);

        // One bit stream per channel. `Color as u8` packs R<<2 | G<<1 | B.
        let mut bits = [vec![false; chan_bits], vec![false; chan_bits], vec![false; chan_bits]];
        for (i, (x, y)) in EncRegionIter::new(ver).take(chan_bits).enumerate() {
            let (xu, yu) = (x.rem_euclid(w) as usize, y.rem_euclid(w) as usize);
            let rgb = grid[yu][xu] as u8;
            for (ch, chan) in bits.iter_mut().enumerate() {
                chan[i] = ((rgb >> (2 - ch)) & 1 == 1) != !mask_fn(x, y);
            }
        }

        let mut received: [Vec<Vec<u8>>; 3] = Default::default();
        let mut block_ok: [Vec<bool>; 3] = Default::default();
        let mut enc = BitStream::new(chan_cap * 3 * 8);
        for ch in 0..3 {
            let bytes: Vec<u8> = bits[ch]
                .chunks_exact(8)
                .map(|b| b.iter().fold(0u8, |acc, &bit| (acc << 1) | bit as u8))
                .collect();

            for mut blk in deinterleave(&bytes, blk_info, ec_len) {
                received[ch].push(blk.full().to_vec());
                let fixed = blk.rectify().map(|d| d.to_vec());
                block_ok[ch].push(fixed.is_ok());
                if let Ok(d) = fixed {
                    enc.extend(&d);
                }
            }
        }

        let payload = if block_ok.iter().flatten().all(|&ok| ok) {
            match codec_decode(&mut enc, ver, ecl, true) {
                Ok(msg) => Payload::Decoded(msg),
                Err(_) => Payload::CodecFailed,
            }
        } else {
            Payload::RsFailed
        };

        GridDecode { received, block_ok, ec_len, payload }
    }
}
