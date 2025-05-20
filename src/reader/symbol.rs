use std::path::Path;

use super::{
    binarize::{BinaryImage, Pixel, Region},
    finder::FinderGroup,
    utils::{
        accumulate::CenterLocator,
        geometry::{Homography, Line, Point, Slope},
    },
};
use crate::{
    ec::rectify_info,
    metadata::{
        parse_format_info_qr, Color, FORMAT_ERROR_CAPACITY, FORMAT_INFOS_QR,
        FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK, VERSION_ERROR_BIT_LEN,
        VERSION_ERROR_CAPACITY, VERSION_INFOS, VERSION_INFO_COORDS_BL, VERSION_INFO_COORDS_TR,
    },
    utils::{BitArray, EncRegionIter, QRError, QRResult},
    ECLevel, MaskPattern, Palette, Version,
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
    pub fn locate(img: &mut BinaryImage, group: &mut FinderGroup) -> Option<SymbolLocation> {
        let mut c0 = group.finders[0].center;
        let c1 = group.finders[1].center;
        let mut c2 = group.finders[2].center;

        // Hypotenuse slope
        let mut hm = Slope { dx: c2.x - c0.x, dy: c2.y - c0.y };

        // Make sure the middle(datum) finder is top-left and not bottom-right
        if (c1.y - c0.y) * hm.dx - (c1.x - c0.x) * hm.dy > 0 {
            group.finders.swap(0, 2);
            std::mem::swap(&mut c0, &mut c2);
            group.mids.reverse();
            hm.dx = -hm.dx;
            hm.dy = -hm.dy;
        }

        let ver = Version::from_grid_size(group.size as usize)?;

        let hm = Slope::new(&c1, &c2);
        let hor_line = Line::from_point_slope(&group.mids[1], &hm);
        let vm = Slope::new(&c1, &c0);
        let ver_line = Line::from_point_slope(&group.mids[4], &vm);
        let mut align_seed = hor_line.intersection(&ver_line)?;

        // Exit if projected alignment pt is outside the image
        let Point { x: ax, y: ay } = align_seed;
        if ax < 0 || ax as u32 >= img.w || ay < 0 || ay as u32 > img.h {
            return None;
        }

        if let Version::Normal(2..=40) = ver {
            align_seed = locate_alignment_pattern(img, group, align_seed)?;

            let cl = CenterLocator::new();
            let color = Color::from(*img.get_at_point(&align_seed));
            let src = (align_seed.x as u32, align_seed.y as u32);
            let to = Pixel::Reserved(color);

            let cl = img.fill_and_accumulate(src, to, cl);
            align_seed = cl.get_center();
        }

        let h = setup_homography(img, group, align_seed)?;

        let w = group.size as f64;
        let bounds = [h.map(0.0, 0.0), h.map(w, 0.0), h.map(w, w), h.map(0.0, w)];

        Some(Self { h, bounds, ver })
    }
}

// Symbol
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Symbol {
    img: BinaryImage,
    h: Homography,
    bounds: [Point; 4],
    pub ver: Version,
}

impl Symbol {
    pub fn new(img: BinaryImage, sym_loc: SymbolLocation) -> Self {
        let SymbolLocation { h, bounds, ver } = sym_loc;
        Self { img, h, bounds, ver }
    }

    pub fn get(&self, x: i32, y: i32) -> &Pixel {
        let (x, y) = self.wrap_coord(x, y);
        let pt = self.map(x as f64 + 0.5, y as f64 + 0.5);
        self.img.get_at_point(&pt)
    }

    pub fn get_mut(&mut self, x: i32, y: i32) -> &mut Pixel {
        let (x, y) = self.wrap_coord(x, y);
        let pt = self.map(x as f64 + 0.5, y as f64 + 0.5);
        self.img.get_mut_at_point(&pt)
    }

    pub fn set(&mut self, x: i32, y: i32, px: Pixel) {
        *self.get_mut(x, y) = px
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
    pub fn map(&self, x: f64, y: f64) -> Point {
        self.h.map(x, y)
    }

    #[inline]
    pub fn save(&self, path: &Path) {
        self.img.save(path).unwrap()
    }
}

fn locate_alignment_pattern(
    img: &mut BinaryImage,
    group: &FinderGroup,
    mut seed: Point,
) -> Option<Point> {
    let (w, h) = (img.w, img.h);

    // Calculate area of module
    let m0 = Slope::new(&group.finders[0].center, &group.mids[0]);
    let m1 = Slope::new(&group.finders[1].center, &group.mids[5]);
    let mod_area = m0.cross(&m1).unsigned_abs();

    // x & y increments w.r.t direction
    const DX: [i32; 4] = [1, 0, -1, 0];
    const DY: [i32; 4] = [0, -1, 0, 1];

    // Spiral outward to find stone
    let mut dir = 0;
    let mut run_len = 1;

    let invalid = Color::White;
    // WARN: 10 instead of 100 as multiplier for size estimate
    while run_len * run_len < mod_area * 10 {
        for _ in 0..run_len {
            let x = seed.x as u32;
            let y = seed.y as u32;

            let color = Color::from(*img.get_at_point(&seed));
            if x < w && y < h && color != invalid {
                let reg = img.get_region((x, y));
                let sz = match reg {
                    Some(Region { area, .. }) => area,
                    _ => continue,
                };

                // Match with expected size of alignment stone
                if mod_area / 2 <= sz && sz <= mod_area * 2 {
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
    img: &BinaryImage,
    group: &FinderGroup,
    align_center: Point,
) -> Option<Homography> {
    let size = group.size as f64;
    let src = [(3.5, 3.5), (size - 3.5, 3.5), (size - 6.5, size - 6.5), (3.5, size - 3.5)];

    let c0 = (group.finders[0].center.x as f64, group.finders[0].center.y as f64);
    let c1 = (group.finders[1].center.x as f64, group.finders[1].center.y as f64);
    let c2 = (group.finders[2].center.x as f64, group.finders[2].center.y as f64);
    let ca = (align_center.x as f64, align_center.y as f64);
    let dst = [c1, c2, ca, c0];

    let initial_h = Homography::compute(src, dst)?;

    let ver = Version::from_grid_size(group.size as usize)?;

    Some(jiggle_homography(img, initial_h, ver))
}

// Adjust the homography slightly to refine viewport of qr
fn jiggle_homography(img: &BinaryImage, mut h: Homography, ver: Version) -> Homography {
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

fn symbol_fitness(img: &BinaryImage, h: &Homography, ver: Version) -> i32 {
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
    let w = img.w;
    let h = img.h;
    let white = Color::White;
    let mut score = 0;

    for dy in OFFSETS.iter() {
        for dx in OFFSETS.iter() {
            let pt = hm.map(x as f64 + dx, y as f64 + dy);
            if !(pt.x < 0 || w <= pt.x as u32 || pt.y < 0 || h <= pt.y as u32) {
                let color = Color::from(*img.get_at_point(&pt));
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

    use crate::{
        reader::{
            binarize::BinaryImage,
            finder::{group_finders, locate_finders},
            locate_symbol,
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
        let bounds = [(40, 40), (370, 40), (370, 370), (40, 370)];

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let symbol = locate_symbol(img, groups).expect("No symbol found");
        for b in symbol.bounds {
            assert!(bounds.contains(&(b.x, b.y)), "Symbol not within bounds");
        }
    }
}

// Format, version & palette info read and mark
//------------------------------------------------------------------------------

impl Symbol {
    pub fn read_format_info(&mut self) -> QRResult<(ECLevel, MaskPattern)> {
        let main = self.get_number(&FORMAT_INFO_COORDS_QR_MAIN);
        let mut format = rectify_info(main, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY)
            .or_else(|_| {
                let side = self.get_number(&FORMAT_INFO_COORDS_QR_SIDE);
                rectify_info(side, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY)
            })
            .or(Err(QRError::InvalidFormatInfo))?;

        self.mark_coords(&FORMAT_INFO_COORDS_QR_MAIN);
        self.mark_coords(&FORMAT_INFO_COORDS_QR_SIDE);

        format ^= FORMAT_MASK;
        let (ecl, mask) = parse_format_info_qr(format);
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

    pub fn read_palette_info(&mut self) -> Palette {
        let color = Color::from(*self.get(8, -8));
        self.set(8, -8, Pixel::Reserved(color));

        if color == Color::Black {
            Palette::Mono
        } else {
            Palette::Poly
        }
    }

    pub fn get_number(&mut self, coords: &[(i32, i32)]) -> u32 {
        let mut num = 0;
        for (y, x) in coords {
            let color = Color::from(*self.get(*x, *y));
            let bit = (color != Color::White) as u32;
            num = (num << 1) | bit;
        }
        num
    }

    pub fn mark_coords(&mut self, coords: &[(i32, i32)]) {
        for (y, x) in coords {
            let color = Color::from(*self.get(*x, *y));
            self.set(*x, *y, Pixel::Reserved(color));
        }
    }
}

#[cfg(test)]
mod symbol_infos_tests {

    use crate::{
        metadata::Color,
        reader::{
            binarize::BinaryImage,
            finder::{group_finders, locate_finders},
            locate_symbol,
        },
        ECLevel, MaskPattern, Module, QRBuilder, Version,
    };

    #[test]
    fn test_read_format_info() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let fmt_info = symbol.read_format_info().expect("Failed to read format info");
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
        qr.set(8, 1, Module::Format(Color::White));
        qr.set(8, 2, Module::Format(Color::White));
        qr.set(8, 4, Module::Format(Color::Black));
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let fmt_info = symbol.read_format_info().expect("Failed to read format info");
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
        qr.set(8, 1, Module::Format(Color::White));
        qr.set(8, 2, Module::Format(Color::White));
        qr.set(8, 3, Module::Format(Color::Black));
        qr.set(8, 4, Module::Format(Color::Black));
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let fmt_info = symbol.read_format_info().expect("Failed to read format info");
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
        qr.set(8, 1, Module::Format(Color::White));
        qr.set(8, 2, Module::Format(Color::White));
        qr.set(8, 3, Module::Format(Color::Black));
        qr.set(8, 4, Module::Format(Color::Black));
        qr.set(-2, 8, Module::Format(Color::White));
        qr.set(-3, 8, Module::Format(Color::White));
        qr.set(-4, 8, Module::Format(Color::Black));
        qr.set(-5, 8, Module::Format(Color::Black));
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let _ = symbol.read_format_info().expect("Failed to read format info");
    }

    #[test]
    fn test_read_version_info() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let scanned_ver = symbol.read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    fn test_read_version_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Black));
        qr.set(-10, 5, Module::Format(Color::Black));
        qr.set(-11, 5, Module::Format(Color::Black));
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let scanned_ver = symbol.read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    fn test_read_version_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Black));
        qr.set(-10, 5, Module::Format(Color::Black));
        qr.set(-11, 5, Module::Format(Color::Black));
        qr.set(-9, 4, Module::Format(Color::White));
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let scanned_ver = symbol.read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    #[should_panic]
    fn test_read_version_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(-9, 5, Module::Format(Color::Black));
        qr.set(-10, 5, Module::Format(Color::Black));
        qr.set(-11, 5, Module::Format(Color::Black));
        qr.set(-9, 4, Module::Format(Color::White));
        qr.set(5, -9, Module::Format(Color::Black));
        qr.set(5, -10, Module::Format(Color::Black));
        qr.set(5, -11, Module::Format(Color::Black));
        qr.set(4, -9, Module::Format(Color::White));
        let img = qr.to_image(3);

        let mut img = BinaryImage::prepare(img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&img, &finders);
        let mut symbol = locate_symbol(img, groups).expect("Symbol not found");

        let _ = symbol.read_version_info().expect("Failed to read format info");
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
                let color = Color::from(*self.get(x + j, y + i));
                self.set(x + j, y + i, Pixel::Reserved(color));
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
                let color = Color::from(*self.get(x1, j));
                self.set(x1, j, Pixel::Reserved(color));
            }
        } else {
            for i in x1..=x2 {
                let color = Color::from(*self.get(i, y1));
                self.set(i, y1, Pixel::Reserved(color));
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
                let color = Color::from(*self.get(x + i, y + j));
                self.set(x + i, y + j, Pixel::Reserved(color));
            }
        }
    }
}

// Extracts encoded data codewords and error correction codewords
//------------------------------------------------------------------------------

impl Symbol {
    pub fn extract_payload(&mut self, mask: &MaskPattern) -> BitArray {
        let ver = self.ver;
        let mask_fn = mask.mask_functions();
        let chan_bits = ver.channel_codewords() << 3;
        let (g_off, b_off) = (chan_bits, 2 * chan_bits);
        let mut pyld = BitArray::new(chan_bits * 3);
        let mut rgn_iter = EncRegionIter::new(ver);

        for i in 0..chan_bits {
            for (y, x) in rgn_iter.by_ref() {
                let px = self.get(x, y);
                if !matches!(px, Pixel::Reserved(_)) {
                    let color = Color::from(*px);
                    let [mut r, mut g, mut b] = color.to_bits();
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
