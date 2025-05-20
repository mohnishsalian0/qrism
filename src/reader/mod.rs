mod binarize;
mod finder;
mod symbol;
mod utils;

use finder::{group_finders, locate_finders, FinderGroup};
use image::RgbImage;

use crate::{
    codec::decode,
    ec::Block,
    metadata::{Metadata, Version},
    utils::{BitStream, QRError, QRResult},
};
use binarize::BinaryImage;
use symbol::{Symbol, SymbolLocation};

pub struct QRReader();

impl QRReader {
    // TODO: Rename to read
    pub fn read(img: RgbImage) -> QRResult<String> {
        println!("Reading QR...");

        println!("Preparing image...");
        let mut img = BinaryImage::prepare(img);

        println!("Locating finders...");
        let finders = locate_finders(&mut img);

        println!("Grouping finders...");
        let groups = group_finders(&img, &finders);

        println!("Locating symbol...");
        let mut symbol = locate_symbol(img, groups).ok_or(QRError::SymbolNotFound)?;

        println!("Reading format info...");
        let (ecl, mask) = symbol.read_format_info()?;

        println!("Reading version info...");
        if matches!(symbol.ver, Version::Normal(7..=40)) {
            symbol.ver = symbol.read_version_info()?;
        }
        let ver = symbol.ver;

        println!("Reading palette info...");
        let pal = symbol.read_palette_info();

        // FIXME:
        println!("\n{}\n", Metadata::new(Some(ver), Some(ecl), Some(mask)));

        println!("Marking all function patterns...");
        symbol.mark_all_function_patterns();

        println!("Extracting payload...");
        let pld = symbol.extract_payload(&mask);

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);

        // Extracting encoded data from payload
        let mut enc = BitStream::new(pld.len() << 3);
        let chan_cap = ver.channel_codewords();

        println!("Separating channels, deinterleaving & rectifying payload...");
        pld.data().chunks_exact(chan_cap).for_each(|c| {
            let mut blocks = deinterleave(c, blk_info, ec_len);
            let _ = blocks.iter_mut().filter_map(Option::as_mut).map(Block::rectify);
            blocks.iter().filter_map(Option::as_ref).for_each(|b| enc.extend(b.data()));
        });

        println!("Decoding data blocks...");
        let msg = decode(&mut enc, ver, ecl, pal);

        println!("\n{}\n", Metadata::new(Some(ver), Some(ecl), Some(mask)));

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

    use super::{binarize::BinaryImage, finder::locate_finders, locate_symbol, QRReader};

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Palette, Version},
        reader::{deinterleave, finder::group_finders, utils::Highlight},
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
        let data = "Hello, world!🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);
        let pal = Palette::Poly;

        let qr = QRBuilder::new(data.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .palette(pal)
            .mask(mask)
            .build()
            .unwrap();
        let img = qr.to_image(10);

        let extracted_data = QRReader::read(img).expect("Couldn't read data");

        assert_eq!(extracted_data, data, "Incorrect data read from qr image");
    }

    #[test]
    fn test_reader_1() {
        let path = std::path::Path::new("assets/test1.png");
        let img = image::open(path).unwrap().to_rgb8();
        let msg = QRReader::read(img).unwrap();
        println!("Msg: {msg:?}");
    }

    // #[test]
    // fn reader_debugger() {
    //     let path = std::path::Path::new("assets/test1.png");
    //     let img = image::open(path).unwrap().to_rgb8();
    //     let mut img = BinaryImage::prepare(img);
    //     let finders = locate_finders(&mut img);
    //     let groups = group_finders(&img, &finders);
    //     // let symbol = locate_symbol(img, groups).unwrap();
    //
    //     let mut img = image::open(path).unwrap().to_rgb8();
    //     for f in groups[0].finders.iter() {
    //         println!("Finder {} center {:?}", f.id, f.center);
    //         // f.highlight(&mut img);
    //     }
    //     // symbol.highlight(&mut img);
    //
    //     let out = std::path::Path::new("assets/read.png");
    //     img.save(out).unwrap();
    // }
}
