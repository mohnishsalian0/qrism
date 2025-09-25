pub mod binarize;
mod finder;
pub mod symbol;
mod utils;

use std::{collections::HashSet, sync::Arc};

use finder::{group_finders, locate_finders, FinderGroup};

use binarize::BinaryImage;
use image::DynamicImage;
use symbol::{Symbol, SymbolLocation};

// Decode result
//------------------------------------------------------------------------------

pub struct DecodeResult {
    img: Arc<BinaryImage>,
    symbols: Vec<Symbol>,
}

impl DecodeResult {
    pub fn symbols(&mut self) -> &mut [Symbol] {
        &mut self.symbols
    }
}

// MAIN FUNCTION
//------------------------------------------------------------------------------

pub fn detect_qr(img: &DynamicImage) -> DecodeResult {
    let img = img.to_luma8();
    let mut img = BinaryImage::prepare(&img);

    let finders = locate_finders(&mut img);
    let groups = group_finders(&finders);

    let sym_locs = locate_symbols(&mut img, groups);

    let img = Arc::new(img);
    let symbols = sym_locs.into_iter().map(|sl| Symbol::new(img.clone(), sl)).collect::<_>();

    DecodeResult { img, symbols }
}

// Detect high capacity QR
pub fn detect_hc_qr(img: &DynamicImage) -> DecodeResult {
    let gray_img = img.to_luma8();
    let mut gray_bin = BinaryImage::prepare(&gray_img);

    let finders = locate_finders(&mut gray_bin);
    let groups = group_finders(&finders);

    let sym_locs = locate_symbols(&mut gray_bin, groups);

    let rgb_img = img.to_rgb8();
    let rgb_bin = Arc::new(BinaryImage::prepare(&rgb_img));
    let symbols = sym_locs.into_iter().map(|sl| Symbol::new(rgb_bin.clone(), sl)).collect::<_>();

    DecodeResult { img: rgb_bin, symbols }
}

fn locate_symbols(img: &mut BinaryImage, groups: Vec<FinderGroup>) -> Vec<SymbolLocation> {
    let mut is_grouped = HashSet::new();
    let mut sym_locs = Vec::with_capacity(100);
    for mut g in groups {
        if g.finders.iter().any(|f| is_grouped.contains(f)) {
            continue;
        }

        if let Some(sl) = SymbolLocation::locate(img, &mut g) {
            sym_locs.push(sl);
            is_grouped.extend(g.finders);
        }
    }
    sym_locs
}

#[cfg(test)]
mod reader_tests {

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Version},
        reader::{detect_hc_qr, detect_qr},
        MaskPattern,
    };

    #[test]
    fn test_reader_0() {
        let msg = "Hello, world!";
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);
        let hi_cap = false;

        let qr = QRBuilder::new(msg.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .high_capacity(hi_cap)
            .mask(mask)
            .build()
            .unwrap();
        let img = image::DynamicImage::ImageRgb8(qr.to_image(2));

        let mut res = detect_qr(&img);
        let (_meta, exp_msg) = res.symbols()[0].decode().expect("Failed to read QR");

        assert_eq!(msg, exp_msg, "Incorrect data read from qr image");
    }

    #[test]
    fn test_reader_1() {
        let msg = "Hello, world!🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);
        let hi_cap = true;

        let qr = QRBuilder::new(msg.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .high_capacity(hi_cap)
            .mask(mask)
            .build()
            .unwrap();
        let img = image::DynamicImage::ImageRgb8(qr.to_image(2));

        let mut res = detect_hc_qr(&img);
        let (_meta, exp_msg) = res.symbols()[0].decode().expect("Failed to read QR");

        assert_eq!(msg, exp_msg, "Incorrect data read from qr image");
    }

    #[test]
    #[ignore]
    fn debugger() {
        #[allow(unused_imports)]
        use super::{
            binarize::BinaryImage, finder::locate_finders, locate_symbols, symbol::Symbol,
        };
        #[allow(unused_imports)]
        use crate::reader::{
            detect_qr,
            finder::group_finders,
            utils::geometry::{BresenhamLine, X, Y},
        };
        #[allow(unused_imports)]
        use rayon::prelude::*;
        #[allow(unused_imports)]
        use std::sync::Arc;

        let img_path = std::path::Path::new("assets/example1.png");

        let mut img = image::open(img_path).unwrap().to_rgb8();

        let prep_path = std::path::Path::new("assets/prep.png");
        let mut bin_img = BinaryImage::prepare(&img);
        // bin_img.save(prep_path).unwrap();
        // let mut img = image::open(prep_path).unwrap().to_rgb8();

        let finders = locate_finders(&mut bin_img);
        dbg!(finders.len());
        finders.iter().for_each(|f| f.highlight(&mut img, image::Rgb([255, 0, 0])));

        let groups = group_finders(&finders);
        dbg!(groups.len());
        // groups.iter().for_each(|g| g.highlight(&mut img));

        let sym_locs = locate_symbols(&mut bin_img, groups);
        dbg!(sym_locs.len());
        let bin_img = Arc::new(bin_img);
        let mut symbols: Vec<Symbol> =
            sym_locs.into_iter().map(|sl| Symbol::new(bin_img.clone(), sl)).collect::<_>();
        symbols.iter().for_each(|s| s.highlight(&mut img));

        symbols.iter_mut().enumerate().for_each(|(i, s)| {
            let _ = dbg!(s.decode());
        });

        let out_path = std::path::Path::new("assets/detect.png");
        // img.save(out_path).unwrap();
    }
}
