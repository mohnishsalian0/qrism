mod binarize;
mod finder;
mod symbol;
mod utils;

use finder::{group_finders, locate_finders, FinderGroup};
use image::Pixel as ImgPixel;

use crate::{
    codec::decode,
    debug_println,
    ec::Block,
    metadata::{Metadata, Version},
    utils::{BitStream, QRError, QRResult},
};
use binarize::{Binarize, BinaryImage};
use symbol::{Symbol, SymbolLocation};

pub struct QRReader();

impl QRReader {
    pub fn read<I>(img: &I) -> QRResult<String>
    where
        I: image::GenericImageView + Binarize,
        I::Pixel: ImgPixel<Subpixel = u8>,
    {
        debug_println!("Reading QR...");

        debug_println!("Preparing image...");
        let mut img = BinaryImage::prepare(img);

        debug_println!("Locating finders...");
        let finders = locate_finders(&mut img);

        debug_println!("Grouping finders...");
        let groups = group_finders(&img, &finders);

        debug_println!("Locating symbol...");
        let mut symbol = locate_symbol(img, groups).ok_or(QRError::SymbolNotFound)?;

        debug_println!("Reading format info...");
        let (ecl, mask) = symbol.read_format_info()?;

        debug_println!("Reading version info...");
        if matches!(symbol.ver, Version::Normal(7..=40)) {
            symbol.ver = symbol.read_version_info()?;
        }
        let ver = symbol.ver;

        debug_println!("Reading palette info...");
        let pal = symbol.read_palette_info()?;

        debug_println!("Marking all function patterns...");
        symbol.mark_all_function_patterns();

        debug_println!("Extracting payload...");
        let pld = symbol.extract_payload(&mask)?;

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);

        // Extracting encoded data from payload
        let mut enc = BitStream::new(pld.len() << 3);
        let chan_cap = ver.channel_codewords();

        debug_println!("Separating channels, deinterleaving & rectifying payload...");
        for c in pld.data().chunks_exact(chan_cap) {
            let mut blocks = deinterleave(c, blk_info, ec_len);
            for b in blocks.iter_mut().flatten() {
                let rectified = b.rectify()?;
                enc.extend(rectified);
            }
        }

        debug_println!("Decoding data blocks...");
        let msg = decode(&mut enc, ver, ecl, pal)?;

        debug_println!("\n{}\n", Metadata::new(Some(ver), Some(ecl), Some(mask)));

        String::from_utf8(msg).or(Err(QRError::InvalidUTF8Sequence))
    }
}

fn locate_symbol(mut img: BinaryImage, groups: Vec<FinderGroup>) -> Option<Symbol> {
    let mut sym_loc = None;
    for mut g in groups {
        if let Some(sl) = SymbolLocation::locate(&mut img, &mut g) {
            sym_loc = Some(sl);
            break;
        }
    }
    Some(Symbol::new(img, sym_loc?))
}

fn deinterleave(
    data: &[u8],
    blk_info: (usize, usize, usize, usize),
    ec_len: usize,
) -> [Option<Block>; 256] {
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

    let mut blks: [Option<Block>; 256] = [None; 256];
    dilvd
        .iter()
        .enumerate()
        .for_each(|(i, b)| blks[i] = Some(Block::with_encoded(b, b.len() - ec_len)));
    blks
}

#[cfg(test)]
mod reader_tests {

    use super::QRReader;

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Palette, Version},
        reader::deinterleave,
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
        let data = "Hello, world!";
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);
        let pal = Palette::Mono;

        let qr = QRBuilder::new(data.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .palette(pal)
            .mask(mask)
            .build()
            .unwrap();
        let img = qr.to_image(3);

        let extracted_data = QRReader::read(&img).expect("Couldn't read data");

        assert_eq!(extracted_data, data, "Incorrect data read from qr image");
    }

    #[test]
    fn test_reader_1() {
        let data = "Hello, world!🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);
        let pal = Palette::Mono;

        let qr = QRBuilder::new(data.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .palette(pal)
            .mask(mask)
            .build()
            .unwrap();
        let img = qr.to_image(4);

        let extracted_data = QRReader::read(&img).expect("Couldn't read data");

        assert_eq!(extracted_data, data, "Incorrect data read from qr image");
    }

    #[test]
    #[ignore]
    fn decode_debugger() {
        let (folder_id, qr_id) = (2, 5);

        let qr_path_str = format!("tests/images/qrcode-{folder_id}/{qr_id}.png");
        let qr_path = std::path::Path::new(&qr_path_str);
        let img = image::open(qr_path).unwrap().to_luma8();
        let msg = QRReader::read(&img).expect("Couldn't read data");
        let msg = msg.replace("\r\n", "\n");

        let msg_path_str = format!("tests/images/qrcode-{folder_id}/{qr_id}.txt");
        let msg_path = std::path::Path::new(&msg_path_str);
        let exp_msg = std::fs::read_to_string(msg_path).unwrap();
        let exp_msg = exp_msg.replace("\r\n", "\n");

        assert_eq!(msg, exp_msg);
    }

    #[test]
    #[ignore]
    fn detect_debugger() {
        #[allow(unused_imports)]
        use super::{binarize::BinaryImage, finder::locate_finders, locate_symbol, QRReader};
        #[allow(unused_imports)]
        use crate::reader::{
            finder::group_finders,
            utils::geometry::{BresenhamLine, Line, X, Y},
        };

        let (folder_id, qr_id) = (2, 5);
        let inp = format!("tests/images/qrcode-{folder_id}/{qr_id}.png");
        let img = image::open(inp).unwrap().to_luma8();
        let mut bin_img = BinaryImage::prepare(&img);
        let path = std::path::Path::new("assets/inp.png");
        bin_img.save(path).unwrap();

        let mut out_img = image::open(path).unwrap().to_rgb8();

        let finders = locate_finders(&mut bin_img);
        finders.iter().for_each(|f| f.highlight(&mut out_img));

        let groups = group_finders(&bin_img, &finders);
        groups[0].highlight(&mut out_img);

        let symbol = locate_symbol(bin_img, groups).unwrap();
        symbol.highlight(&mut out_img);

        let out = std::path::Path::new("assets/out.png");
        out_img.save(out).unwrap();
    }
}
