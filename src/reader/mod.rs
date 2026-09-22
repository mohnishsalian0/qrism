mod alignment;
pub mod binarize;
#[cfg(test)]
mod color;
mod finder;
mod locate;
pub mod symbol;
mod tile;
mod utils;

use std::sync::Arc;

use finder::{group_finders, locate_finders, FinderGroup};

use binarize::BinaryImage;
use image::DynamicImage;
use locate::SymbolLocation;
use symbol::Symbol;

use crate::reader::finder::Finder;

// Decode result
//------------------------------------------------------------------------------

pub struct DecodeResult {
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

    let sym_locs = locate_symbols(&mut img, finders, groups);

    let img = Arc::new(img);
    let symbols = sym_locs.into_iter().map(|sl| Symbol::new(img.clone(), sl)).collect::<_>();

    DecodeResult { symbols }
}

// Detect high capacity QR
pub fn detect_hc_qr(img: &DynamicImage) -> DecodeResult {
    let rgb_img = img.to_rgb8();

    // Locating runs on a 1-bit view of the symbol, so the colour image has to be projected to
    // gray first. `min_channel` is the projection that keeps a colour symbol's function
    // patterns intact; see its comment for why luma does not. Symbols whose finders are drawn
    // in colours that only luma separates still fall back to it.
    let mut sym_locs = locate_in_gray(&binarize::min_channel(&rgb_img));
    if sym_locs.is_empty() {
        sym_locs = locate_in_gray(&img.to_luma8());
    }

    let rgb_bin = Arc::new(BinaryImage::prepare_discard(&rgb_img));
    let symbols = sym_locs.into_iter().map(|sl| Symbol::new(rgb_bin.clone(), sl)).collect::<_>();

    DecodeResult { symbols }
}

fn locate_in_gray(gray: &image::GrayImage) -> Vec<SymbolLocation> {
    let mut bin = BinaryImage::prepare(gray);
    let finders = locate_finders(&mut bin);
    let groups = group_finders(&finders);
    locate_symbols(&mut bin, finders, groups)
}

fn locate_symbols(
    img: &mut BinaryImage,
    finders: Vec<Finder>,
    groups: Vec<FinderGroup>,
) -> Vec<SymbolLocation> {
    let mut is_grouped = vec![false; finders.len()];

    let mut sym_locs = Vec::with_capacity(100);
    for mut g in groups {
        if g.ids.iter().any(|&fid| is_grouped[fid]) {
            continue;
        }

        if let Some(sl) = SymbolLocation::locate(img, &finders, &mut g) {
            sym_locs.push(sl);
            g.ids.iter().for_each(|&fid| is_grouped[fid] = true);
        }
    }
    sym_locs
}

#[cfg(test)]
mod reader_tests {

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Version},
        reader::{detect_hc_qr, detect_qr, utils::geometry::Point},
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

    // Printed colour symbols with colour-drawn function patterns. Luma sinks the yellow
    // alignment patterns into the quiet zone and localization fails outright; the min-channel
    // projection recovers all of them. Ignored: reads the benchmark dataset.
    #[test]
    #[ignore]
    fn test_locates_printed_color_symbols() {
        use crate::Version;

        let dir = std::path::Path::new("benches/dataset/high_capacity/test");
        let mut paths: Vec<_> = std::fs::read_dir(dir)
            .expect("colour test dataset is missing")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
                    Some("jpg" | "jpeg" | "png")
                )
            })
            .collect();
        paths.sort();
        assert!(!paths.is_empty(), "colour test dataset is empty");

        for path in &paths {
            let img = image::open(path).unwrap();
            let mut res = detect_hc_qr(&img);
            let vers: Vec<Version> = res.symbols().iter().map(|s| s.version()).collect();
            assert_eq!(
                vers,
                vec![Version::Normal(25)],
                "expected one v25 symbol in {}",
                path.display()
            );
        }
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

        let img_path = std::path::Path::new("./assets/test.jpg");

        // let img = image::open(img_path).unwrap().to_luma8();
        let img = image::open(img_path).unwrap().to_rgb8();
        let img = crate::binarize::min_channel(&img);

        let prep_path = std::path::Path::new("assets/prep.png");
        let mut bin_img = BinaryImage::prepare(&img);
        bin_img.save(prep_path).unwrap();
        let mut img = image::open(prep_path).unwrap().to_rgb8();

        let finders = locate_finders(&mut bin_img);
        dbg!(finders.len());
        finders.iter().for_each(|f| Point::from(&f.c).highlight(&mut img, image::Rgb([255, 0, 0])));

        let groups = group_finders(&finders);
        dbg!(groups.len());
        groups.iter().for_each(|g| g.highlight(&mut img, &finders));

        let sym_locs = locate_symbols(&mut bin_img, finders, groups);
        dbg!(sym_locs.len());
        sym_locs.iter().for_each(|sl| sl.highlight(&mut img));

        let bin_img = Arc::new(bin_img);
        let mut symbols: Vec<Symbol> =
            sym_locs.into_iter().map(|sl| Symbol::new(bin_img.clone(), sl)).collect::<_>();

        symbols.iter_mut().for_each(|s| {
            let _ = dbg!(s.decode());
        });

        let out_path = std::path::Path::new("assets/detect.png");
        img.save(out_path).unwrap();
    }
}
