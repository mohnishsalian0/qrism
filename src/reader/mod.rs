pub mod binarize;
mod finder;
mod symbol;
mod utils;

use finder::{group_finders, locate_finders, FinderGroup};
use image::{
    imageops::{resize, FilterType},
    GrayImage, ImageBuffer, Pixel as ImgPixel,
};

use crate::{ec::Block, utils::QRResult};
use binarize::BinaryImage;
use symbol::{Symbol, SymbolLocation};

pub struct QRReader();

impl QRReader {
    pub fn detect(img: &mut BinaryImage) -> Vec<Symbol> {
        let finders = locate_finders(img);
        let groups = group_finders(img, &finders);
        locate_symbols(img, groups)
    }

    #[cfg(feature = "benchmark")]
    pub fn get_corners(img: GrayImage) -> QRResult<Vec<[f64; 8]>> {
        let img = downscale(img);
        let mut img = BinaryImage::prepare(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let symbols = locate_symbols(&mut img, groups);

        let mut symbol_corners = Vec::with_capacity(20);
        for sym in symbols {
            let sz = sym.ver.width() as f64;

            let bl = sym.raw_map(0.0, sz)?;
            let tl = sym.raw_map(0.0, 0.0)?;
            let tr = sym.raw_map(sz, 0.0)?;
            let br = sym.raw_map(sz, sz)?;
            symbol_corners.push([bl.0, bl.1, tl.0, tl.1, tr.0, tr.1, br.0, br.1])
        }

        Ok(symbol_corners)
    }
}

// Downscales image if bigger than 1000px x 1000px
fn downscale<I>(img: ImageBuffer<I, Vec<I::Subpixel>>) -> ImageBuffer<I, Vec<I::Subpixel>>
where
    I: ImgPixel<Subpixel = u8> + 'static,
{
    let (max_w, max_h) = (1000, 1000);
    let (w, h) = img.dimensions();

    if w <= max_w && h <= max_h {
        return img;
    }

    let scale = f32::min(max_w as f32 / w as f32, max_h as f32 / h as f32);
    let new_w = (w as f32 * scale).round() as u32;
    let new_h = (h as f32 * scale).round() as u32;

    resize(&img, new_w, new_h, FilterType::Triangle)
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

    use std::path::Path;

    use image::{
        imageops::{resize, FilterType},
        open, GrayImage, ImageBuffer, Pixel,
    };

    use super::QRReader;

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Palette, Version},
        reader::{binarize::BinaryImage, deinterleave, detect},
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

        let mut img = BinaryImage::prepare_rgb(&img);
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

        let mut img = BinaryImage::prepare_rgb(&img);
        let mut symbols = QRReader::detect(&mut img);
        let (_meta, msg) = symbols[0].decode().expect("Failed to read QR");

        assert_eq!(msg, exp_msg, "Incorrect data read from qr image");
    }

    #[test]
    #[ignore]
    fn decode_debugger() {
        let (folder_id, qr_id) = (4, 11);

        let qr_path_str = format!("tests/images/qrcode-{folder_id}/{qr_id}.png");
        let qr_path = std::path::Path::new(&qr_path_str);
        let img = image::open(qr_path).unwrap().to_luma8();
        let mut img = BinaryImage::prepare(&img);
        let mut symbols = QRReader::detect(&mut img);
        let (_meta, msg) = symbols[0].decode().expect("Failed to read QR");
        let msg = msg.replace("\r\n", "\n");

        let msg_path_str = format!("tests/images/qrcode-{folder_id}/{qr_id}.txt");
        let msg_path = std::path::Path::new(&msg_path_str);
        let exp_msg = std::fs::read_to_string(msg_path).unwrap();
        let exp_msg = exp_msg.replace("\r\n", "\n");

        assert_eq!(msg, exp_msg);
    }

    fn load_grayscale<P: AsRef<Path>>(path: P) -> Option<GrayImage> {
        match open(&path) {
            Ok(img) => {
                let gray = img.to_luma8();
                let downscaled = downscale(gray);
                Some(downscaled)
            }
            Err(e) => {
                eprintln!("Failed to open {}: {}", path.as_ref().display(), e);
                None
            }
        }
    }

    // Downscales image if bigger than 1000px x 1000px
    fn downscale<I>(img: ImageBuffer<I, Vec<I::Subpixel>>) -> ImageBuffer<I, Vec<I::Subpixel>>
    where
        I: Pixel<Subpixel = u8> + 'static,
    {
        let (max_w, max_h) = (2000, 2000);
        let (w, h) = img.dimensions();

        if w <= max_w && h <= max_h {
            return img;
        }

        let scale = f32::min(max_w as f32 / w as f32, max_h as f32 / h as f32);
        let new_w = (w as f32 * scale).round() as u32;
        let new_h = (h as f32 * scale).round() as u32;

        resize(&img, new_w, new_h, FilterType::Triangle)
    }

    #[test]
    #[ignore]
    fn detect_debugger() {
        #[allow(unused_imports)]
        use super::{binarize::BinaryImage, finder::locate_finders, locate_symbols, QRReader};
        #[allow(unused_imports)]
        use crate::reader::{
            finder::group_finders,
            utils::geometry::{BresenhamLine, Line, X, Y},
        };

        // let inp = "benches/dataset/detection/lots/image002.jpg".to_string();
        let inp = "assets/inp.png".to_string();
        let img = image::open(inp).unwrap().to_luma8();
        // let img = load_grayscale(inp).unwrap();
        // img.save(Path::new("assets/inp.img"));
        let mut bin_img = BinaryImage::prepare(&img);
        let path = std::path::Path::new("assets/inp.png");
        bin_img.save(path).unwrap();

        let mut out_img = image::open(path).unwrap().to_rgb8();

        let finders = locate_finders(&mut bin_img);
        // finders.iter().for_each(|f| f.highlight(&mut out_img));

        let groups = group_finders(&bin_img, &finders);
        groups.iter().for_each(|g| g.highlight(&mut out_img));

        // let symbols = locate_symbols(&mut bin_img, groups);
        //
        // let symbols = detect(&mut bin_img);
        // symbols[0].highlight(&mut out_img);

        let out = std::path::Path::new("assets/out.png");
        out_img.save(out).unwrap();
    }
}
