mod deqr;
mod finder;
mod prepare;
mod symbol;
mod utils;

use finder::{group_finders, locate_finders};
use image::{Rgb, RgbImage};
use std::cmp;

use crate::{
    builder::QR,
    codec::decode,
    ec::Block,
    metadata::Version,
    utils::{BitStream, QRError, QRResult},
};
// FIXME: Remove DeQR
use deqr::DeQR;
use prepare::PreparedImage;
use symbol::{Symbol, SymbolLocation};

pub trait QRReadable {
    fn to_deqr(&self, ver: Version) -> DeQR;
}

impl QRReadable for RgbImage {
    fn to_deqr(&self, ver: Version) -> DeQR {
        DeQR::from_clr_img(self, ver)
    }
}

impl QRReadable for String {
    fn to_deqr(&self, ver: Version) -> DeQR {
        DeQR::from_str(self, ver)
    }
}

impl QRReadable for QR {
    fn to_deqr(&self, _ver: Version) -> DeQR {
        DeQR::from(self)
    }
}

pub struct QRReader();

impl QRReader {
    // TODO: Remove version
    pub fn read<T: QRReadable>(qr: &T, ver: Version) -> QRResult<String> {
        println!("Reading QR...");
        let mut deqr = qr.to_deqr(ver);

        println!("Reading format info...");
        let (ecl, mask) = deqr.read_format_info()?;

        println!("Reading version info...");
        let ver = match ver {
            Version::Normal(7..=40) => deqr.read_version_info()?,
            _ => ver,
        };

        println!("Marking all function patterns...");
        deqr.mark_all_function_patterns();

        println!("Unmasking payload...");
        deqr.unmask(mask);

        println!("Extracting payload...");
        let pld = deqr.extract_payload(ver);

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);

        // Extracting encoded data from payload
        let mut enc = BitStream::new(pld.len() << 3);
        let chan_cap = ver.channel_codewords();

        println!("Separating channels, deinterleaving & rectifying payload...");
        pld.data().chunks_exact(chan_cap).for_each(|c| {
            let mut blocks = Self::deinterleave(c, blk_info, ec_len);
            let _ = blocks.iter_mut().filter_map(Option::as_mut).map(Block::rectify);
            blocks.iter().filter_map(Option::as_ref).for_each(|b| enc.extend(b.data()));
        });

        // If the QR is B&W, discard duplicate data from 2 channels
        Self::dedupe(&mut enc);

        println!("Decoding data blocks...");
        let msg = decode(&mut enc, ver);

        println!("\n{}\n", deqr.metadata());

        String::from_utf8(msg).or(Err(QRError::InvalidUTF8Sequence))
    }

    /// Performs adaptive binarization on an RGB image using a sliding window
    /// and per-channel average filtering.
    fn prepare(img: &mut RgbImage) {
        let (w, h) = img.dimensions();
        let win_sz = cmp::max(w / 8, 1);
        let den = 200 * win_sz;
        let mut u_avg = [0, 0, 0];
        let mut v_avg = [0, 0, 0];
        let mut row_avg = vec![[0, 0, 0]; w as usize];

        for y in 0..h {
            for x in 0..w {
                let (u, v) = if y & 1 == 0 { (x, w - 1 - x) } else { (w - 1 - x, x) };
                let (u_usize, v_usize) = (u as usize, v as usize);
                let (pu, pv) = (img.get_pixel(u, y), img.get_pixel(v, y));

                for i in 0..3 {
                    u_avg[i] = u_avg[i] * (win_sz - 1) / win_sz + pu[i] as u32;
                    v_avg[i] = v_avg[i] * (win_sz - 1) / win_sz + pv[i] as u32;
                    row_avg[u_usize][i] += u_avg[i];
                    row_avg[v_usize][i] += v_avg[i];
                }
            }

            for x in 0..w {
                let mut out = [0, 0, 0];
                let px = img.get_pixel(x, y);
                for (i, p) in out.iter_mut().enumerate() {
                    let thresh = row_avg[x as usize][i] * (100 - 5) / den;
                    if px[i] as u32 >= thresh {
                        *p = 255;
                    }
                }
                img.put_pixel(x, y, Rgb(out));
            }

            row_avg.fill([0, 0, 0]);
        }
    }

    fn locate_symbol(img: &mut PreparedImage) -> Option<Symbol> {
        let finders = locate_finders(img);
        let groups = group_finders(&finders);
        let mut sym_loc = None;
        for mut g in groups {
            if let Some(sl) = SymbolLocation::locate(img, &mut g) {
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

    fn dedupe(enc: &mut BitStream) {
        let data = enc.data();
        let split = (enc.len() >> 3) / 3;
        if data[..split] == data[split..split * 2] && data[..split] == data[split * 2..] {
            enc.truncate(split << 3);
        }
    }
}

#[cfg(test)]
mod reader_tests {

    use std::path::Path;

    use super::QRReader;
    use crate::builder::QRBuilder;
    use crate::metadata::{ECLevel, Palette, Version};
    use crate::utils::BitStream;

    #[test]
    fn test_prepare() {
        let path = Path::new("assets/test1.png");
        let mut img = image::open(path).unwrap().to_rgb8();
        QRReader::prepare(&mut img);
        let out_path = Path::new("assets/test_out.png");
        img.save(out_path).expect("Failed to save image");
    }

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
        let blks = QRReader::deinterleave(bs.data(), blk_info, ec_len);
        assert_eq!(blks, exp_blks);
    }
}
