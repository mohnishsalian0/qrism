use std::cmp::Ordering;

use super::{
    alignment::{
        alignment_coords, infer_alignment_centres, locate_alignment_centres, Anchors,
        MAX_ALIGN_CELLS,
    },
    binarize::BinaryImage,
    finder::FinderGroup,
    tile::{band_table, build_tiles, Tile, MAX_WIDTH},
    utils::{
        frame::LocalFrame,
        geometry::{Axis, BresenhamLine, Point, Slope, X, Y},
    },
};
use crate::{
    ec::rectify_info,
    metadata::{Color, VERSION_ERROR_BIT_LEN, VERSION_ERROR_CAPACITY, VERSION_INFOS},
    utils::{QRError, QRResult},
    Version,
};

// Locates symbol based on 3 finder centres, their edge points & provisional grid size
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SymbolLocation {
    pub(super) ver: Version,
    pub(super) tiles: [[Option<Tile>; 6]; 6],
    pub(super) bands: [u8; MAX_WIDTH], // module -> tile, both axes
    pub(super) _anchors: Anchors,
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
    // ************m10*************              ************m23*************
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

        // Compute provisional location of alignment centre (c3)
        let dx = c2.x - c1.x;
        let dy = c2.y - c1.y;
        let align = Point { x: c0.x + dx, y: c0.y + dy };

        // Skip if intersection pt is outside the image
        if align.x < 0 || align.x as u32 >= img.w || align.y < 0 || align.y as u32 >= img.h {
            return None;
        }

        // Hypotenuse slope
        let mut hm = Slope::new(&c0, &c2);

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
            find_ring_mid(img, &c0, &align)?,
            find_ring_mid(img, &c0, &c1)?,
            find_ring_mid(img, &c1, &c0)?,
            find_ring_mid(img, &c1, &c2)?,
            find_ring_mid(img, &c2, &c1)?,
            find_ring_mid(img, &c2, &align)?,
        ];

        let size = verify_symbol_size(img, &group.finders, &mids)?;
        let ver = Version::from_grid_size(size as usize)?;
        let span = (ver.width() - 7) as f64; // Modules between finders
        let ff = LocalFrame::new(&c1, &c2, &c0, span, span); // Finders frame

        // Alignement pattern points
        let mut align_centres: Anchors = [[None; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];

        // Predict each alignment centre from the 3 finder centres, then spiral out of the
        // prediction to identify the potential stone. Confirm it by sweeping the white ring the
        // stone sits in, and settle on the centre of the stone.
        locate_alignment_centres(img, &group.finders, ver, &ff, &mut align_centres);

        // Unfound alignment centres are inferred from the intersection of horizontal & vertical
        // lines built with nearest found alignment centres
        infer_alignment_centres(ver, &ff, &mut align_centres);

        let tiles = build_tiles(ver, &align_centres);
        let bands = band_table(ver);

        let sym_loc = Self { ver, tiles, bands, _anchors: align_centres };

        // Reject symbol with fitness score below 40% max score
        if sym_loc.symbol_fitness(img) < max_fitness_score(ver) * 40 / 100 {
            return None;
        }

        Some(sym_loc)
    }

    // The symbol's outline in image space: TL, TR, BR, BL. These sit on the symbol boundary -- `sz` is
    // one past the last module on two sides -- so they are not module lookups; each corner is
    // projected through the outermost tile that meets it.
    pub fn outline(&self) -> QRResult<[(f64, f64); 4]> {
        let sz = self.ver.width() as f64;
        let last = self.ver.alignment_pattern().len().max(2) - 2;

        // Each corner paired with the tile that owns it, so the two can't drift apart.
        let corners = [
            ((0, 0), (0.0, 0.0)),
            ((0, last), (sz, 0.0)),
            ((last, last), (sz, sz)),
            ((last, 0), (0.0, sz)),
        ];

        let mut out = [(0.0, 0.0); 4];
        for (o, &((tr, tc), (x, y))) in out.iter_mut().zip(corners.iter()) {
            *o = self.tiles[tr][tc].as_ref().ok_or(QRError::TileNotFound)?.exact_map(x, y)?;
        }
        Ok(out)
    }

    #[cfg(test)]
    pub fn highlight(&self, img: &mut image::RgbImage) {
        use super::utils::geometry::{BresenhamLine, X, Y};
        use crate::reader::utils::rnd_rgb;

        let color = rnd_rgb();

        for &a in self._anchors.iter().flatten() {
            if let Some(pt) = a {
                pt.highlight(img, color);
            }
        }

        let (w, h) = img.dimensions();
        let Ok(corners) = self.outline() else { return };
        let bounds = corners.map(|(x, y)| Point { x: x.round() as i32, y: y.round() as i32 });

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

// Verifies the symbol and returns its size in modules, or None if the measurements disagree.
//
// The size is first estimated geometrically: the module count along each leg (c1->c2 and
// c1->c0) is derived from the finder centre-to-centre distances scaled by the finder widths,
// then snapped to the nearest valid size. The leg needing the smaller correction wins.
//
// That estimate is then confirmed against a second, independent reading of the size:
// 1. Below version 7 (size 45), by counting transitions along both timing patterns. Each run
//    must agree with the module count estimated for its own leg, and the two runs must agree
//    with each other.
// 2. From version 7 up, by decoding the version info blocks beside the BL & TR finders. The
//    block that needed fewer error corrections wins; if neither decodes, the geometric
//    estimate stands in. The resulting size must agree with that estimate.
//
// Every comparison is a relative difference measured against SYMBOL_HEURISTIC_THRESHOLD.
fn verify_symbol_size(img: &BinaryImage, finders: &[Point; 3], mids: &[Point; 6]) -> Option<u32> {
    let [c0, c1, c2] = finders;
    let [m03, m01, m10, m12, m21, m23] = mids;

    // Estimate module count from c1 to c2. The error estimates how far is the symbol dimension
    // from closest valid dimension. Size is the closest valid dimension
    let mc12 = estimate_mod_count(c1, m12, c2, m21);
    let (size12, err12) = nearest_valid_size(mc12);

    // Estimate module count from c1 to c3
    let mc10 = estimate_mod_count(c1, m10, c0, m01);
    let (size10, err10) = nearest_valid_size(mc10);

    let est_size = match err12.abs().cmp(&err10.abs()) {
        Ordering::Less => size12,
        Ordering::Equal => size12.min(size10),
        Ordering::Greater => size10,
    } as u32;

    // For version 6 (size 41) or below, use timing pattern
    // For version 7 (size 45) or above, use version info bits
    let size = if est_size < 45 {
        // Measure timing pattern from c1 to c2
        let t12 = measure_timing_patterns(img, m10, m23);
        let mod_score12 = ((mc12 / (t12 + 6) as f64) - 1.0).abs();

        // Skip if one is more than twice as long as the other
        if mod_score12 > SYMBOL_HEURISTIC_THRESHOLD {
            return None;
        }

        // Measure timing pattern from c1 to c3
        let t10 = measure_timing_patterns(img, m12, m03);
        let mod_score10 = ((mc10 / (t10 + 6) as f64) - 1.0).abs();

        // Skip if one is more than twice as long as the other
        if mod_score10 > SYMBOL_HEURISTIC_THRESHOLD {
            return None;
        }

        // Closeness of horizontal and vertical timing patterns
        let timing_score = ((t12 as f64 / t10 as f64) - 1.0).abs();
        if timing_score > SYMBOL_HEURISTIC_THRESHOLD {
            return None;
        }

        // Provisional width and version
        let size = (t12 + t10) / 2 + 13;
        let ver = ((size as f64 - 15.0) / 4.0).floor() as u32;
        ver * 4 + 17
    } else {
        // BL & TR finder local frames of reference
        let blf = LocalFrame::new(c0, m03, m01, 3.0, 3.0);
        let trf = LocalFrame::new(c2, m23, m21, 3.0, 3.0);

        // Read version from BL & TR version info
        let blvi = read_version_info(img, blf);
        let trvi = read_version_info(img, trf);

        let ver = match (blvi, trvi) {
            (Some((blv, blerr)), Some((trv, trerr))) => match blerr.cmp(&trerr) {
                Ordering::Less => blv,
                Ordering::Greater => trv,
                Ordering::Equal => blv.min(trv),
            },
            (Some((v, _)), None) | (None, Some((v, _))) => v,
            (None, None) => (est_size - 17) / 4,
        };

        let size = ver * 4 + 17;

        let score = ((size as f64 / est_size as f64) - 1.0).abs();
        if score > SYMBOL_HEURISTIC_THRESHOLD {
            return None;
        }

        size
    };

    Some(size)
}

// Snaps a centre-to-centre module count to the nearest valid symbol size (always 1 mod 4),
// returning that size and the signed distance from the estimate to it.
fn nearest_valid_size(mod_count: f64) -> (i32, i32) {
    let est = mod_count.round() as i32 + 7;
    let err = 1 - (est % 4);
    (est + err, err)
}

fn find_ring_mid(img: &BinaryImage, from: &Point, to: &Point) -> Option<Point> {
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
    let mut last = img.get_at_point(from).unwrap();
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let color = img.get_at_point(&p).unwrap();

        if color != last {
            flips += 1;
            last = color;
            if flips == 3 {
                let idx = buffer.len() * 6 / 7;
                let mid = buffer[idx];
                return Some(mid);
            }
        }

        buffer.push(p);
    }

    None
}

fn measure_timing_patterns(img: &BinaryImage, from: &Point, to: &Point) -> u32 {
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
    let mut transitions = [0; 3];
    let mut last = img.get_at_point(from).unwrap() as u8;
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let color = img.get_at_point(&p).unwrap() as u8;
        for (i, t) in transitions.iter_mut().enumerate() {
            if color >> i != last >> i {
                *t += 1;
                last ^= 1 << i;
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

fn read_version_info(img: &BinaryImage, fr: LocalFrame) -> Option<(u32, u32)> {
    let mut vinfo = 0;
    for x in (-3..3).rev() {
        for y in 5..8 {
            let pt = fr.map(x as f64, y as f64);
            let clr = img.get_at_point(&pt)?;
            let bit = (clr != Color::White) as u32;
            vinfo = (vinfo << 1) | bit;
        }
    }

    rectify_info(vinfo, &VERSION_INFOS, VERSION_ERROR_CAPACITY)
        .map(|(v, e)| (v >> VERSION_ERROR_BIT_LEN, e))
        .ok()
}

#[cfg(test)]
mod symbol_locate_tests {
    use super::{read_version_info, LocalFrame};
    use crate::metadata::{Color, ECLevel, Version};
    use crate::reader::binarize::BinaryImage;
    use crate::reader::utils::geometry::Point;
    use crate::{Module, QRBuilder};

    #[test]
    fn test_read_version_info() {
        let data = "Hello, world! 🌎";
        let ecl = ECLevel::L;
        let k = 3.0;

        for v in 7..=40u32 {
            let ver = Version::Normal(v as usize);
            let w = ver.width();

            let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
            let img = BinaryImage::prepare(&qr.to_image(k as u32));

            let q = 4.0; // quiet zone, modules
            let c = (3.5 + q) * k; // 3.5 modules in from the edge
            let far = (w as f64 - 3.5 + q) * k;
            let p = |x: f64, y: f64| Point { x: x.round() as i32, y: y.round() as i32 };

            // BL: centre, then m03 (+3 modules right) and m01 (3 modules up)
            let blf =
                LocalFrame::new(&p(c, far), &p(c + 3.0 * k, far), &p(c, far - 3.0 * k), 3.0, 3.0);
            let (blv, blerr) = read_version_info(&img, blf).expect("Version read failed");
            assert_eq!(blv, v);
            assert_eq!(blerr, 0);

            // TR: centre, then m23 (+3 modules down) and m21 (3 modules left)
            let trf =
                LocalFrame::new(&p(far, c), &p(far, c + 3.0 * k), &p(far - 3.0 * k, c), 3.0, 3.0);
            let (trv, trerr) = read_version_info(&img, trf).expect("Version read failed");
            assert_eq!(trv, v);
            assert_eq!(trerr, 0);
        }
    }

    #[test]
    fn test_read_version_info_partially_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;
        let w = ver.width();
        let k = 3.0;

        // Bottom left version bits coords to corrupt
        let blc = [(5, -9), (5, -10), (5, -11)];

        for err in 0..4 {
            let mut qr =
                QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();

            for (x, y) in blc.iter().take(err) {
                let clr = *qr.get(*x, *y);
                let nclr = if clr == Color::Black { Color::White } else { Color::Black };
                qr.set(*x, *y, Module::Format(nclr));
            }

            let img = BinaryImage::prepare(&qr.to_image(k as u32));

            let q = 4.0; // quiet zone, modules
            let c = (3.5 + q) * k; // 3.5 modules in from the edge
            let far = (w as f64 - 3.5 + q) * k;
            let p = |x: f64, y: f64| Point { x: x.round() as i32, y: y.round() as i32 };

            // BL: centre, then m03 (+3 modules right) and m01 (3 modules up)
            let blf =
                LocalFrame::new(&p(c, far), &p(c + 3.0 * k, far), &p(c, far - 3.0 * k), 3.0, 3.0);
            let (blv, blerr) = read_version_info(&img, blf).expect("Version read failed");
            assert_eq!(blv, *ver as u32);
            assert_eq!(blerr, err as u32);
        }
    }

    #[test]
    fn test_read_version_info_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;
        let w = ver.width();
        let k = 3.0;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();

        for (x, y) in [(5, -9), (5, -10), (5, -11), (4, -9)] {
            let clr = *qr.get(x, y);
            let nclr = if clr == Color::Black { Color::White } else { Color::Black };
            qr.set(x, y, Module::Format(nclr));
        }

        let img = BinaryImage::prepare(&qr.to_image(k as u32));

        let q = 4.0; // quiet zone, modules
        let c = (3.5 + q) * k; // 3.5 modules in from the edge
        let far = (w as f64 - 3.5 + q) * k;
        let p = |x: f64, y: f64| Point { x: x.round() as i32, y: y.round() as i32 };

        // BL: centre, then m03 (+3 modules right) and m01 (3 modules up)
        let blf = LocalFrame::new(&p(c, far), &p(c + 3.0 * k, far), &p(c, far - 3.0 * k), 3.0, 3.0);
        let blv = read_version_info(&img, blf);
        assert!(blv.is_none());
    }
}

#[cfg(test)]
mod symbol_locate_integration_tests {

    use crate::{
        reader::{
            binarize::BinaryImage,
            finder::{group_finders, locate_finders},
            locate_symbols,
            utils::geometry::Point,
        },
        ECLevel, MaskPattern, QRBuilder, Version,
    };

    #[test]
    fn test_locate_symbol_0() {
        let data = "Hello, world!🌎";
        let ver = Version::Normal(4);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);
        let hi_cap = false;

        let qr = QRBuilder::new(data.as_bytes())
            .version(ver)
            .ec_level(ecl)
            .high_capacity(hi_cap)
            .mask(mask)
            .build()
            .unwrap();

        let img = qr.to_image(10);
        let exp_anchors = [
            [Some(Point { x: 75, y: 75 }), Some(Point { x: 335, y: 75 })],
            [Some(Point { x: 75, y: 335 }), Some(Point { x: 305, y: 305 })],
        ];

        let mut img = BinaryImage::prepare(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);
        for (i, exp_row) in exp_anchors.iter().enumerate() {
            for (j, &exp_anc) in exp_row.iter().enumerate() {
                assert_eq!(symbols[0]._anchors[i][j], exp_anc)
            }
        }
    }
}

impl SymbolLocation {
    pub(super) fn tile_at(&self, x: usize, y: usize) -> QRResult<&Tile> {
        debug_assert!(
            x < self.ver.width() && y < self.ver.width(),
            "Module coord x: {x} or y: {y} is out of bound"
        );

        let tx = self.bands[x] as usize;
        let ty = self.bands[y] as usize;
        self.tiles[ty][tx].as_ref().ok_or(QRError::TileNotFound)
    }
}

// Symbol fitness
//------------------------------------------------------------------------------

impl SymbolLocation {
    fn symbol_fitness(&self, img: &BinaryImage) -> i32 {
        let mut score = 0;
        let grid_size = self.ver.width() as i32;

        // Score timing patterns
        for i in 7..grid_size - 7 {
            let flip = if i & 1 == 0 { -1 } else { 1 };
            score += self.cell_fitness(img, i, 6) * flip;
            score += self.cell_fitness(img, 6, i) * flip;
        }

        // Score finders
        score += self.finder_fitness(img, 0, 0);
        score += self.finder_fitness(img, grid_size - 7, 0);
        score += self.finder_fitness(img, 0, grid_size - 7);

        // Score alignment patterns
        for (x, y) in alignment_coords(self.ver) {
            score += self.alignment_fitness(img, x, y);
        }

        score
    }

    fn finder_fitness(&self, img: &BinaryImage, x: i32, y: i32) -> i32 {
        let (x, y) = (x + 3, y + 3);
        self.cell_fitness(img, x, y) + self.ring_fitness(img, x, y, 1)
            - self.ring_fitness(img, x, y, 2)
            + self.ring_fitness(img, x, y, 3)
    }

    fn alignment_fitness(&self, img: &BinaryImage, x: i32, y: i32) -> i32 {
        self.cell_fitness(img, x, y) - self.ring_fitness(img, x, y, 1)
            + self.ring_fitness(img, x, y, 2)
    }

    fn ring_fitness(&self, img: &BinaryImage, cx: i32, cy: i32, r: i32) -> i32 {
        let mut score = 0;

        for i in 0..r * 2 {
            score += self.cell_fitness(img, cx - r + i, cy - r);
            score += self.cell_fitness(img, cx - r, cy + r - i);
            score += self.cell_fitness(img, cx + r, cy - r + i);
            score += self.cell_fitness(img, cx + r - i, cy + r);
        }

        score
    }

    fn cell_fitness(&self, img: &BinaryImage, x: i32, y: i32) -> i32 {
        const OFFSETS: [f64; 3] = [0.3, 0.5, 0.7];
        let white = Color::White;
        let mut score = 0;
        let Ok(tile) = self.tile_at(x as usize, y as usize) else { return 0 };

        for dy in OFFSETS.iter() {
            for dx in OFFSETS.iter() {
                let pt = match tile.map(x as f64 + dx, y as f64 + dy) {
                    Ok(v) => v,
                    Err(_) => return 0,
                };
                if let Some(color) = img.get_at_point(&pt) {
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
}

// The score a flawless symbol would earn, i.e. the ceiling `symbol_fitness` is measured against.
// Every module the scorer visits -- finders, timing patterns, alignment patterns -- counts once.
fn max_fitness_score(ver: Version) -> i32 {
    let mut total_mods = 0;

    // Finder modules
    total_mods += 49 * 3;

    // Timing modules
    let grid_size = ver.width() as i32;
    total_mods += (grid_size - 14) * 2;

    // Alignment modules
    total_mods += 25 * alignment_coords(ver).count() as i32;

    total_mods * 9 // Each module has a maximum score of 9
}

#[cfg(test)]
mod fitness_tests {
    use super::{alignment_coords, max_fitness_score};
    use crate::metadata::Version;
    use std::collections::HashSet;

    // Alignment pattern counts per version from ISO/IEC 18004 Annex E: the alignment
    // coordinates form an n x n grid, minus the three cells taken by the finders.
    fn spec_alignment_count(v: usize) -> usize {
        let n = match v {
            1 => return 0,
            2..=6 => 2,
            7..=13 => 3,
            14..=20 => 4,
            21..=27 => 5,
            28..=34 => 6,
            35..=40 => 7,
            _ => unreachable!(),
        };
        n * n - 3
    }

    #[test]
    fn alignment_centres_matches_spec_count() {
        for v in 1..=40 {
            assert_eq!(
                alignment_coords(Version::Normal(v)).count(),
                spec_alignment_count(v),
                "version {v}: wrong number of alignment patterns"
            );
        }
    }

    #[test]
    fn alignment_centres_skips_finder_corners_and_has_no_duplicates() {
        for v in 2..=40 {
            let ver = Version::Normal(v);
            let aps = ver.alignment_pattern();
            let (first, last) = (aps[0], aps[aps.len() - 1]);
            let centres: Vec<_> = alignment_coords(ver).collect();
            let uniq: HashSet<_> = centres.iter().copied().collect();

            assert_eq!(uniq.len(), centres.len(), "version {v}: duplicate alignment centres");
            for corner in [(first, first), (first, last), (last, first)] {
                assert!(
                    !uniq.contains(&corner),
                    "version {v}: {corner:?} collides with a finder but was emitted"
                );
            }
        }
    }

    // Version 1 has no alignment patterns; the iterator must stay empty rather than panic.
    #[test]
    fn version_1_has_no_alignment_centres() {
        assert_eq!(alignment_coords(Version::Normal(1)).count(), 0);
        assert_eq!(max_fitness_score(Version::Normal(1)), (49 * 3 + (21 - 14) * 2) * 9);
    }

    #[test]
    fn max_fitness_score_accounts_for_every_scored_module() {
        for v in 1..=40 {
            let ver = Version::Normal(v);
            let grid_size = ver.width() as i32;
            let expected_mods = 49 * 3                                  // 3 finders
                + (grid_size - 14) * 2                                  // 2 timing patterns
                + 25 * spec_alignment_count(v) as i32; // alignment patterns
            assert_eq!(
                max_fitness_score(ver),
                expected_mods * 9,
                "version {v}: max_fitness_score disagrees with what symbol_fitness scores"
            );
        }
    }
}

// Global constants
//------------------------------------------------------------------------------

const SYMBOL_HEURISTIC_THRESHOLD: f64 = 0.5;
