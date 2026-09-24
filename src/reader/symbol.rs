use std::sync::Arc;

use super::{binarize::BinaryImage, locate::SymbolLocation};
use crate::{
    codec::decode as codec_decode,
    ec::{rectify_info, Block},
    metadata::{
        parse_format_info_qr, Color, Metadata, FORMAT_ERROR_CAPACITY, FORMAT_INFOS_QR,
        FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK,
    },
    utils::{BitArray, BitStream, EncRegionIter, QRError, QRResult},
    ECLevel, MaskPattern, Version,
};

// Symbol
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Symbol {
    img: Arc<BinaryImage>,
    loc: SymbolLocation,
}

impl Symbol {
    pub fn new(img: Arc<BinaryImage>, loc: SymbolLocation) -> Self {
        Self { img, loc }
    }

    pub fn decode(&mut self) -> QRResult<(Metadata, String)> {
        let (ecl, mask) = self.read_format_info()?;
        let ver = self.loc.ver;
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

    /// Decodes using a per-module confidence grid, so the blocks are rectified as
    /// errors-and-erasures rather than errors alone.
    ///
    /// Reed-Solomon spends two parity symbols locating and fixing an error whose position it
    /// must discover, but only one on an *erasure* — a position already known to be suspect.
    /// A colour decoder knows more than the bit it emitted: it also knows how close the call
    /// was. Feeding that through lets a block survive up to `ec_len` corrupt codewords instead
    /// of `ec_len / 2`, provided the flags land on the corruption.
    ///
    /// `conf` is indexed `[y][x]` in module coordinates, higher meaning more reliable; its
    /// scale is arbitrary because only the ranking within a block is used. `steps` are
    /// fractions of `ec_len` to erase, tried in order until a block's parity checks clear, so
    /// `&[0.0]` reproduces [`Self::decode`] and a rising ladder costs nothing on blocks that
    /// were already clean.
    pub fn decode_with_confidence(
        &mut self,
        conf: &[Vec<f64>],
        steps: &[f64],
    ) -> QRResult<(Metadata, String)> {
        let (ecl, mask) = self.read_format_info()?;
        let ver = self.loc.ver;
        let hi_cap = self.read_capacity_info()?;

        let (pld, step_conf) = self.extract_payload_inner(&mask, Some(conf))?;

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let mut enc = BitStream::new(pld.len() << 3);
        let chan_cap = ver.channel_codewords();

        // A codeword is only as trustworthy as its least trustworthy module: its eight bits
        // come from eight consecutive steps of the region walk, and one bad module ruins the
        // byte. Every channel region shares the same step ordering, so this is computed once.
        let byte_conf: Vec<f64> = (0..chan_cap)
            .map(|j| {
                let hi = ((j + 1) * 8).min(step_conf.len());
                step_conf[(j * 8).min(hi)..hi].iter().copied().fold(f64::INFINITY, f64::min)
            })
            .collect();

        // The confidences ride the same interleave permutation as the codewords they describe.
        let conf_blks = deinterleave_vals(&byte_conf, blk_info, ec_len);

        for c in pld.data().chunks_exact(chan_cap) {
            let mut blocks = deinterleave(c, blk_info, ec_len);
            for (b, bc) in blocks.iter_mut().zip(conf_blks.iter()) {
                let rectified = rectify_laddered(b, bc, ec_len, steps)?;
                enc.extend(rectified);
            }
        }

        let msg = codec_decode(&mut enc, ver, ecl, hi_cap)?;
        let meta = Metadata::new(Some(ver), Some(ecl), Some(mask));

        Ok((meta, msg))
    }

    pub fn get(&self, x: i32, y: i32) -> QRResult<Color> {
        let (xp, yp) = self.wrap_coord(x, y);
        let tile = self.loc.tile_at(xp as usize, yp as usize)?;
        let pt = tile.map(xp as f64 + 0.5, yp as f64 + 0.5)?;
        self.img.get_at_point(&pt).ok_or(QRError::PixelOutOfBounds)
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

    #[inline]
    pub fn version(&self) -> Version {
        self.loc.ver
    }

    /// Projects a point given in module coordinates to its subpixel position in the source
    /// image, through the homography of the tile owning module `(x, y)`. `None` when the
    /// localizer left no tile there, or the mapping is degenerate.
    ///
    /// Exposed for the colour benchmark, which samples raw RGB per module rather than going
    /// through the binarized read path. `Tile` itself stays crate-private.
    #[cfg(feature = "benchmark")]
    pub fn exact_map(&self, x: usize, y: usize, dx: f64, dy: f64) -> Option<(f64, f64)> {
        let tile = self.loc.tile_at(x, y).ok()?;
        tile.exact_map(x as f64 + dx, y as f64 + dy).ok()
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
        Ok(self.extract_payload_inner(mask, None)?.0)
    }

    /// The payload read, optionally recording each region-walk step's module confidence so a
    /// caller can map it onto codewords. The second element is empty when `conf` is `None`.
    fn extract_payload_inner(
        &self,
        mask: &MaskPattern,
        conf: Option<&[Vec<f64>]>,
    ) -> QRResult<(BitArray, Vec<f64>)> {
        let ver = self.loc.ver;
        let mask_fn = mask.mask_functions();
        let chan_bits = ver.channel_codewords() << 3;
        let offsets = [2 * chan_bits, chan_bits, 0]; // B, G, R offsets
        let mut payload = BitArray::new(chan_bits * 3);
        let mut step_conf = Vec::with_capacity(if conf.is_some() { chan_bits } else { 0 });
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
            if let Some(cg) = conf {
                step_conf.push(cg[y as usize][x as usize]);
            }
        }

        debug_assert_eq!(
            rgn_iter.count(),
            self.loc.ver.remainder_bits(),
            "Remainder bits don't match"
        );

        Ok((payload, step_conf))
    }
}

/// Undoes the interleave for any per-codeword quantity — the codewords themselves, or a
/// parallel array describing them such as a confidence. Generic so a descriptor cannot drift
/// out of step with the bytes it annotates.
fn deinterleave_vals<T: Copy>(
    data: &[T],
    blk_info: (usize, usize, usize, usize),
    _ec_len: usize,
) -> Vec<Vec<T>> {
    // b1s = block1_size, b1c = block1_count
    let (b1s, b1c, b2s, b2c) = blk_info;

    let total_blks = b1c + b2c;
    let spl = b1s * total_blks;
    let data_sz = b1s * b1c + b2s * b2c;

    let mut dilvd: Vec<Vec<T>> = vec![Vec::with_capacity(b2s); total_blks];

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

    dilvd
}

fn deinterleave(data: &[u8], blk_info: (usize, usize, usize, usize), ec_len: usize) -> Vec<Block> {
    deinterleave_vals(data, blk_info, ec_len)
        .iter()
        .map(|b| Block::with_encoded(b, b.len() - ec_len))
        .collect()
}

/// Rectifies one block, escalating through `steps` (fractions of `ec_len` to erase, lowest
/// confidence first) and stopping at the first attempt whose parity checks clear.
///
/// Each attempt restarts from the block as received, because a failed rectification leaves
/// partially applied corrections behind.
fn rectify_laddered<'a>(
    blk: &'a mut Block,
    conf: &[f64],
    ec_len: usize,
    steps: &[f64],
) -> QRResult<&'a [u8]> {
    let pristine = *blk;
    let mut order: Vec<usize> = (0..conf.len().min(blk.len)).collect();
    order.sort_by(|&a, &b| conf[a].partial_cmp(&conf[b]).unwrap_or(std::cmp::Ordering::Equal));

    let mut last = Err(QRError::TooManyError);
    for &frac in steps {
        *blk = pristine;
        let n = ((frac * ec_len as f64).floor() as usize).min(order.len());
        let mut erased = vec![false; blk.len];
        for &i in order.iter().take(n) {
            erased[i] = true;
        }
        // Borrow-checker: probe on a copy, then redo the winning attempt on `blk` itself.
        let mut probe = pristine;
        match probe.rectify_with_erasures(&erased) {
            Ok(_) => {
                *blk = pristine;
                return blk.rectify_with_erasures(&erased);
            }
            Err(e) => last = Err(e),
        }
    }
    *blk = pristine;
    last
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
