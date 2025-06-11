use super::{
    binarize::{BinaryImage, Pixel},
    finder::FinderGroup,
    utils::{
        geometry::{Axis, BresenhamLine, Point, Slope},
        homography::Homography,
    },
};
use crate::{
    codec::decode as codec_decode,
    ec::{rectify_info, Block},
    metadata::{
        parse_format_info_qr, Color, Metadata, FORMAT_ERROR_CAPACITY, FORMAT_INFOS_QR,
        FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK, VERSION_ERROR_BIT_LEN,
        VERSION_ERROR_CAPACITY, VERSION_INFOS, VERSION_INFO_COORDS_BL, VERSION_INFO_COORDS_TR,
    },
    reader::utils::{
        geometry::{X, Y},
        verify_alignment_pattern,
    },
    utils::{BitArray, BitStream, EncRegionIter, QRError, QRResult},
    ECLevel, MaskPattern, Palette, Version,
};

#[cfg(test)]
use image::RgbImage;

#[cfg(test)]
use std::path::Path;

// Locates symbol based on 3 finder centres, their edge points & provisional grid size
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct SymbolLocation {
    h: Homography,
    anchors: [Point; 4],
    ver: Version,
}

impl SymbolLocation {
    // Below diagram shows the location of all centres and edge mid points
    // referenced in the group finder function
    // ****************************              ****************************
    // ****************************              ****************************
    // ****************************              ****************************
    // ****                   *****              *****                   ****
    // ****                   *****              *****                   ****
    // ****                   *****              *****                   ****
    // ****    ************   *****              *****   ************    ****
    // ****    *****c1*****   *m12*              *m21*   *****c2*****    ****
    // ****    ************   *****              *****   ************    ****
    // ****                   *****              *****                   ****
    // ****                   *****              *****                   ****
    // ****                   *****              *****                   ****
    // ****************************              ****************************
    // ************m10*************              ************m24*************
    // ****************************              ****************************
    //
    //
    //
    // ****************************
    // ************m01*************
    // ****************************
    // ****                   *****
    // ****                   *****
    // ****                   *****
    // ****    ************   *****
    // ****    *****c0*****   *m03*                           c3
    // ****    ************   *****
    // ****                   *****
    // ****                   *****
    // ****                   *****
    // ****************************
    // ****************************
    // ****************************
    pub fn locate(img: &mut BinaryImage, group: &mut FinderGroup) -> Option<SymbolLocation> {
        let [mut c0, c1, mut c2] = group.finders;

        // Compute provisional location of alignment centre (c4)
        let dx = c2.x - c1.x;
        let dy = c2.y - c1.y;
        let mut align = Point { x: c0.x + dx, y: c0.y + dy };

        // Skip if intersection pt is outside the image
        if align.x < 0 || align.x as u32 >= img.w || align.y < 0 || align.y as u32 >= img.h {
            return None;
        }

        // Hypotenuse slope
        let mut hm = Slope { dx: c2.x - c0.x, dy: c2.y - c0.y };

        // Make sure the middle(datum) finder is top-left and not bottom-right
        if (c1.y - c0.y) * hm.dx - (c1.x - c0.x) * hm.dy > 0 {
            group.finders.swap(0, 2);
            std::mem::swap(&mut c0, &mut c2);
            hm.dx *= -1;
            hm.dy *= -1;
        }

        // Locating midpoints for finder edges which cross the lines connecting the centres. In
        // other words the edges which don't lie on the boundary. These will be used as endpoints
        // to measure timing patterns, and also to locate the provisional alignment centre for
        // versions above 1.
        let mids = [
            find_edge_mid(img, &c0, &align)?,
            find_edge_mid(img, &c0, &c1)?,
            find_edge_mid(img, &c1, &c0)?,
            find_edge_mid(img, &c1, &c2)?,
            find_edge_mid(img, &c2, &c1)?,
            find_edge_mid(img, &c2, &align)?,
        ];

        let size = verify_symbol_size(img, group, &mids)?;

        let ver = Version::from_grid_size(size as usize)?;

        // For versions greater than 1, a more robust algorithm to locate align centre.
        // First, locate provisional centre from mid 1 with distance of c1 from mid 4.
        // Spiral out of provisional align pt to identify potential pt. Then compare the area of
        // black region with estimate module size to confirm alignment stone. Finally, locate the
        // centre of the stone.
        if *ver != 1 {
            let dx = mids[4].x - c1.x;
            let dy = mids[4].y - c1.y;
            let seed = Point { x: mids[1].x + dx, y: mids[1].y + dy };

            // Calculate estimate width of module
            let hor_w = c0.dist_sq(&mids[0]);
            let ver_w = c2.dist_sq(&mids[5]);
            let mod_w = ((hor_w + ver_w) as f64 / 2.0).sqrt() / 3.0;

            // Calculate estimate area of module by taking cross product of vectors
            let v0 = Slope::new(&c0, &mids[0]);
            let v1 = Slope::new(&c2, &mids[5]);
            let area = v0.cross(&v1).unsigned_abs() / 9;

            align = locate_alignment_pattern(img, seed, mod_w, area)?;
        }

        let h = setup_homography(img, group, align, ver)?;

        let anchors = [c1, c2, align, c0];

        Some(Self { h, anchors, ver })
    }
}

// Validates the symbol and returns its size if valid. Validation involves:
// 1. Ensuring the horizontal and vertical timing patterns are consistent.
// 2. Verifying that the estimated number of modules along the center matches the timing patterns.
fn verify_symbol_size(img: &BinaryImage, group: &FinderGroup, mids: &[Point; 6]) -> Option<u32> {
    let [c0, c1, c2] = &group.finders;
    let [m03, m01, m10, m12, m21, m23] = mids;

    // Measure timing pattern from c1 to c2
    let t12 = measure_timing_patterns(img, m10, m23);

    // Measure timing pattern from c1 to c3
    let t13 = measure_timing_patterns(img, m12, m03);

    // Closeness of horizontal and vertical timing patterns
    let timing_score = ((t12 as f64 / t13 as f64) - 1.0).abs();
    if timing_score > SYMBOL_HEURICTIC_THRESHOLD {
        return None;
    }

    // Estimate module count from c1 to c2
    let est_mod_count12 = estimate_mod_count(c1, m12, c2, m21);
    let mod_score12 = ((est_mod_count12 / (t12 + 6) as f64) - 1.0).abs();

    // Skip if one is more than twice as long as the other
    if mod_score12 > SYMBOL_HEURICTIC_THRESHOLD {
        return None;
    }

    // Estimate module count from c1 to c3
    let est_mod_count13 = estimate_mod_count(c1, m10, c0, m01);
    let mod_score13 = ((est_mod_count13 / (t13 + 6) as f64) - 1.0).abs();

    // Skip if one is more than twice as long as the other
    if mod_score13 > SYMBOL_HEURICTIC_THRESHOLD {
        return None;
    }

    // Provisional width and version
    let size = std::cmp::max(t12, t13) + 13;
    let ver = ((size as f64 - 15.0) / 4.0).floor() as u32;
    let size = ver * 4 + 17;

    Some(size)
}

fn find_edge_mid(img: &BinaryImage, from: &Point, to: &Point) -> Option<Point> {
    let dx = (to.x - from.x).abs();
    let dy = (to.y - from.y).abs();
    if dx > dy {
        mid_scan::<X>(img, from, to)
    } else {
        mid_scan::<Y>(img, from, to)
    }
}

fn mid_scan<A: Axis>(img: &BinaryImage, from: &Point, to: &Point) -> Option<Point>
where
    BresenhamLine<A>: Iterator<Item = Point>,
{
    let mut flips = 0;
    let mut buffer = Vec::with_capacity(100);
    let px = img.get_at_point(from).unwrap();
    let mut last = px.get_color();
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let px = img.get_at_point(&p).unwrap();
        let color = px.get_color();

        if color != last {
            flips += 1;
            last = color;
            if flips == 3 {
                let idx = buffer.len() * 6 / 7;
                let mid = buffer[idx];
                // let mid = buffer[buffer.len() / 2];
                return Some(mid);
            }
        }

        buffer.push(p);
    }

    None
}

pub fn measure_timing_patterns(img: &BinaryImage, from: &Point, to: &Point) -> u32 {
    let dx = (to.x - from.x).abs();
    let dy = (to.y - from.y).abs();

    if dx > dy {
        timing_scan::<X>(img, from, to)
    } else {
        timing_scan::<Y>(img, from, to)
    }
}

fn timing_scan<A: Axis>(img: &BinaryImage, from: &Point, to: &Point) -> u32
where
    BresenhamLine<A>: Iterator<Item = Point>,
{
    let mut transitions = [0, 0, 0];
    let px = img.get_at_point(from).unwrap();
    let mut last = px.get_color().to_bits();
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let px = img.get_at_point(&p).unwrap();
        let color = px.get_color().to_bits();
        for i in 0..3 {
            if color[i] != last[i] {
                transitions[i] += 1;
                last[i] = color[i];
            }
        }
    }

    *transitions.iter().min().unwrap()
}

fn estimate_mod_count(c1: &Point, m1: &Point, c2: &Point, m2: &Point) -> f64 {
    let d1 = c1.dist_sq(m1);
    let d2 = c2.dist_sq(m2);

    let avg_d = ((d1 + d2) / 2) as f64;
    let d12 = c1.dist_sq(c2) as f64;

    (d12 * 9.0 / avg_d).sqrt()
}

// Symbol
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Symbol<'a> {
    img: &'a BinaryImage,
    h: Homography,
    anchors: [Point; 4],
    pub ver: Version,
}

impl<'a> Symbol<'a> {
    pub fn new(img: &'a BinaryImage, sym_loc: SymbolLocation) -> Self {
        let SymbolLocation { h, anchors, ver } = sym_loc;
        Self { img, h, anchors, ver }
    }

    pub fn decode(&mut self) -> QRResult<(Metadata, String)> {
        let (ecl, mask) = self.read_format_info()?;
        if matches!(self.ver, Version::Normal(7..=40)) {
            self.ver = self.read_version_info()?;
        }
        let ver = self.ver;
        let pal = self.read_palette_info()?;

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

        let msg = codec_decode(&mut enc, ver, ecl, pal)?;

        Ok((Metadata::new(Some(ver), Some(ecl), Some(mask)), msg))
    }

    pub fn get(&self, x: i32, y: i32) -> Option<&Pixel> {
        let (xp, yp) = self.wrap_coord(x, y);
        let pt = self.map(xp as f64 + 0.5, yp as f64 + 0.5).ok()?;
        self.img.get_at_point(&pt)
    }

    fn wrap_coord(&self, x: i32, y: i32) -> (i32, i32) {
        let w = self.ver.width() as i32;
        debug_assert!(-w <= x && x < w, "x shouldn't be greater than or equal to w");
        debug_assert!(-w <= y && y < w, "y shouldn't be greater than or equal to w");

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + w } else { y };
        (x, y)
    }

    #[inline]
    pub fn map(&self, x: f64, y: f64) -> QRResult<Point> {
        self.h.map(x, y)
    }

    #[cfg(feature = "benchmark")]
    #[inline]
    pub fn raw_map(&self, x: f64, y: f64) -> QRResult<(f64, f64)> {
        self.h.raw_map(x, y)
    }

    #[cfg(test)]
    #[inline]
    pub fn save(&self, path: &Path) {
        self.img.save(path).unwrap()
    }

    #[cfg(test)]
    pub fn highlight(&self, img: &mut RgbImage) {
        use super::utils::geometry::{BresenhamLine, X, Y};
        use crate::reader::utils::rnd_rgb;

        let color = rnd_rgb();

        for p in self.anchors.iter() {
            p.highlight(img, color);
        }

        let (w, h) = img.dimensions();
        let sz = self.ver.width() as f64;
        let tl = self.map(0.0, 0.0).unwrap();
        let tr = self.map(sz, 0.0).unwrap();
        let br = self.map(sz, sz).unwrap();
        let bl = self.map(0.0, sz).unwrap();
        let bounds = [tl, tr, br, bl];

        for i in 0..4 {
            let mut a = bounds[i % 4];
            let mut b = bounds[(i + 1) % 4];
            let dx = (b.x - a.x).abs();
            let dy = (b.y - a.y).abs();

            a.x = (a.x.max(0) as u32).min(w - 1) as i32;
            a.y = (a.y.max(0) as u32).min(h - 1) as i32;
            b.x = (b.x.max(0) as u32).min(w - 1) as i32;
            b.y = (b.y.max(0) as u32).min(h - 1) as i32;

            if dx > dy {
                let line = BresenhamLine::<X>::new(&a, &b);
                for pt in line {
                    pt.highlight(img, color);
                }
            } else {
                let line = BresenhamLine::<Y>::new(&a, &b);
                for pt in line {
                    pt.highlight(img, color);
                }
            }
        }
    }
}

fn locate_alignment_pattern(
    img: &mut BinaryImage,
    mut seed: Point,
    mod_w: f64,
    area: u32,
) -> Option<Point> {
    let (w, h) = (img.w, img.h);
    let mod_w_i32 = mod_w as i32;
    let threshold = area * 2;
    let pattern = [1.0, 1.0, 1.0];

    // Directional increment for x & y: [right, down, left, up]
    const DX: [i32; 4] = [1, 0, -1, 0];
    const DY: [i32; 4] = [0, -1, 0, 1];

    // Spiral outward to find stone
    let mut dir = 0;
    let mut run_len = 1;
    let mut rejected = Vec::with_capacity(100);

    while run_len < mod_w_i32 * 15 {
        for _ in 0..run_len {
            let x = seed.x as u32;
            let y = seed.y as u32;

            if let Some(px) = img.get_at_point(&seed) {
                let color = px.get_color();

                if x < w && y < h && color == Color::Black {
                    let reg = img.get_region((x, y));
                    let (reg_centre, reg_area) = (reg.centre, reg.area);

                    if !rejected.contains(&reg_centre) {
                        // Check if region area is roughly equal to mod area with 100% tolerance
                        // and crosscheck 1:1:1 ratio horizontally and vertically
                        if reg_area <= threshold
                            && verify_alignment_pattern::<X>(
                                img,
                                &reg_centre,
                                &pattern,
                                mod_w,
                                threshold,
                            )
                            && verify_alignment_pattern::<Y>(
                                img,
                                &reg_centre,
                                &pattern,
                                mod_w,
                                threshold,
                            )
                        {
                            return Some(reg_centre);
                        } else {
                            rejected.push(reg_centre);
                        }
                    }
                }
            }

            seed.x += DX[dir];
            seed.y += DY[dir];
        }

        // Cycle direction
        dir = (dir + 1) & 3;
        if dir & 1 == 0 {
            run_len += 1;
        }
    }

    None
}

fn setup_homography(
    img: &BinaryImage,
    group: &FinderGroup,
    align_centre: Point,
    ver: Version,
) -> Option<Homography> {
    let size = ver.width() as f64;
    let br_off = if *ver == 1 { 3.5 } else { 6.5 };
    let src = [(3.5, 3.5), (size - 3.5, 3.5), (size - br_off, size - br_off), (3.5, size - 3.5)];

    let c0 = (group.finders[0].x as f64, group.finders[0].y as f64);
    let c1 = (group.finders[1].x as f64, group.finders[1].y as f64);
    let c2 = (group.finders[2].x as f64, group.finders[2].y as f64);
    let ca = (align_centre.x as f64, align_centre.y as f64);
    let dst = [c1, c2, ca, c0];

    let initial_h = Homography::compute(src, dst).ok()?;

    jiggle_homography(img, initial_h, ver)
}

// Adjust the homography slightly to refine projection of qr
fn jiggle_homography(img: &BinaryImage, mut h: Homography, ver: Version) -> Option<Homography> {
    let mut best = symbol_fitness(img, &h, ver);

    // Create an adjustment matrix by scaling the homography
    let mut adjustments = h.0.map(|x| x * 0.04);

    for _pass in 0..6 {
        for i in 0..8 {
            let old = h[i];
            for j in 0..2 {
                let step = adjustments[i];
                h[i] = if j & 1 == 0 { old - step } else { old + step };

                let test = symbol_fitness(img, &h, ver);
                if test > best {
                    best = test
                } else {
                    h[i] = old
                }
            }
        }

        // Halve all adjustment steps
        adjustments = adjustments.map(|x| x * 0.5);
    }
    let max_score = max_fitness_score(ver);
    if best >= max_score / 2 {
        Some(h)
    } else {
        None
    }
}

fn symbol_fitness(img: &BinaryImage, h: &Homography, ver: Version) -> i32 {
    let mut score = 0;
    let grid_size = ver.width() as i32;

    // Score timing patterns
    for i in 7..grid_size - 7 {
        let flip = if i & 1 == 0 { -1 } else { 1 };
        score += cell_fitness(img, h, i, 6) * flip;
        score += cell_fitness(img, h, 6, i) * flip;
    }

    // Score finders
    score += finder_fitness(img, h, 0, 0);
    score += finder_fitness(img, h, grid_size - 7, 0);
    score += finder_fitness(img, h, 0, grid_size - 7);

    // Score alignment patterns
    if *ver == 1 {
        return score;
    }
    let aps = ver.alignment_pattern();
    let len = aps.len();

    for i in aps[1..len - 1].iter() {
        score += alignment_fitness(img, h, 6, *i);
        score += alignment_fitness(img, h, *i, 6);
    }
    for i in aps[1..].iter() {
        for j in aps[1..].iter() {
            score += alignment_fitness(img, h, *i, *j);
        }
    }

    score
}

fn max_fitness_score(ver: Version) -> i32 {
    let mut max_score = 0;

    // Finder score
    max_score += 49 * 3;

    // Timing score
    let grid_size = ver.width() as i32;
    max_score += (grid_size - 14) * 2;

    // Alignment score
    let align_count = ver.alignment_pattern().len();
    max_score += 25 * align_count as i32;

    max_score
}

fn finder_fitness(img: &BinaryImage, h: &Homography, x: i32, y: i32) -> i32 {
    let (x, y) = (x + 3, y + 3);
    cell_fitness(img, h, x, y) + ring_fitness(img, h, x, y, 1) - ring_fitness(img, h, x, y, 2)
        + ring_fitness(img, h, x, y, 3)
}

fn alignment_fitness(img: &BinaryImage, h: &Homography, x: i32, y: i32) -> i32 {
    cell_fitness(img, h, x, y) - ring_fitness(img, h, x, y, 1) + ring_fitness(img, h, x, y, 2)
}

fn ring_fitness(img: &BinaryImage, h: &Homography, cx: i32, cy: i32, r: i32) -> i32 {
    let mut score = 0;

    for i in 0..r * 2 {
        score += cell_fitness(img, h, cx - r + i, cy - r);
        score += cell_fitness(img, h, cx - r, cy + r - i);
        score += cell_fitness(img, h, cx + r, cy - r + 1);
        score += cell_fitness(img, h, cx + r - i, cy + r);
    }

    score
}

fn cell_fitness(img: &BinaryImage, hm: &Homography, x: i32, y: i32) -> i32 {
    const OFFSETS: [f64; 3] = [0.3, 0.5, 0.7];
    let white = Color::White;
    let mut score = 0;

    for dy in OFFSETS.iter() {
        for dx in OFFSETS.iter() {
            let pt = match hm.map(x as f64 + dx, y as f64 + dy) {
                Ok(v) => v,
                Err(_) => return 0,
            };
            if let Some(px) = img.get_at_point(&pt) {
                let color = px.get_color();
                if color == white {
                    score -= 1;
                } else {
                    score += 1;
                }
            }
        }
    }
    score
}

#[cfg(test)]
mod symbol_tests {

    use crate::{
        reader::{
            binarize::BinaryImage,
            finder::{group_finders, locate_finders},
            locate_symbols,
        },
        ECLevel, MaskPattern, Palette, QRBuilder, Version,
    };

    #[test]
    fn test_locate_symbol_0() {
        let data = "Hello, world!🌎";
        let ver = Version::Normal(4);
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

        let img = qr.to_image(10);
        let exp_anchors = [(75, 75), (335, 75), (305, 305), (75, 335)];

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);
        for b in symbols[0].anchors {
            assert!(exp_anchors.contains(&(b.x, b.y)), "Symbol not within bounds");
        }
    }
}

// Read format, version & palette info
//------------------------------------------------------------------------------

impl Symbol<'_> {
    pub fn read_format_info(&self) -> QRResult<(ECLevel, MaskPattern)> {
        // Parse main format area
        if let Some(main) = self.get_number(&FORMAT_INFO_COORDS_QR_MAIN) {
            if let Ok(format) = rectify_info(main, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY) {
                let format = format ^ FORMAT_MASK;
                let (ecl, mask) = parse_format_info_qr(format);
                return Ok((ecl, mask));
            }
        }

        // Parse side format area
        if let Some(side) = self.get_number(&FORMAT_INFO_COORDS_QR_SIDE) {
            if let Ok(format) = rectify_info(side, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY) {
                let format = format ^ FORMAT_MASK;
                let (ecl, mask) = parse_format_info_qr(format);
                return Ok((ecl, mask));
            }
        }

        Err(QRError::InvalidFormatInfo)
    }

    pub fn read_version_info(&self) -> QRResult<Version> {
        // Parse bottom left version area
        if let Some(bl) = self.get_number(&VERSION_INFO_COORDS_BL) {
            if let Ok(v) = rectify_info(bl, &VERSION_INFOS, VERSION_ERROR_CAPACITY) {
                return Ok(Version::Normal(v as usize >> VERSION_ERROR_BIT_LEN));
            }
        }

        // Parse top right version area
        if let Some(tr) = self.get_number(&VERSION_INFO_COORDS_TR) {
            if let Ok(v) = rectify_info(tr, &VERSION_INFOS, VERSION_ERROR_CAPACITY) {
                return Ok(Version::Normal(v as usize >> VERSION_ERROR_BIT_LEN));
            }
        }

        Err(QRError::InvalidVersionInfo)
    }

    pub fn read_palette_info(&self) -> QRResult<Palette> {
        if let Some(px) = self.get(8, -8) {
            let color = px.get_color();

            if color == Color::Black {
                return Ok(Palette::Mono);
            } else {
                return Ok(Palette::Poly);
            }
        }

        Err(QRError::InvalidPaletteInfo)
    }

    pub fn get_number(&self, coords: &[(i32, i32)]) -> Option<u32> {
        let mut num = 0;
        for &(x, y) in coords {
            let color = self.get(x, y)?.get_color();
            let bit = (color != Color::White) as u32;
            num = (num << 1) | bit;
        }
        Some(num)
    }
}

#[cfg(test)]
mod symbol_infos_tests {

    use crate::{
        metadata::Color,
        reader::{
            binarize::BinaryImage,
            finder::{group_finders, locate_finders},
            locate_symbols,
        },
        ECLevel, MaskPattern, Module, QRBuilder, Version,
    };

    #[test]
    fn test_read_format_info_clean() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        let img = qr.to_image(3);

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);

        let fmt_info = symbols[0].read_format_info().expect("Failed to read format info");
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
        let img = qr.to_image(3);

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);

        let fmt_info = symbols[0].read_format_info().expect("Failed to read format info");
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
        let img = qr.to_image(3);

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);

        let fmt_info = symbols[0].read_format_info().expect("Failed to read format info");
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
        let img = qr.to_image(3);

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);

        let _ = symbols[0].read_format_info().expect("Failed to read format info");
    }

    #[test]
    fn test_read_version_info() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let img = qr.to_image(3);

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);

        let scanned_ver = symbols[0].read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    fn test_read_version_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(5, -9, Module::Format(Color::Black));
        qr.set(5, -10, Module::Format(Color::Black));
        qr.set(5, -11, Module::Format(Color::Black));
        let img = qr.to_image(3);

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);

        let scanned_ver = symbols[0].read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    fn test_read_version_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(5, -9, Module::Format(Color::Black));
        qr.set(5, -10, Module::Format(Color::Black));
        qr.set(5, -11, Module::Format(Color::Black));
        qr.set(4, -9, Module::Format(Color::White));
        let img = qr.to_image(3);

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);

        let scanned_ver = symbols[0].read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    #[should_panic]
    fn test_read_version_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(5, -9, Module::Format(Color::Black));
        qr.set(5, -10, Module::Format(Color::Black));
        qr.set(5, -11, Module::Format(Color::Black));
        qr.set(4, -9, Module::Format(Color::White));
        qr.set(-9, 5, Module::Format(Color::Black));
        qr.set(-10, 5, Module::Format(Color::Black));
        qr.set(-11, 5, Module::Format(Color::Black));
        qr.set(-9, 4, Module::Format(Color::White));
        let img = qr.to_image(3);

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);

        let _ = symbols[0].read_version_info().expect("Failed to read format info");
    }
}

// Extracts encoded data codewords and error correction codewords
//------------------------------------------------------------------------------

impl Symbol<'_> {
    pub fn extract_payload(&self, mask: &MaskPattern) -> QRResult<BitArray> {
        let ver = self.ver;
        let mask_fn = mask.mask_functions();
        let chan_bits = ver.channel_codewords() << 3;
        let (g_off, b_off) = (chan_bits, 2 * chan_bits);
        let mut payload = BitArray::new(chan_bits * 3);
        let mut rgn_iter = EncRegionIter::new(ver);

        for (i, (x, y)) in rgn_iter.by_ref().take(chan_bits).enumerate() {
            let px = self.get(x, y).ok_or(QRError::PixelOutOfBounds)?;
            let color = px.get_color();
            let [mut r, mut g, mut b] = color.to_bits();

            if !mask_fn(x, y) {
                r = !r;
                g = !g;
                b = !b;
            };

            payload.put(i, r);
            payload.put(i + g_off, g);
            payload.put(i + b_off, b);
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
        metadata::{ECLevel, Palette, Version},
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

        let mut bs = BitStream::new(ver.total_codewords(Palette::Mono) << 3);
        QRBuilder::interleave_into(&exp_blks, &mut bs);

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let blks = deinterleave(bs.data(), blk_info, ec_len);
        assert_eq!(blks, exp_blks);
    }
}

// Global constants
//------------------------------------------------------------------------------

pub const SYMBOL_HEURICTIC_THRESHOLD: f64 = 0.5;
