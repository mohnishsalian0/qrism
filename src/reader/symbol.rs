use std::sync::Arc;

use image::RgbImage;

use super::{binarize::BinaryImage, locate::SymbolLocation};
use crate::{
    codec::decode as codec_decode,
    ec::{rectify_info, Block},
    metadata::{
        parse_format_info_qr, Color, Metadata, FORMAT_ERROR_CAPACITY, FORMAT_INFOS_QR,
        FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK,
    },
    reader::mahalanobis::Mahalanobis,
    utils::{BitArray, BitStream, EncRegionIter, QRError, QRResult},
    ECLevel, MaskPattern,
};

// Symbol
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Symbol {
    bin: Arc<BinaryImage>,
    rgb: Option<Arc<RgbImage>>,
    loc: SymbolLocation,
    clf: Option<Mahalanobis>,
}

impl Symbol {
    pub(crate) fn new(
        bin: Arc<BinaryImage>,
        rgb: Option<Arc<RgbImage>>,
        loc: SymbolLocation,
    ) -> Self {
        Self { bin, rgb, loc, clf: None }
    }

    pub fn decode(&mut self) -> QRResult<(Metadata, String)> {
        let (ecl, mask) = self.read_format_info()?;
        let ver = self.loc.ver;
        let hi_cap = self.read_capacity_info()?;

        let pld = if self.rgb.is_some() {
            let sampler = |x: i32, y: i32| self.get_rgb(x, y);
            let clf = Mahalanobis::fit(ver, sampler)?;
            self.clf = Some(clf);
            self.extract_payload_hc(&mask)?
        } else {
            self.extract_payload(&mask)?
        };

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let mut enc = BitStream::new(pld.len() << 3);
        let chan_cap = ver.channel_codewords();

        // Chunking channel data, deinterleaving & rectifying payload
        for c in pld.data().chunks_exact(chan_cap) {
            let mut blocks = deinterleave(c, blk_info, ec_len);
            for b in blocks.iter_mut() {
                let rectified = b.rectify()?;
                enc.extend(rectified);
            }
        }

        let msg = codec_decode(&mut enc, ver, ecl, hi_cap)?;
        let meta = Metadata::new(Some(ver), Some(ecl), Some(mask));

        Ok((meta, msg))
    }

    /// Module colour at (x, y), with blur from its 4 neighbouring modules removed.
    fn get_rgb(&self, x: i32, y: i32) -> QRResult<[u8; 3]> {
        let rgb_img = self.rgb.as_ref().ok_or(QRError::RgbImageMissing)?;
        let (x, y) = self.wrap_coord(x, y);

        let raw = |x: i32, y: i32| {
            self.get_pt(x, y).and_then(|pt| {
                rgb_img.get_pixel_checked(pt.0, pt.1).ok_or(QRError::PixelOutOfBounds)
            })
        };

        let centre = raw(x, y)?.0;

        let mut n = 0;
        let mut acc = centre.map(f64::from);
        let w = self.loc.ver.width() as i32;
        for (dx, dy) in [(0, -1), (1, 0), (0, 1), (-1, 0)] {
            let (nx, ny) = (x + dx, y + dy);
            if nx < 0 || w <= nx || ny < 0 || w <= ny {
                continue;
            }
            let Ok(ng) = raw(nx, ny) else { continue };
            for i in 0..3 {
                acc[i] -= BLUR_BETA * ng[i] as f64;
            }
            n += 1;
        }

        let denom = 1.0 - BLUR_BETA * n as f64;

        Ok(acc.map(|v| (v / denom).round() as u8))
    }

    /// Whether the pixel in binary image is dark
    pub(crate) fn is_dark(&self, x: i32, y: i32) -> QRResult<bool> {
        let pt = self.get_pt(x, y)?;
        self.bin.get_bit(pt.0, pt.1).map(|b| !b).ok_or(QRError::PixelOutOfBounds)
    }

    pub(crate) fn get_color(&self, x: i32, y: i32) -> QRResult<Color> {
        let rgb = self.get_rgb(x, y)?;

        Ok(self.clf.as_ref().ok_or(QRError::ClassifierMissing)?.classify(&rgb))
    }

    fn get_pt(&self, x: i32, y: i32) -> QRResult<(u32, u32)> {
        let (xp, yp) = self.wrap_coord(x, y);
        let tile = self.loc.tile_at(xp as usize, yp as usize)?;
        tile.map(xp as f64 + 0.5, yp as f64 + 0.5).map_err(|_| QRError::PixelOutOfBounds)
    }

    fn wrap_coord(&self, x: i32, y: i32) -> (i32, i32) {
        let w = self.loc.ver.width() as i32;
        debug_assert!(-w <= x && x < w, "x shouldn't be greater than or equal to w");
        debug_assert!(-w <= y && y < w, "y shouldn't be greater than or equal to w");

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + w } else { y };
        (x, y)
    }

    #[inline]
    pub fn outline(&self) -> QRResult<[(f64, f64); 4]> {
        self.loc.outline()
    }
}

// Read format, version & capacity info
//------------------------------------------------------------------------------

impl Symbol {
    pub(crate) fn read_format_info(&self) -> QRResult<(ECLevel, MaskPattern)> {
        // Parse main format area
        if let Some(main) = self.get_number(&FORMAT_INFO_COORDS_QR_MAIN) {
            if let Ok((format, _)) = rectify_info(main, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY) {
                let format = format ^ FORMAT_MASK;
                let (ecl, mask) = parse_format_info_qr(format);
                return Ok((ecl, mask));
            }
        }

        // Parse side format area
        if let Some(side) = self.get_number(&FORMAT_INFO_COORDS_QR_SIDE) {
            if let Ok((format, _)) = rectify_info(side, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY) {
                let format = format ^ FORMAT_MASK;
                let (ecl, mask) = parse_format_info_qr(format);
                return Ok((ecl, mask));
            }
        }

        Err(QRError::InvalidFormatInfo)
    }

    pub(crate) fn read_capacity_info(&self) -> QRResult<bool> {
        self.is_dark(8, -8).map(|dark| !dark).map_err(|_| QRError::InvalidCapacityInfo)
    }

    pub(crate) fn get_number(&self, coords: &[(i32, i32)]) -> Option<u32> {
        let mut num = 0;
        for &(x, y) in coords {
            let dark = self.is_dark(x, y).ok()?;
            num = (num << 1) | (dark as u32);
        }
        Some(num)
    }
}

#[cfg(test)]
mod symbol_infos_tests {

    use crate::{
        metadata::Color, reader::detect_qr, ECLevel, MaskPattern, Module, QRBuilder, Version,
    };

    #[test]
    fn test_read_format_info_clean() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let fmt_info = res.symbols()[0].read_format_info().expect("Failed to read format info");
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    fn test_read_format_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(1, 8, Module::Format(Color::White));
        qr.set(2, 8, Module::Format(Color::White));
        qr.set(4, 8, Module::Format(Color::Black));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let fmt_info = res.symbols()[0].read_format_info().expect("Failed to read format info");
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    fn test_read_format_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(1, 8, Module::Format(Color::White));
        qr.set(2, 8, Module::Format(Color::White));
        qr.set(3, 8, Module::Format(Color::Black));
        qr.set(4, 8, Module::Format(Color::Black));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let fmt_info = res.symbols()[0].read_format_info().expect("Failed to read format info");
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    #[should_panic]
    fn test_read_format_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(1, 8, Module::Format(Color::White));
        qr.set(2, 8, Module::Format(Color::White));
        qr.set(3, 8, Module::Format(Color::Black));
        qr.set(4, 8, Module::Format(Color::Black));
        qr.set(8, -2, Module::Format(Color::White));
        qr.set(8, -3, Module::Format(Color::White));
        qr.set(8, -4, Module::Format(Color::Black));
        qr.set(8, -5, Module::Format(Color::Black));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let _ = res.symbols()[0].read_format_info().expect("Failed to read format info");
    }
}

// Extracts encoded data codewords and error correction codewords
//------------------------------------------------------------------------------

impl Symbol {
    pub(crate) fn extract_payload(&self, mask: &MaskPattern) -> QRResult<BitArray> {
        let ver = self.loc.ver;
        let mask_fn = mask.mask_functions();
        let chan_bits = ver.channel_codewords() << 3;
        let mut payload = BitArray::new(chan_bits);
        let mut rgn_iter = EncRegionIter::new(ver);

        for (i, (x, y)) in rgn_iter.by_ref().take(chan_bits).enumerate() {
            let mut bit = !self.is_dark(x, y)?;
            if !mask_fn(x, y) {
                bit = !bit;
            }
            payload.put(i, bit);
        }

        debug_assert_eq!(
            rgn_iter.count(),
            self.loc.ver.remainder_bits(),
            "Remainder bits don't match"
        );

        Ok(payload)
    }

    pub(crate) fn extract_payload_hc(&self, mask: &MaskPattern) -> QRResult<BitArray> {
        debug_assert!(self.rgb.is_some());
        debug_assert!(self.clf.is_some());

        let ver = self.loc.ver;
        let mask_fn = mask.mask_functions();
        let chan_bits = ver.channel_codewords() << 3;
        let offsets = [2 * chan_bits, chan_bits, 0]; // B, G, R offsets
        let mut payload = BitArray::new(chan_bits * 3);
        let mut rgn_iter = EncRegionIter::new(ver);

        for (i, (x, y)) in rgn_iter.by_ref().take(chan_bits).enumerate() {
            let color = self.get_color(x, y)?;
            let rgb = color as u8;
            for (j, off) in offsets.iter().enumerate() {
                let mut bit = ((rgb >> j) & 1) == 1;
                if !mask_fn(x, y) {
                    bit = !bit;
                }
                payload.put(i + off, bit);
            }
        }

        debug_assert_eq!(
            rgn_iter.count(),
            self.loc.ver.remainder_bits(),
            "Remainder bits don't match"
        );

        Ok(payload)
    }
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

    let mut blks: Vec<Block> = Vec::with_capacity(256);
    dilvd.iter().for_each(|b| blks.push(Block::with_encoded(b, b.len() - ec_len)));
    blks
}

#[cfg(test)]
mod reader_tests {

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Version},
        reader::symbol::deinterleave,
        utils::BitStream,
    };

    #[test]
    fn test_deinterleave() {
        // Data length has to match version capacity
        let data = "Hello, world!!!🌍".as_bytes();
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;

        let exp_blks = QRBuilder::blockify(data, ver, ecl);

        let mut bs = BitStream::new(ver.total_codewords(false) << 3);
        QRBuilder::interleave_into(&exp_blks, &mut bs);

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let blks = deinterleave(bs.data(), blk_info, ec_len);
        assert_eq!(blks, exp_blks);
    }
}

// Global constants
//------------------------------------------------------------------------------

const BLUR_BETA: f64 = 0.06;
