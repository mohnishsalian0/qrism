pub mod binarize;
mod finder;
pub mod symbol;
mod utils;

use std::collections::HashSet;

use finder::{group_finders, locate_finders, FinderGroup};

use binarize::BinaryImage;
use symbol::{Symbol, SymbolLocation};

pub fn detect(img: &'_ mut BinaryImage) -> Vec<Symbol<'_>> {
    let finders = locate_finders(img);
    let groups = group_finders(&finders);
    locate_symbols(img, groups)
}

fn locate_symbols(img: &'_ mut BinaryImage, groups: Vec<FinderGroup>) -> Vec<Symbol<'_>> {
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
    sym_locs.into_iter().map(|sl| Symbol::new(img, sl)).collect::<_>()
}

#[cfg(test)]
mod reader_tests {

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Palette, Version},
        reader::{binarize::BinaryImage, detect},
        MaskPattern,
    };

    #[test]
    fn test_reader_0() {
        let msg = "Hello, world!";
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);
        let pal = Palette::Mono;

        let qr = QRBuilder::new(msg.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .palette(pal)
            .mask(mask)
            .build()
            .unwrap();
        let img = qr.to_image(2);

        let mut img = BinaryImage::prepare(&img);
        let mut symbols = detect(&mut img);
        let (_meta, exp_msg) = symbols[0].decode().expect("Failed to read QR");

        assert_eq!(msg, exp_msg, "Incorrect data read from qr image");
    }

    #[test]
    fn test_reader_1() {
        let exp_msg = "Hello, world!🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);
        let pal = Palette::Poly;

        let qr = QRBuilder::new(exp_msg.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .palette(pal)
            .mask(mask)
            .build()
            .unwrap();
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(&img);
        let mut symbols = detect(&mut img);
        let (_meta, msg) = symbols[0].decode().expect("Failed to read QR");

        assert_eq!(msg, exp_msg, "Incorrect data read from qr image");
    }

    #[test]
    #[ignore]
    fn debugger() {
        #[allow(unused_imports)]
        use super::{binarize::BinaryImage, finder::locate_finders, locate_symbols};
        #[allow(unused_imports)]
        use crate::reader::{
            detect,
            finder::group_finders,
            utils::geometry::{BresenhamLine, X, Y},
        };
        #[allow(unused_imports)]
        use rayon::prelude::*;

        let img_path = std::path::Path::new("assets/example6.png");

        let img = image::open(img_path).unwrap().to_rgb8();

        let prep_path = std::path::Path::new("assets/prep.png");
        let mut bin_img = BinaryImage::prepare(&img);
        bin_img.save(prep_path).unwrap();
        let mut out_img = image::open(prep_path).unwrap().to_rgb8();

        let finders = locate_finders(&mut bin_img);
        dbg!(finders.len());
        finders.iter().for_each(|f| f.highlight(&mut out_img, image::Rgb([255, 0, 0])));

        let groups = group_finders(&finders);
        dbg!(groups.len());
        // groups.iter().for_each(|g| g.highlight(&mut out_img));

        let mut symbols = locate_symbols(&mut bin_img, groups);
        dbg!(symbols.len());
        symbols.iter().for_each(|s| s.highlight(&mut out_img));

        symbols.iter_mut().enumerate().for_each(|(i, s)| {
            let _ = dbg!(s.decode());
        });

        let out_path = std::path::Path::new("assets/detect.png");
        out_img.save(out_path).unwrap();
    }
}
