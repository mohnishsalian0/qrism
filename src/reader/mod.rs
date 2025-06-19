pub mod binarize;
mod finder;
pub mod symbol;
mod utils;

use std::collections::HashSet;

use finder::{group_finders, locate_finders, FinderGroup};

use binarize::BinaryImage;
use symbol::{Symbol, SymbolLocation};

pub fn detect(img: &mut BinaryImage) -> Vec<Symbol> {
    let finders = locate_finders(img);
    let groups = group_finders(&finders);
    locate_symbols(img, groups)
}

fn locate_symbols(img: &mut BinaryImage, groups: Vec<FinderGroup>) -> Vec<Symbol> {
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

        let dataset_dir = std::path::Path::new("benches/dataset/detection/monitor/image005.jpg");

        let image_paths: Vec<_> = walkdir::WalkDir::new(dataset_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(is_image_file)
            .map(|e| e.path().to_path_buf())
            .collect();

        image_paths.par_iter().for_each(|inp_path| {
            let parent = get_parent(inp_path);
            let file_name = inp_path.file_name().unwrap().to_str().unwrap();
            let img = image::open(inp_path).unwrap().to_luma8();

            // let mut img = rqrr::PreparedImage::prepare(img);
            // let symbols = img.detect_grids();
            // dbg!(file_name, symbols.len());
            // symbols.iter().enumerate().for_each(|(i, s)| {
            //     if let Ok((meta, msg)) = s.decode() {
            //         println!("[{file_name}] id: {i}, Metadata: {meta:?}, Message: {msg}");
            //     }
            // });

            let inp_str = format!("assets/{parent}/{file_name}");
            let inp_path = std::path::Path::new(&inp_str);
            let mut bin_img = BinaryImage::prepare(&img);
            bin_img.save(inp_path).unwrap();
            let mut out_img = image::open(inp_path).unwrap().to_rgb8();

            let finders = locate_finders(&mut bin_img);
            dbg!(file_name, finders.len());
            finders.iter().for_each(|f| f.highlight(&mut out_img, image::Rgb([255, 0, 0])));

            let groups = group_finders(&finders);
            dbg!(file_name, groups.len());
            groups.iter().for_each(|g| g.highlight(&mut out_img));

            let mut symbols = locate_symbols(&mut bin_img, groups);
            dbg!(file_name, symbols.len());
            symbols.iter().for_each(|s| s.highlight(&mut out_img));

            symbols.iter_mut().enumerate().for_each(|(i, s)| {
                dbg!(s.decode());
            });

            let out_str = format!("assets/{parent}/{file_name}");
            let out_path = std::path::Path::new(&out_str);
            out_img.save(out_path).unwrap();
        })
    }

    fn is_image_file(entry: &walkdir::DirEntry) -> bool {
        entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .map(|e| matches!(e.to_str(), Some("png" | "jpg" | "jpeg" | "bmp")))
                .unwrap_or(false)
    }

    fn get_parent(path: &std::path::Path) -> String {
        path.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap().to_string()
    }
}
