use std::cmp::max;

use image::Rgb;

use super::{
    finder::Finder,
    prepare::{Pixel, PreparedImage, Region},
    utils::{
        accumulate::TopLeftCornerFinder,
        geometry::{Axis, BresenhamLine, Homography, Line, Point, Slope, X, Y},
    },
};
use crate::{
    ec::rectify_info,
    metadata::{
        parse_format_info_qr, FORMAT_ERROR_CAPACITY, FORMAT_INFOS_QR, FORMAT_INFO_COORDS_QR_MAIN,
        FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK, VERSION_ERROR_BIT_LEN, VERSION_ERROR_CAPACITY,
        VERSION_INFOS, VERSION_INFO_COORDS_BL, VERSION_INFO_COORDS_TR,
    },
    utils::{BitArray, EncRegionIter, QRError, QRResult},
    ECLevel, MaskPattern, Version,
};

// Locates symbol based on 3 finders
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct SymbolLocation {
    h: Homography,
    bounds: [Point; 4],
    ver: Version,
}

impl SymbolLocation {
    pub fn locate(img: &mut PreparedImage, fds: &mut [Finder; 3]) -> Option<SymbolLocation> {
        let mut c0 = fds[0].center;
        let c1 = fds[1].center;
        let mut c2 = fds[2].center;

        // Hypotenuse slope
        let mut hm = Slope { dx: c2.x - c0.x, dy: c2.y - c0.y };

        // Make sure the middle finder is top-left and not bottom-right
        if (c1.y - c0.y) * hm.dx - (c1.x - c0.x) * hm.dy > 0 {
            fds.swap(0, 2);
            std::mem::swap(&mut c0, &mut c2);
            hm.dx = -hm.dx;
            hm.dy = -hm.dy;
        }

        // Rotate finders so the top left corner is first in the list
        fds.iter_mut().for_each(|f| f.rotate(&c0, &hm));

        let grid_size = measure_timing_patterns(img, fds);
        let ver = Version::from_grid_size(grid_size)?;

        let hor_line = Line::new(&fds[0].corners[0], &fds[0].corners[1]);
        let ver_line = Line::new(&fds[2].corners[0], &fds[2].corners[3]);
        let mut align_pt = hor_line.intersection(&ver_line)?;

        if grid_size > 21 {
            align_pt = locate_alignment_pattern(img, align_pt, &fds[0], &fds[2])?;

            let tlcf = TopLeftCornerFinder::new(&align_pt, &hm);
            let color = img.get_at_point(&align_pt);
            let src = (align_pt.x as u32, align_pt.y as u32);
            let to = Pixel::Reserved.to_rgb(color);

            let tlcf = img.fill_and_accumulate(src, to, tlcf);

            align_pt = tlcf.corner;
        }

        let h = setup_homography(img, fds, &align_pt, ver)?;

        let w = grid_size as f64;
        let bounds = [h.map(0.0, 0.0), h.map(w, 0.0), h.map(w, w), h.map(0.0, w)];

        Some(Self { h, bounds, ver })
    }
}

// Symbol
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Symbol {
    img: PreparedImage,
    h: Homography,
    bounds: [Point; 4],
    pub ver: Version,
}

impl Symbol {
    pub fn new(img: PreparedImage, sym_loc: SymbolLocation) -> Self {
        let SymbolLocation { h, bounds, ver } = sym_loc;
        Self { img, h, bounds, ver }
    }

    pub fn get(&self, x: i32, y: i32) -> &Rgb<u8> {
        let (x, y) = self.wrap_coord(x, y);
        let pt = self.map(x as f64 + 0.5, y as f64 + 0.5);
        self.img.get_at_point(&pt)
    }

    pub fn get_mut(&mut self, x: i32, y: i32) -> &mut Rgb<u8> {
        let (x, y) = self.wrap_coord(x, y);
        let pt = self.map(x as f64 + 0.5, y as f64 + 0.5);
        self.img.get_mut_at_point(&pt)
    }

    pub fn set(&mut self, x: i32, y: i32, px: Rgb<u8>) {
        *self.get_mut(x, y) = px
    }

    pub fn set_version(&mut self, ver: Version) {
        self.ver = ver;
    }

    fn wrap_coord(&self, x: i32, y: i32) -> (i32, i32) {
        let w = self.ver.width() as i32;
        debug_assert!(-w <= x && x < w, "row shouldn't be greater than or equal to w");
        debug_assert!(-w <= y && y < w, "column shouldn't be greater than or equal to w");

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + w } else { y };
        (x, y)
    }

    #[inline]
    pub fn map(&self, x: f64, y: f64) -> Point {
        self.h.map(x, y)
    }

    #[inline]
    pub fn unmap(&self, p: &Point) -> (f64, f64) {
        self.h.unmap(p)
    }
}

// Locates pt on the middle line of each finder's ring band
// This pt is nearest to the center of symbol
// Traces vert and hor lines along these middle pts to count modules
pub fn measure_timing_patterns(img: &PreparedImage, fds: &[Finder; 3]) -> usize {
    let p0 = fds[0].map(6.5, 0.5);
    let p1 = fds[1].map(6.5, 6.5);
    let p2 = fds[2].map(0.5, 6.5);

    // Measuring horizontal timing pattern
    let dx = (p2.x - p1.x).abs();
    let dy = (p2.y - p1.y).abs();
    let hscan =
        if dx > dy { timing_scan::<X>(img, &p1, &p2) } else { timing_scan::<Y>(img, &p1, &p2) };

    // Measuring vertical timing pattern
    let dx = (p0.x - p1.x).abs();
    let dy = (p0.y - p1.y).abs();
    let vscan =
        if dx > dy { timing_scan::<X>(img, &p1, &p0) } else { timing_scan::<Y>(img, &p1, &p0) };

    let scan = max(hscan, vscan);

    // Choose nearest valid grid size
    let size = scan + 13;
    let ver = (size as f64 - 15.0).floor() as usize / 4;
    ver * 4 + 17
}

fn timing_scan<A: Axis>(img: &PreparedImage, from: &Point, to: &Point) -> usize
where
    BresenhamLine<A>: Iterator<Item = Point>,
{
    const SEQUENCE: [Rgb<u8>; 3] = [Rgb([255, 0, 0]), Rgb([0, 255, 0]), Rgb([0, 0, 255])];
    let mut transitions = 0;
    let mut last = img.get_at_point(from);
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let px = img.get_at_point(&p);
        if px != last {
            transitions += 1;
            last = px;
        }
    }

    transitions
}

fn locate_alignment_pattern(
    img: &mut PreparedImage,
    mut seed: Point,
    f0: &Finder,
    f2: &Finder,
) -> Option<Point> {
    // Get the 2 adjacent corners from seed of alignment pattern
    let w = img.width();
    let h = img.height();
    let (x, y) = f0.unmap(&seed);
    let a = f0.map(x, y + 1.0);
    let (x, y) = f2.unmap(&seed);
    let c = f2.map(x + 1.0, y);

    // Compute estimate size of alignment stone using area of parallelogram formula
    let sz_est =
        ((a.x - seed.x) * -(c.y - seed.y) + (a.y - seed.y) * (c.x - seed.x)).unsigned_abs();

    // x & y increments w.r.t direction
    const DX: [i32; 4] = [1, 0, -1, 0];
    const DY: [i32; 4] = [0, -1, 0, 1];

    // Spiral outward to find stone
    let mut dir = 0;
    let mut run_len = 1;

    // WARN: 10 instead of 100 as multiplier for size estimate
    while run_len * run_len < sz_est * 10 {
        for _ in 0..run_len {
            let x = seed.x as u32;
            let y = seed.y as u32;
            let invalid = &Rgb([255, 255, 255]);

            if x < w && y < h && img.get_at_point(&seed) != invalid {
                let reg = img.get_region((x, y));
                let sz = match reg {
                    Some(Region { area, .. }) => area,
                    _ => continue,
                };

                // Match with expected size of alignment stone
                if sz_est / 2 <= sz && sz <= sz_est * 2 {
                    return Some(seed);
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
    img: &PreparedImage,
    fds: &[Finder; 3],
    align_stone_tl: &Point,
    ver: Version,
) -> Option<Homography> {
    let grid_size = ver.width();
    let grid_size = (grid_size - 7) as f64;
    let corners = [fds[1].corners[0], fds[2].corners[0], *align_stone_tl, fds[0].corners[0]];
    let initial_h = Homography::create(&corners, grid_size, grid_size)?;
    Some(jiggle_homography(img, initial_h, ver))
}

// Adjust the homography slightly to refine viewport of qr
fn jiggle_homography(img: &PreparedImage, mut h: Homography, ver: Version) -> Homography {
    let mut best = symbol_fitness(img, &h, ver);
    let mut adjustments = [0.0; 8];
    h.0.iter().enumerate().for_each(|(i, v)| adjustments[i] = v * 0.02f64);

    for _pass in 0..5 {
        for i in 0..16 {
            let j = i >> 1;
            let old = h[j];
            let step = adjustments[j];

            let new = if i & 1 == 0 { old - step } else { old + step };

            h[j] = new;
            let test = symbol_fitness(img, &h, ver);
            if test > best {
                best = test
            } else {
                h[j] = old
            }
        }

        for i in adjustments.iter_mut() {
            *i *= 0.5f64;
        }
    }
    h
}

fn symbol_fitness(img: &PreparedImage, h: &Homography, ver: Version) -> i32 {
    let mut score = 0;
    let grid_size = ver.width() as i32;

    // Score timing patterns
    // WARN: Using usize instead of i32 for i
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

fn finder_fitness(img: &PreparedImage, h: &Homography, x: i32, y: i32) -> i32 {
    let (x, y) = (x + 3, y + 3);
    cell_fitness(img, h, x, y) + ring_fitness(img, h, x, y, 1) - ring_fitness(img, h, x, y, 2)
        + ring_fitness(img, h, x, y, 3)
}

fn alignment_fitness(img: &PreparedImage, h: &Homography, x: i32, y: i32) -> i32 {
    cell_fitness(img, h, x, y) - ring_fitness(img, h, x, y, 1) + ring_fitness(img, h, x, y, 2)
}

fn ring_fitness(img: &PreparedImage, h: &Homography, cx: i32, cy: i32, r: i32) -> i32 {
    let mut score = 0;

    for i in 0..r * 2 {
        score += cell_fitness(img, h, cx - r + i, cy - r);
        score += cell_fitness(img, h, cx - r, cy + r - i);
        score += cell_fitness(img, h, cx + r, cy - r + 1);
        score += cell_fitness(img, h, cx + r - i, cy + r);
    }

    score
}

fn cell_fitness(img: &PreparedImage, hm: &Homography, x: i32, y: i32) -> i32 {
    const OFFSETS: [f64; 3] = [0.3, 0.5, 0.7];
    let w = img.width();
    let h = img.height();
    let white = &Rgb([255, 255, 255]);
    let mut score = 0;

    for dy in OFFSETS.iter() {
        for dx in OFFSETS.iter() {
            let pt = hm.map(x as f64 + dx, y as f64 + dy);
            if !(pt.x < 0 || w <= pt.x as u32 || pt.y < 0 || h <= pt.y as u32) {
                if img.get_at_point(&pt) == white {
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
mod symbol_highlight {
    use image::RgbImage;

    use crate::reader::utils::{
        geometry::{BresenhamLine, X, Y},
        Highlight,
    };

    use super::Symbol;

    impl Highlight for Symbol {
        fn highlight(&self, img: &mut RgbImage) {
            for (i, crn) in self.bounds.iter().enumerate() {
                let next = self.bounds[(i + 1) % 4];
                let dx = (next.x - crn.x).abs();
                let dy = (next.y - crn.y).abs();
                if dx > dy {
                    let line = BresenhamLine::<X>::new(crn, &next);
                    for pt in line {
                        pt.highlight(img);
                    }
                } else {
                    let line = BresenhamLine::<Y>::new(crn, &next);
                    for pt in line {
                        pt.highlight(img);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod symbol_tests {
    use std::path::Path;

    use crate::reader::{
        finder::{group_finders, locate_finders},
        locate_symbol,
        prepare::PreparedImage,
    };

    #[test]
    fn test_locate_symbol() {
        let path = Path::new("tests/images/clean.png");
        let img = image::open(path).unwrap().to_rgb8();
        let bounds = [(40, 40), (370, 40), (370, 370), (40, 370)];

        let mut img = PreparedImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbol = locate_symbol(img, groups).expect("No symbol found");
        for b in symbol.bounds {
            assert!(bounds.contains(&(b.x, b.y)), "Symbol not within bounds");
        }
    }
}

// Format & version info read and mark
//------------------------------------------------------------------------------

impl Symbol {
    pub fn read_format_info(&mut self) -> QRResult<(ECLevel, MaskPattern)> {
        let main = self.get_number(&FORMAT_INFO_COORDS_QR_MAIN);
        let mut f = rectify_info(main, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY)
            .or_else(|_| {
                let side = self.get_number(&FORMAT_INFO_COORDS_QR_SIDE);
                rectify_info(side, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY)
            })
            .or(Err(QRError::InvalidFormatInfo))?;

        self.mark_coords(&FORMAT_INFO_COORDS_QR_MAIN);
        self.mark_coords(&FORMAT_INFO_COORDS_QR_SIDE);
        let color = self.get(8, -8);
        self.set(8, -8, Pixel::Reserved.to_rgb(color));

        f ^= FORMAT_MASK;
        let (ecl, mask) = parse_format_info_qr(f);
        Ok((ecl, mask))
    }

    pub fn read_version_info(&mut self) -> QRResult<Version> {
        debug_assert!(
            !matches!(self.ver, Version::Micro(_) | Version::Normal(1..=6)),
            "Version is too small to read version info"
        );
        let bl = self.get_number(&VERSION_INFO_COORDS_BL);
        let v = rectify_info(bl, &VERSION_INFOS, VERSION_ERROR_CAPACITY)
            .or_else(|_| {
                let tr = self.get_number(&VERSION_INFO_COORDS_TR);
                rectify_info(tr, &VERSION_INFOS, VERSION_ERROR_CAPACITY)
            })
            .or(Err(QRError::InvalidVersionInfo))?;

        self.mark_coords(&VERSION_INFO_COORDS_BL);
        self.mark_coords(&VERSION_INFO_COORDS_TR);
        Ok(Version::Normal(v as usize >> VERSION_ERROR_BIT_LEN))
    }

    pub fn get_number(&mut self, coords: &[(i32, i32)]) -> u32 {
        let mut num = 0;
        for (y, x) in coords {
            let Rgb([r, g, b]) = *self.get(*x, *y);
            let bit = if r != 255 || g != 255 || b != 255 { 1 } else { 0 } as u32;
            num = (num << 1) | bit;
        }
        num
    }

    pub fn mark_coords(&mut self, coords: &[(i32, i32)]) {
        for (y, x) in coords {
            let color = self.get(*x, *y);
            self.set(*x, *y, Pixel::Reserved.to_rgb(color));
        }
    }
}

#[cfg(test)]
mod symbol_infos_tests {

    use crate::{
        reader::{
            finder::{group_finders, locate_finders},
            locate_symbol,
            prepare::PreparedImage,
        },
        ECLevel, MaskPattern, QRBuilder, Version,
    };

    #[test]
    fn test_read_format_info() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        let img = qr.to_image(10);

        let mut img = PreparedImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let fmt_info = symbol.read_format_info().expect("Failed to read format info");
        assert_eq!(fmt_info, (ecl, mask));
    }
}

// Mark all function patterns
//------------------------------------------------------------------------------

// Marks all function pattern so they are ignored while extracting data
impl Symbol {
    pub fn mark_all_function_patterns(&mut self) {
        self.mark_finder_patterns();
        self.mark_timing_patterns();
        self.mark_alignment_patterns();
    }
}

// Mark finder pattern
//------------------------------------------------------------------------------

impl Symbol {
    pub fn mark_finder_patterns(&mut self) {
        self.mark_finder_pattern_at(3, 3);
        match self.ver {
            Version::Micro(_) => {}
            Version::Normal(_) => {
                self.mark_finder_pattern_at(3, -4);
                self.mark_finder_pattern_at(-4, 3);
            }
        }
    }

    fn mark_finder_pattern_at(&mut self, x: i32, y: i32) {
        let (dy_l, dy_r) = if y > 0 { (-3, 4) } else { (-4, 3) };
        let (dx_t, dx_b) = if x > 0 { (-3, 4) } else { (-4, 3) };
        for i in dy_l..=dy_r {
            for j in dx_t..=dx_b {
                let color = self.get(x + j, y + i);
                self.set(x + j, y + i, Pixel::Reserved.to_rgb(color));
            }
        }
    }
}

// Mark timing patterns
//------------------------------------------------------------------------------

impl Symbol {
    pub fn mark_timing_patterns(&mut self) {
        let w = self.ver.width() as i32;
        let (off, last) = match self.ver {
            Version::Micro(_) => (0, w - 1),
            Version::Normal(_) => (6, w - 9),
        };
        self.mark_line(8, off, last, off);
        self.mark_line(off, 8, off, last);
    }

    fn mark_line(&mut self, x1: i32, y1: i32, x2: i32, y2: i32) {
        debug_assert!(x1 == x2 || y1 == y2, "Line is neither vertical nor horizontal");

        if x1 == x2 {
            for j in y1..=y2 {
                let color = self.get(x1, j);
                self.set(x1, j, Pixel::Reserved.to_rgb(color));
            }
        } else {
            for i in x1..=x2 {
                let color = self.get(i, y1);
                self.set(i, y1, Pixel::Reserved.to_rgb(color));
            }
        }
    }
}

// Mark alignment patterns
//------------------------------------------------------------------------------

impl Symbol {
    pub fn mark_alignment_patterns(&mut self) {
        let poses = self.ver.alignment_pattern();
        for &x in poses {
            for &y in poses {
                self.mark_alignment_pattern_at(x, y);
            }
        }
    }

    fn mark_alignment_pattern_at(&mut self, x: i32, y: i32) {
        let w = self.ver.width() as i32;
        if (x == 6 && (y == 6 || y - w == -7)) || (x - w == -7 && y == 6) {
            return;
        }
        for i in -2..=2 {
            for j in -2..=2 {
                let color = self.get(x + i, y + j);
                self.set(x + i, y + j, Pixel::Reserved.to_rgb(color));
            }
        }
    }
}

// Extracts encoded data codewords and error correction codewords
//------------------------------------------------------------------------------

impl Symbol {
    // TODO: Write testcases
    pub fn extract_payload(&mut self, mask: &MaskPattern) -> BitArray {
        let ver = self.ver;
        let mask_fn = mask.mask_functions();
        let chan_bits = ver.channel_codewords() << 3;
        let (g_off, b_off) = (chan_bits, 2 * chan_bits);
        let mut pyld = BitArray::new(chan_bits * 3);
        let mut rgn_iter = EncRegionIter::new(ver);

        for i in 0..chan_bits {
            for (y, x) in rgn_iter.by_ref() {
                let color = self.get(x, y);
                let px: Pixel = (*color).into();
                if !matches!(px, Pixel::Reserved) {
                    let mut r = color[0] == 255;
                    let mut g = color[1] == 255;
                    let mut b = color[2] == 255;
                    if !mask_fn(y, x) {
                        r = !r;
                        g = !g;
                        b = !b;
                    };
                    pyld.put(i, r);
                    pyld.put(i + g_off, g);
                    pyld.put(i + b_off, b);
                    break;
                }
            }
        }
        pyld
    }
}
