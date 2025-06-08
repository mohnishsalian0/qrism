pub mod binarize;
mod finder;
mod symbol;
mod utils;

use std::time::Instant;

use finder::{group_finders, locate_finders, FinderGroup};
use image::GrayImage;

use crate::ec::Block;
use binarize::BinaryImage;
use symbol::{Symbol, SymbolLocation};

pub struct QRReader();

impl QRReader {
    pub fn detect(img: &mut BinaryImage) -> Vec<Symbol> {
        let start = Instant::now();
        let finders = locate_finders(img);
        println!("Finder: {}", start.elapsed().as_millis());
        let start = Instant::now();
        let groups = group_finders(img, &finders);
        println!("Grouping: {}", start.elapsed().as_millis());
        let start = Instant::now();
        let symbols = locate_symbols(img, groups);
        println!("Symbol: {}", start.elapsed().as_millis());
        symbols
    }

    #[cfg(feature = "benchmark")]
    pub fn get_corners(img: GrayImage) -> Vec<[f64; 8]> {
        //FIXME:
        let start = Instant::now();
        let mut img = BinaryImage::binarize(&img);
        println!("Binry: {}", start.elapsed().as_millis());

        let start = Instant::now();
        let finders = locate_finders(&mut img);
        println!("Findr: {}", start.elapsed().as_millis());

        let start = Instant::now();
        let groups = group_finders(&img, &finders);
        println!("Group: {}", start.elapsed().as_millis());

        let start = Instant::now();
        let symbols = locate_symbols(&mut img, groups);
        println!("Symbl: {}", start.elapsed().as_millis());

        let mut symbol_corners = Vec::with_capacity(20);
        for sym in symbols {
            let sz = sym.ver.width() as f64;

            let bl = match sym.raw_map(0.0, sz) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let tl = match sym.raw_map(0.0, 0.0) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let tr = match sym.raw_map(sz, 0.0) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let br = match sym.raw_map(sz, sz) {
                Ok(p) => p,
                Err(_) => continue,
            };

            symbol_corners.push([bl.0, bl.1, tl.0, tl.1, tr.0, tr.1, br.0, br.1])
        }

        symbol_corners
    }
}

fn detect(img: &mut BinaryImage) -> Vec<Symbol> {
    let finders = locate_finders(img);
    let groups = group_finders(img, &finders);
    locate_symbols(img, groups)
}

fn locate_symbols(img: &mut BinaryImage, groups: Vec<FinderGroup>) -> Vec<Symbol> {
    let mut sym_locs = Vec::with_capacity(20);
    for mut g in groups {
        if let Some(sl) = SymbolLocation::locate(img, &mut g) {
            sym_locs.push(sl);
        }
    }
    sym_locs.into_iter().map(|sl| Symbol::new(img, sl)).collect::<_>()
}

fn deinterleave(data: &[u8], blk_info: (usize, usize, usize, usize), ec_len: usize) -> Vec<Block> {
    // b1s = block1_size, b1c = block1_count
    let (b1s, b1c, b2s, b2c) = blk_info;

    let total_blks = b1c + b2c;
    let spl = b1s * total_blks;
    let data_sz = b1s * b1c + b2s * b2c;

    let mut dilvd = vec![Vec::with_capacity(b2s); total_blks];

    // Deinterleaving data
    data[..spl]
        .chunks(total_blks)
        .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| dilvd[i].push(*v)));
    if b2c > 0 {
        data[spl..data_sz]
            .chunks(b2c)
            .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| dilvd[b1c + i].push(*v)));
    }

    // Deinterleaving ecc
    data[data_sz..]
        .chunks(total_blks)
        .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| dilvd[i].push(*v)));

    let mut blks = Vec::with_capacity(256);
    dilvd.iter().enumerate().for_each(|(i, b)| blks.push(Block::with_encoded(b, b.len() - ec_len)));
    blks
}

#[cfg(test)]
mod reader_tests {

    use super::QRReader;

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Palette, Version},
        reader::{binarize::BinaryImage, deinterleave},
        utils::BitStream,
        MaskPattern,
    };

    #[test]
    fn test_deinterleave() {
        // Data length has to match version capacity
        let data = "Hello, world!!!🌍".as_bytes();
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;

        let exp_blks = QRBuilder::blockify(data, ver, ecl);

        let mut bs = BitStream::new(ver.total_codewords(Palette::Mono) << 3);
        QRBuilder::interleave_into(&exp_blks, &mut bs);

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let blks = deinterleave(bs.data(), blk_info, ec_len);
        assert_eq!(blks, exp_blks);
    }

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

        let mut img = BinaryImage::binarize(&img);
        let mut symbols = QRReader::detect(&mut img);
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

        let mut img = BinaryImage::binarize(&img);
        let mut symbols = QRReader::detect(&mut img);
        let (_meta, msg) = symbols[0].decode().expect("Failed to read QR");

        assert_eq!(msg, exp_msg, "Incorrect data read from qr image");
    }

    // #[test]
    // #[ignore]
    // fn decode_debugger() {
    //     let inp_path = std::path::Path::new("benches/dataset/detection/rotations/image001.jpg");
    //     let img = image::open(inp_path).unwrap().to_luma8();
    //     let mut img = BinaryImage::binarize(&img);
    //     let mut symbols = QRReader::detect(&mut img);
    //     let (_meta, msg) = symbols[0].decode().expect("Failed to read QR");
    // }

    #[test]
    #[ignore]
    fn detect_debugger() {
        #[allow(unused_imports)]
        use super::{binarize::BinaryImage, finder::locate_finders, locate_symbols, QRReader};
        #[allow(unused_imports)]
        use crate::reader::{
            detect,
            finder::group_finders,
            utils::geometry::{BresenhamLine, Line, X, Y},
        };

        let inp_path = std::path::Path::new("benches/dataset/detection/bright_spots/image001.jpg");
        // let inp_path = std::path::Path::new("assets/cleaned.png");
        let img = image::open(inp_path).unwrap().to_luma8();
        let mut bin_img = BinaryImage::binarize(&img);

        let out_path = std::path::Path::new("assets/inp.png");
        bin_img.save(out_path).unwrap();
        let mut out_img = image::open(out_path).unwrap().to_rgb8();

        // let finders = locate_finders(&mut bin_img);
        // finders.iter().for_each(|f| f.highlight(&mut out_img));

        // let groups = group_finders(&bin_img, &finders);
        // groups.iter().for_each(|g| g.highlight(&mut out_img));

        let symbols = detect(&mut bin_img);
        symbols.iter().for_each(|s| s.highlight(&mut out_img));

        let out = std::path::Path::new("assets/out.png");
        out_img.save(out).unwrap();
    }
}
