use std::sync::Arc;

use super::{binarize::BinaryImage, locate::SymbolLocation, utils::geometry::Point};
use crate::{
    codec::decode as codec_decode,
    ec::{rectify_info, Block},
    metadata::{
        parse_format_info_qr, Color, Metadata, FORMAT_ERROR_CAPACITY, FORMAT_INFOS_QR,
        FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK,
    },
    reader::fitness::Tile,
    utils::{BitArray, BitStream, EncRegionIter, QRError, QRResult},
    ECLevel, MaskPattern, Version,
};

// Symbol
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Symbol {
    img: Arc<BinaryImage>,
    pub ver: Version,
    tiles: [[Option<Tile>; 6]; 6],
    bands: [u8; MAX_WIDTH], // module -> tile, both axes
    _anchors: [[Option<Point>; 7]; 7],
}

impl Symbol {
    pub fn new(img: Arc<BinaryImage>, sym_loc: SymbolLocation) -> Self {
        let SymbolLocation { ver, tiles, _anchors } = sym_loc;
        let bands = Self::band_table(ver);
        Self { img, ver, tiles, bands, _anchors }
    }

    // Maps each module coordinate to the index of the tile owning it along that axis. The symbol is
    // square and both axes use the same alignment coordinates, so one table serves x and y alike.
    fn band_table(ver: Version) -> [u8; MAX_WIDTH] {
        let w = ver.width();
        let aps = ver.alignment_pattern();
        let n = aps.len().max(2);
        let interior = aps.get(1..n - 1).unwrap_or(&[]);

        let mut table = [u8::MAX; MAX_WIDTH];
        let mut band = 0usize;

        for (m, slot) in table.iter_mut().enumerate().take(w) {
            if interior.get(band).is_some_and(|&e| m as i32 >= e) {
                band += 1;
            }
            *slot = band as u8;
        }

        table
    }

    pub fn decode(&mut self) -> QRResult<(Metadata, String)> {
        let (ecl, mask) = self.read_format_info()?;
        let ver = self.ver;
        let hi_cap = self.read_capacity_info()?;

        let pld = self.extract_payload(&mask)?;

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

    fn tile_at(&self, x: usize, y: usize) -> QRResult<&Tile> {
        debug_assert!(
            x < self.ver.width() && y < self.ver.width(),
            "Module coord x: {x} or y: {y} is out of bound"
        );

        let tx = self.bands[x] as usize;
        let ty = self.bands[y] as usize;
        self.tiles[ty][tx].as_ref().ok_or(QRError::TileNotFound)
    }

    pub fn get(&self, x: i32, y: i32) -> QRResult<Color> {
        let (xp, yp) = self.wrap_coord(x, y);
        let tile = self.tile_at(xp as usize, yp as usize)?;
        let pt = tile.map(xp as f64 + 0.5, yp as f64 + 0.5)?;
        self.img.get_at_point(&pt).ok_or(QRError::PixelOutOfBounds)
    }

    fn wrap_coord(&self, x: i32, y: i32) -> (i32, i32) {
        let w = self.ver.width() as i32;
        debug_assert!(-w <= x && x < w, "x shouldn't be greater than or equal to w");
        debug_assert!(-w <= y && y < w, "y shouldn't be greater than or equal to w");

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + w } else { y };
        (x, y)
    }
}

// Read format, version & capacity info
//------------------------------------------------------------------------------

impl Symbol {
    pub fn read_format_info(&self) -> QRResult<(ECLevel, MaskPattern)> {
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

    pub fn read_capacity_info(&self) -> QRResult<bool> {
        if let Ok(color) = self.get(8, -8) {
            if color == Color::Black {
                return Ok(false); // Standard capacity
            } else {
                return Ok(true); // High capacity
            }
        }

        Err(QRError::InvalidCapacityInfo)
    }

    pub fn get_number(&self, coords: &[(i32, i32)]) -> Option<u32> {
        let mut num = 0;
        for &(x, y) in coords {
            let color = self.get(x, y).ok()?;
            let bit = (color != Color::White) as u32;
            num = (num << 1) | bit;
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
    pub fn extract_payload(&self, mask: &MaskPattern) -> QRResult<BitArray> {
        let ver = self.ver;
        let mask_fn = mask.mask_functions();
        let chan_bits = ver.channel_codewords() << 3;
        let offsets = [2 * chan_bits, chan_bits, 0]; // B, G, R offsets
        let mut payload = BitArray::new(chan_bits * 3);
        let mut rgn_iter = EncRegionIter::new(ver);

        for (i, (x, y)) in rgn_iter.by_ref().take(chan_bits).enumerate() {
            let color = self.get(x, y)?;
            let rgb = color as u8;
            for (j, off) in offsets.iter().enumerate() {
                let mut bit = ((rgb >> j) & 1) == 1;
                if !mask_fn(x, y) {
                    bit = !bit;
                }
                payload.put(i + off, bit);
            }
        }

        debug_assert_eq!(rgn_iter.count(), self.ver.remainder_bits(), "Remainder bits don't match");

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

const MAX_WIDTH: usize = 177;
