use std::{cmp::Ordering, collections::HashSet};

use super::{
    binarize::BinaryImage,
    finder::FinderGroup,
    tile::Tile,
    utils::geometry::{Axis, BresenhamLine, Point, Slope, X, Y},
};
use crate::{
    ec::rectify_info,
    metadata::{Color, VERSION_ERROR_BIT_LEN, VERSION_ERROR_CAPACITY, VERSION_INFOS},
    utils::{QRError, QRResult},
    Version,
};

// Local frame
//
// An affine frame: an origin plus two basis vectors, each normalised to span exactly one
// module. It maps module coordinates, taken relative to the origin, onto image pixels.
//
// A frame is built from three image points -- the origin and one endpoint per axis -- together
// with the module span each endpoint sits at. The constructor divides through by those spans,
// so the stored basis is per-module however far apart the defining points were. The two axes
// carry independent spans and so need not reach equally far.
//
// The frame picks up the symbol's local rotation, scale and skew straight from the points it
// was built on, without needing the grid. Being affine it carries no perspective term and
// drifts the further it is extrapolated, which is what confines it to short reaches. That
// same independence from the grid is what makes it usable before the homography exists.
//
// Two frames are built today:
// 1. At a finder centre, with the midpoints of its black ring as endpoints -- 3 modules out on
//    each axis -- to read the version info block beside the BL & TR finders. The homography
//    needs the version, so the version has to be read first; hence a frame that predates it.
// 2. At the TL finder centre, with the TR & BL finder centres as endpoints -- width - 7
//    modules out on each axis -- to predict the provisional alignment centres.
//------------------------------------------------------------------------------

#[derive(Debug, Copy, Clone)]
struct LocalFrame {
    o: Point,      // Origin
    u: (f64, f64), // Basis vector along x axis
    v: (f64, f64), // Basis vector along y axis
}

impl LocalFrame {
    // `px` and `py` are the endpoints that define the +x and +y directions, sitting `spanx`
    // and `spany` modules out from the origin along their respective axes. The two
    // directions must not be parallel: a degenerate basis leaves `map` returning
    // meaningless points rather than failing.
    fn new(o: &Point, px: &Point, py: &Point, spanx: f64, spany: f64) -> Self {
        debug_assert!(spanx > 0.0 && spany > 0.0, "Spans cannot be zero");

        Self {
            o: *o,
            u: ((px.x - o.x) as f64 / spanx, (px.y - o.y) as f64 / spanx),
            v: ((py.x - o.x) as f64 / spany, (py.y - o.y) as f64 / spany),
        }
    }

    // Maps module coordinates, relative to the origin, onto image pixels. Offsets are counted
    // in modules and the basis is already per-module, so they scale it directly.
    fn map(&self, x: f64, y: f64) -> Point {
        Point {
            x: (self.o.x as f64 + x * self.u.0 + y * self.v.0).round() as i32,
            y: (self.o.y as f64 + x * self.u.1 + y * self.v.1).round() as i32,
        }
    }

    // Side of one module in pixels, averaged over the two axes. The mean of the two lengths, not
    // the root of their mean square: the two disagree once perspective foreshortens one axis, and
    // the root mean square runs high when they do.
    fn mod_size(&self) -> f64 {
        (self.u.0.hypot(self.u.1) + self.v.0.hypot(self.v.1)) / 2.0
    }

    // Area one module covers in pixels: the parallelogram the two basis vectors span.
    fn mod_area(&self) -> f64 {
        (self.u.0 * self.v.1 - self.u.1 * self.v.0).abs()
    }
}

// Locates symbol based on 3 finder centres, their edge points & provisional grid size
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SymbolLocation {
    pub(super) ver: Version,
    pub(super) tiles: [[Option<Tile>; 6]; 6],
    pub(super) bands: [u8; MAX_WIDTH], // module -> tile, both axes
    pub(super) _anchors: [[Option<Point>; 7]; 7],
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
        let mut align_centres: [[Option<Point>; 7]; 7] = [[None; 7]; 7];

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

// Alignment patterns
//------------------------------------------------------------------------------

// Fills `centres` with the image position of every alignment pattern in the symbol.
//
// The three cells that coincide with finders are taken straight from the finder centres. Every
// other cell is resolved in two steps: a local frame predicts where the pattern should sit, then
// `pinpoint_alignment_centre` spirals out from that prediction until it finds a black region that
// reads as the pattern's centre stone. The measured centre is what gets stored -- the prediction
// only ever seeds the search. A cell whose search comes up empty is left `None`, marking a centre
// the caller was not given rather than one that sits at the prediction. In case of v1, the bottom
// right point is extrapolated from local frame
//
// The frame is built on the three finder centres, so its basis spans `width - 7` modules rather
// than the 3 a single finder's own ring would give. Both scale gates come off that same frame --
// the module size, and the area a candidate stone is allowed to cover -- which makes them averages
// over the whole symbol rather than measurements taken at the cell being searched. On a strongly
// warped symbol the module footprint in the far corner will not match that average.
//
// The set of regions already tried is likewise shared by every cell, so each stone in the symbol
// can be claimed only once. A cell whose spiral reaches a stone another cell has already taken
// finds nothing and is left `None`.
fn locate_alignment_centres(
    img: &mut BinaryImage,
    finders: &[Point; 3],
    ver: Version,
    ff: &LocalFrame,
    centres: &mut [[Option<Point>; 7]; 7],
) {
    let aps = ver.alignment_pattern();
    let [c0, c1, c2] = finders;
    let span = (ver.width() - 7) as f64; // Modules between finders

    if aps.is_empty() {
        centres[1][0] = Some(*c0);
        centres[0][0] = Some(*c1);
        centres[0][1] = Some(*c2);
        centres[1][1] = Some(ff.map(span, span));
        return;
    }

    // Fill finder centres
    let n = aps.len();
    centres[n - 1][0] = Some(*c0);
    centres[0][0] = Some(*c1);
    centres[0][n - 1] = Some(*c2);

    let mod_size = ff.mod_size();
    let search_radius = mod_size.round() as i32 * (n as i32 + 14);
    let mod_area = ff.mod_area();
    let mut visited_regs: HashSet<usize> = HashSet::new();

    for r in 0..n {
        for c in 0..n {
            if centres[r][c].is_none() {
                let seed = provisional_alignment(r, c, ver, ff, centres);

                let exact_centre = pinpoint_alignment_centre(
                    img,
                    &mut visited_regs,
                    seed,
                    mod_size,
                    search_radius,
                    mod_area,
                );

                centres[r][c] = exact_centre;
            }
        }
    }
}

// Where the alignment pattern at cell (row, col) is expected to sit in the image. A seed for
// the search.
fn provisional_alignment(
    row: usize,
    col: usize,
    ver: Version,
    ff: &LocalFrame, // Finders frame
    centres: &[[Option<Point>; 7]; 7],
) -> Point {
    debug_assert!(centres[row][col].is_none(), "Cell ({row}, {col}) is already located");
    let n = ver.alignment_pattern().len();

    // 1st row & column alignments are skipped, because they are close enough to the finders
    // that we can use use finders frame to map them. The alignments which have finders as
    // one of their neighbors are also skipped because we cant apply parallelogram extrapolation
    // directly
    if row > 0
        && col > 0
        && ![(1, 1), (n - 2, 1), (n - 1, 1), (1, n - 2), (1, n - 1)].contains(&(row, col))
    {
        if let (Some(left), Some(top_left), Some(top)) =
            (centres[row][col - 1], centres[row - 1][col - 1], centres[row - 1][col])
        {
            return Point { x: left.x + top.x - top_left.x, y: left.y + top.y - top_left.y };
        }
    }

    let aps = ver.alignment_pattern();
    ff.map(aps[col] as f64 - 3.0, aps[row] as f64 - 3.0)
}

// Locates the centre of the alignment pattern nearest `seed`, or `None` if the spiral runs out
// to `search_radius` without finding one.
//
// The search walks a square spiral outward from `seed`. At each black pixel it flood-fills the
// region underneath and tests it as a candidate centre stone, by sweeping the white ring that
// encircles it -- see `verify_alignment_centre`.
//
// The upper bound on the stone's area is enforced by the fill rather than by a comparison:
// `get_region_capped` abandons a fill that grows past `max_area` and returns `None`, so a `None`
// there means the blob was too big to be a stone, not that anything went wrong.
//
// `visited_regs` holds the ids of regions already tried, whether they passed or failed. It is
// shared across every cell of the grid, so each stone can be claimed only once: a cell whose
// spiral reaches a stone that another cell has already taken passes over it and keeps searching.
fn pinpoint_alignment_centre(
    img: &mut BinaryImage,
    visited_regs: &mut HashSet<usize>,
    seed: Point,
    mod_size: f64,
    search_radius: i32,
    mod_area: f64,
) -> Option<Point> {
    // Directional increment for x & y: [right, down, left, up]
    const DX: [i32; 4] = [1, 0, -1, 0];
    const DY: [i32; 4] = [0, -1, 0, 1];

    let (w, h) = (img.w, img.h);
    let max_area = (mod_area * STONE_AREA_TOLERANCE).round() as u32;

    // Spiral outward to find stone
    let mut cursor = seed;
    let mut dir = 0;
    let mut run_len = 1;

    while run_len < search_radius {
        for _ in 0..run_len {
            // Drop a cursor that has spiralled off the image before looking it up
            let px = u32::try_from(cursor.x).ok().zip(u32::try_from(cursor.y).ok());

            if let Some((x, y)) = px.filter(|&(x, y)| x < w && y < h) {
                if img.get_at_point(&cursor) == Some(Color::Black) {
                    if let Some(stone) = img.get_region_capped((x, y), max_area) {
                        let (stone_id, stone_centre) = (stone.id, stone.centre);

                        if !visited_regs.contains(&stone_id) {
                            visited_regs.insert(stone_id);
                            if verify_alignment_centre(img, &stone_centre, mod_size, mod_area) {
                                return Some(stone_centre);
                            }
                        }
                    }
                }
            }

            cursor.x += DX[dir];
            cursor.y += DY[dir];
        }

        // Cycle direction
        dir = (dir + 1) & 3;
        if dir & 1 == 0 {
            run_len += 1;
        }
    }
    None
}

// Sweeps the white ring encircling a candidate centre stone and reports whether it reads as
// the middle band of an alignment pattern.
//
// The area is measured against the stone's own area rather than the symbol-wide module estimate,
// which keeps the gate local: the white ring covers 8 modules to the stone's 1 whatever the scale or
// warp is at this corner. And a closed white ring shares its centroid with what it encloses, so the
// two centres must very nearly agree.
fn verify_alignment_centre(
    img: &mut BinaryImage,
    stone_centre: &Point,
    mod_size: f64,
    mod_area: f64,
) -> bool {
    let mut step = 0;
    let max_steps = (mod_size * 2.0).round() as u32;
    let mut seed = *stone_centre;
    while img.get_at_point(&seed) == Some(Color::Black) {
        if step > max_steps {
            return false;
        }
        seed.x += 1;
        step += 1;
    }

    if img.get_at_point(&seed) != Some(Color::White) {
        return false;
    }

    debug_assert!(seed.x >= 0);

    let (x, y) = (seed.x as u32, seed.y as u32);
    let area_cap = (mod_area * RING_MODS * RING_AREA_TOLERANCE).round() as u32;
    let Some(ring) = img.get_region_capped((x, y), area_cap) else {
        return false;
    };

    let max_drift = mod_size * CENTRE_DRIFT_TOLERANCE;
    stone_centre.dist_sq(&ring.centre) as f64 <= max_drift * max_drift
}

fn infer_alignment_centres(ver: Version, ff: &LocalFrame, centres: &mut [[Option<Point>; 7]; 7]) {
    let aps = ver.alignment_pattern();
    let n = aps.len();
    for r in 0..n {
        for c in 0..n {
            if centres[r][c].is_some() {
                continue;
            }
            if let Some((xn, xsn, yn, ysn)) = nearest_pair(r, c, ver, centres) {
                centres[r][c] = Some(line_intersection(xn, xsn, yn, ysn));
            }
            if centres[r][c].is_none() {
                centres[r][c] = Some(ff.map(aps[c] as f64 - 3.0, aps[r] as f64 - 3.0));
            }
        }
    }
}

fn nearest_pair(
    row: usize,
    col: usize,
    ver: Version,
    centres: &[[Option<Point>; 7]; 7],
) -> Option<(Point, Point, Point, Point)> {
    let (sr, sc) = (row as i32, col as i32);
    let n = ver.alignment_pattern().len();

    debug_assert_ne!(n, 0);

    let finders = [(0, n - 1), (0, 0), (n - 1, 0)];
    let n = n as i32;

    // Zigzagging along row
    let mut nearest: Option<Point> = None;
    let mut second_nearest: Option<Point> = None;
    for i in 2..(2 * n) {
        let dir = ((i & 1) << 1) - 1;
        let step = i >> 1;
        let ncol = sc + dir * step;
        if !(0..n).contains(&ncol) {
            continue;
        };
        let ncol = ncol as usize;
        if centres[row][ncol].is_some() && !finders.contains(&(row, ncol)) {
            if nearest.is_none() {
                nearest = centres[row][ncol];
            } else if second_nearest.is_none() {
                second_nearest = centres[row][ncol];
            } else {
                break;
            }
        }
    }

    let xn = nearest?;
    let xsn = second_nearest?;

    // Zigzagging along column
    let mut nearest: Option<Point> = None;
    let mut second_nearest: Option<Point> = None;
    for i in 2..14i32 {
        let dir = ((i & 1) << 1) - 1;
        let step = i >> 1;
        let nrow = sr + dir * step;
        if !(0..n).contains(&nrow) {
            continue;
        };
        let nrow = nrow as usize;
        if centres[nrow][col].is_some() && !finders.contains(&(nrow, col)) {
            if nearest.is_none() {
                nearest = centres[nrow][col];
            } else if second_nearest.is_none() {
                second_nearest = centres[nrow][col];
            } else {
                break;
            }
        }
    }

    let yn = nearest?;
    let ysn = second_nearest?;

    Some((xn, xsn, yn, ysn))
}

// Intersection point of line p1 -> p2 and line p3 -> p4
fn line_intersection(p1: Point, p2: Point, p3: Point, p4: Point) -> Point {
    let (x1, y1) = (f64::from(p1.x), f64::from(p1.y));
    let (x2, y2) = (f64::from(p2.x), f64::from(p2.y));
    let (x3, y3) = (f64::from(p3.x), f64::from(p3.y));
    let (x4, y4) = (f64::from(p4.x), f64::from(p4.y));

    let dx1 = x2 - x1;
    let dy1 = y2 - y1;
    let dx2 = x4 - x3;
    let dy2 = y4 - y3;

    let denom = dx1 * dy2 - dy1 * dx2;

    // Parallel / collinear
    debug_assert!(denom.abs() > 1e-9);

    let t = ((x3 - x1) * dy2 - (y3 - y1) * dx2) / denom;

    let x = (x1 + t * dx1).round() as i32;
    let y = (y1 + t * dy1).round() as i32;

    Point { x, y }
}

#[cfg(test)]
mod alignment_pattern_tests {
    use super::locate_alignment_centres;
    use crate::metadata::Version;
    use crate::reader::binarize::BinaryImage;
    use crate::reader::locate::{infer_alignment_centres, provisional_alignment, LocalFrame};
    use crate::reader::utils::geometry::Point;
    use crate::{ECLevel, QRBuilder};

    #[test]
    fn test_locate_alignment_centres() {
        let data = "Hello, world! 🌎";
        let ecl = ECLevel::L;
        let k = 3.0; // Pixels per module
        let q = 4.0; // Quiet zone, modules

        // Centre of module `m` along either axis, in pixels
        let centre_px = |m: f64| (q + m + 0.5) * k;

        for v in 2..=40u32 {
            let ver = Version::Normal(v as usize);
            let w = ver.width();
            let ap_coords = ver.alignment_pattern();
            let n = ap_coords.len();

            let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
            let mut img = BinaryImage::prepare(&qr.to_image(k as u32));

            // Finder centres sit on module 3 and module w - 4
            let p = |x: f64, y: f64| Point { x: x.round() as i32, y: y.round() as i32 };
            let near = centre_px(3.0);
            let far = centre_px(w as f64 - 4.0);
            let finders = [p(near, far), p(near, near), p(far, near)]; // BL, TL, TR
            let span = (ver.width() - 7) as f64; // Modules between finders
            let ff = LocalFrame::new(&finders[1], &finders[2], &finders[0], span, span); // Finders frame

            let mut centres: [[Option<Point>; 7]; 7] = [[None; 7]; 7];

            locate_alignment_centres(&mut img, &finders, ver, &ff, &mut centres);

            assert_eq!(finders[0], centres[n - 1][0].unwrap(), "version {v}: BL finder centre");
            assert_eq!(finders[1], centres[0][0].unwrap(), "version {v}: TL finder centre");
            assert_eq!(finders[2], centres[0][n - 1].unwrap(), "version {v}: TR finder centre");

            for (row, &apy) in ap_coords.iter().enumerate() {
                let actualy = centre_px(apy as f64) as i32;
                for (col, &apx) in ap_coords.iter().enumerate() {
                    if [(0, 0), (0, n - 1), (n - 1, 0)].contains(&(row, col)) {
                        continue;
                    }
                    let actualx = centre_px(apx as f64) as i32;
                    let pred = centres[row][col].unwrap();

                    assert_eq!(actualx, pred.x);
                    assert_eq!(actualy, pred.y);
                }
            }
        }
    }

    #[test]
    fn test_alignment_centres_version_1() {
        let data = "Hello, world!";
        let ecl = ECLevel::L;
        let ver = Version::Normal(1);
        let k = 3.0; // Pixels per module

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let mut img = BinaryImage::prepare(&qr.to_image(k as u32));

        // Finder centres for a version 1 symbol at 3 px per module with a 4 module quiet zone
        let finders = [Point { x: 23, y: 65 }, Point { x: 23, y: 23 }, Point { x: 65, y: 23 }];
        let br = Point { x: 65, y: 65 };
        let span = (ver.width() - 7) as f64; // Modules between finders
        let ff = LocalFrame::new(&finders[1], &finders[2], &finders[0], span, span); // Finders frame

        let mut centres: [[Option<Point>; 7]; 7] = [[None; 7]; 7];

        locate_alignment_centres(&mut img, &finders, ver, &ff, &mut centres);

        assert_eq!(centres[1][0], Some(finders[0]));
        assert_eq!(centres[0][0], Some(finders[1]));
        assert_eq!(centres[0][1], Some(finders[2]));
        assert_eq!(centres[1][1], Some(br));
    }

    #[test]
    fn test_provisional_alignment_centre() {
        let q = 4.0;
        let k = 3.0;
        for v in 2..=40 {
            let ver = Version::Normal(v);
            let w = ver.width() as f64;
            let near = ((q + 3.5) * k) as i32;
            let far = ((q + w - 3.5) * k) as i32;
            let c0 = Point { x: near, y: far };
            let c1 = Point { x: near, y: near };
            let c2 = Point { x: far, y: near };
            let ff = LocalFrame::new(&c1, &c2, &c0, w - 7.0, w - 7.0);

            let aps = ver.alignment_pattern();
            let n = aps.len();

            let mut centres: [[Option<Point>; 7]; 7] = [[None; 7]; 7];
            centres[n - 1][0] = Some(c0);
            centres[0][0] = Some(c1);
            centres[0][n - 1] = Some(c2);

            for r in 0..n {
                for c in 0..n {
                    if centres[r][c].is_none() {
                        centres[r][c] = Some(provisional_alignment(r, c, ver, &ff, &centres));
                        let exp_centre = Some(Point {
                            x: ((q + aps[c] as f64 + 0.5) * k) as i32,
                            y: ((q + aps[r] as f64 + 0.5) * k) as i32,
                        });
                        assert_eq!(
                            centres[r][c], exp_centre,
                            "Version = {v}, Row = {r}, Column = {c}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_infer_alignment_centre() {
        let q = 4.0;
        let k = 3.0;
        for v in 2..=40 {
            let ver = Version::Normal(v);
            let w = ver.width() as f64;
            let near = ((q + 3.5) * k) as i32;
            let far = ((q + w - 3.5) * k) as i32;
            let c0 = Point { x: near, y: far };
            let c1 = Point { x: near, y: near };
            let c2 = Point { x: far, y: near };
            let ff = LocalFrame::new(&c1, &c2, &c0, w - 7.0, w - 7.0);

            let aps = ver.alignment_pattern();
            let n = aps.len();

            let mut centres: [[Option<Point>; 7]; 7] = [[None; 7]; 7];
            centres[n - 1][0] = Some(c0);
            centres[0][0] = Some(c1);
            centres[0][n - 1] = Some(c2);

            infer_alignment_centres(ver, &ff, &mut centres);

            for r in 0..n {
                for c in 0..n {
                    if [(0, 0), (n - 1, 0), (0, n - 1)].contains(&(r, c)) {
                        continue;
                    }
                    let exp_centre = Some(Point {
                        x: ((q + aps[c] as f64 + 0.5) * k) as i32,
                        y: ((q + aps[r] as f64 + 0.5) * k) as i32,
                    });
                    assert_eq!(centres[r][c], exp_centre, "Version = {v}, Row = {r}, Column = {c}");
                }
            }
        }
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

// Symbol tiles & band
//------------------------------------------------------------------------------

// Module space centre of the alignment pattern
// Note: the three cells coinciding with finders carry the finder centre
fn anchor_coord(ver: Version, row: usize, col: usize) -> (f64, f64) {
    let aps = ver.alignment_pattern();
    let last_cell = aps.len().max(2) - 1;
    let w = ver.width() as f64;

    if [(0, 0), (0, last_cell), (last_cell, 0)].contains(&(row, col)) {
        let x = if col == 0 { 3.5 } else { w - 3.5 };
        let y = if row == 0 { 3.5 } else { w - 3.5 };
        (x, y)
    } else if aps.is_empty() {
        // Handles v1
        (w - 3.5, w - 3.5)
    } else {
        (aps[col] as f64 + 0.5, aps[row] as f64 + 0.5)
    }
}

// Image space centres in the order: TL, TR, BR, BL -- or `None` if any of
// the four went unlocated. A tile is only as good as its worst corner.
fn tile_centres(centres: &[[Option<Point>; 7]; 7], row: usize, col: usize) -> Option<[Point; 4]> {
    Some([
        centres[row][col]?,
        centres[row][col + 1]?,
        centres[row + 1][col + 1]?,
        centres[row + 1][col]?,
    ])
}

// Splits the symbol into a grid of tiles, one per cell of the alignment grid, each carrying a
// homography fitted to the four located centres at its own corners.
//
// A tile spans from one alignment coordinate to the next, but the outermost tiles run out to the
// symbol edge rather than stopping at their anchor -- the top-left tile is anchored on a finder
// centre at module 3 yet owns every module from 0. A tile with an unlocated corner is left `None`.
//
// Versions carrying no alignment patterns yield no tiles.
fn build_tiles(ver: Version, centres: &[[Option<Point>; 7]; 7]) -> [[Option<Tile>; 6]; 6] {
    let mut tiles: [[Option<Tile>; 6]; 6] = Default::default();

    let aps = ver.alignment_pattern().iter().map(|&ap| ap as u32).collect::<Vec<_>>();

    let w = ver.width() as u32;
    let last_tile = aps.len().max(2) - 2; // Last tile of v1 is 0

    for row in 0..=last_tile {
        for col in 0..=last_tile {
            let Some(quad) = tile_centres(centres, row, col) else { continue };

            // Half-open module bounds. The outer edges reach the symbol boundary; the inner ones
            // stop on the next alignment coordinate, which the neighbouring
            let x0 = if col == 0 { 0 } else { aps[col] };
            let x1 = if col == last_tile { w } else { aps[col + 1] };
            let y0 = if row == 0 { 0 } else { aps[row] };
            let y1 = if row == last_tile { w } else { aps[row + 1] };

            let anchors = [
                anchor_coord(ver, row, col),
                anchor_coord(ver, row, col + 1),
                anchor_coord(ver, row + 1, col + 1),
                anchor_coord(ver, row + 1, col),
            ];

            tiles[row][col] = Tile::new((x0, y0, x1, y1), anchors, quad).ok();
        }
    }

    tiles
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

#[cfg(test)]
mod tile_tests {
    use super::build_tiles;
    use crate::metadata::Version;
    use crate::reader::utils::geometry::Point;

    const KX: f64 = 12.0; // Pixels per module, x
    const KY: f64 = 16.0; // Pixels per module, y
    const Q: f64 = 4.0; // Quiet zone, modules

    // Ground-truth placement of the symbol in an image, the same way this file's other tests do it.
    //
    // The two axes scale differently on purpose: at equal scales a row/column transposition inside
    // `build_tiles` maps onto itself and becomes invisible. Both scales are even, so a module
    // *centre* -- carrying its half-module offset -- still lands on a whole pixel. That keeps the
    // fitted homography exact and lets the reprojection check below run at zero tolerance.
    fn project(x: f64, y: f64) -> (f64, f64) {
        ((x + Q) * KX, (y + Q) * KY)
    }

    // Module-space centre of the pattern at grid cell (row, col), restated from the spec rather
    // than taken from `anchor_coord` -- a fixture that calls the code under test cannot catch it
    // being wrong. Finder cells sit on module 3 or w - 4; every other cell on its alignment
    // coordinate.
    fn cell_centre(ver: Version, row: usize, col: usize) -> (f64, f64) {
        let aps = ver.alignment_pattern();
        let last = aps.len().max(2) - 1;
        let w = ver.width() as f64;

        if [(0, 0), (0, last), (last, 0)].contains(&(row, col)) {
            let x = if col == 0 { 3.5 } else { w - 3.5 };
            let y = if row == 0 { 3.5 } else { w - 3.5 };
            (x, y)
        } else if aps.is_empty() {
            (w - 3.5, w - 3.5)
        } else {
            (aps[col] as f64 + 0.5, aps[row] as f64 + 0.5)
        }
    }

    // A fully located grid: every alignment centre present, placed by `project`.
    fn synthetic_centres(ver: Version) -> [[Option<Point>; 7]; 7] {
        let n = ver.alignment_pattern().len().max(2);
        let mut centres: [[Option<Point>; 7]; 7] = [[None; 7]; 7];

        for (row, centres_row) in centres.iter_mut().enumerate().take(n) {
            for (col, centre) in centres_row.iter_mut().enumerate().take(n) {
                let (mx, my) = cell_centre(ver, row, col);
                let (px, py) = project(mx, my);
                *centre = Some(Point { x: px as i32, y: py as i32 });
            }
        }
        centres
    }

    // Every cell of the alignment grid must yield a tile when all four of its corners are
    // located, and no cell outside that block may.
    #[test]
    fn build_tiles_fills_the_alignment_grid() {
        for v in 1..=40usize {
            let ver = Version::Normal(v);
            let n = ver.alignment_pattern().len().max(2);
            let expected = (n - 1) * (n - 1);

            let tiles = build_tiles(ver, &synthetic_centres(ver));

            let built = tiles.iter().flatten().filter(|t| t.is_some()).count();
            assert_eq!(built, expected, "version {v}: wrong number of tiles built");

            // The right count is not enough -- they have to be the right cells.
            for (row, tile_row) in tiles.iter().enumerate() {
                for (col, tile) in tile_row.iter().enumerate() {
                    let want = row < n - 1 && col < n - 1;
                    assert_eq!(
                        tile.is_some(),
                        want,
                        "version {v}: tile ({row}, {col}) is {}, expected {}",
                        if tile.is_some() { "built" } else { "absent" },
                        if want { "built" } else { "absent" },
                    );
                }
            }
        }
    }

    // Every module of the symbol must be owned by exactly one tile, and by the specific tile whose
    // band it falls in
    #[test]
    fn tile_bounds_partition_the_symbol() {
        for v in 1..=40usize {
            let ver = Version::Normal(v);
            let aps = ver.alignment_pattern();
            let n = aps.len().max(2);
            let w = ver.width() as u32;

            let tiles = build_tiles(ver, &synthetic_centres(ver));

            // Tile boundaries along either axis: the symbol edge, the interior alignment
            // coordinates, then the far edge. n boundaries enclose the n - 1 tiles on that axis.
            let mut edges = Vec::with_capacity(n);
            edges.push(0);
            edges.extend(aps.get(1..n - 1).unwrap_or(&[]).iter().map(|&ap| ap as u32));
            edges.push(w);

            // Band index of each module along one axis, precomputed once and shared by both.
            let bands: Vec<usize> = (0..w)
                .map(|m| {
                    edges.windows(2).position(|e| e[0] <= m && m < e[1]).unwrap_or_else(|| {
                        panic!("version {v}: module {m} falls outside every band")
                    })
                })
                .collect();

            for y in 0..w {
                let row = bands[y as usize];
                for x in 0..w {
                    let col = bands[x as usize];
                    let mut owners = 0;

                    for (r, tile_row) in tiles.iter().enumerate() {
                        for (c, tile) in tile_row.iter().enumerate() {
                            if tile.as_ref().is_some_and(|t| t.contains(x, y)) {
                                owners += 1;
                                assert_eq!(
                                    (r, c),
                                    (row, col),
                                    "version {v}: module ({x}, {y}) claimed by tile ({r}, {c})"
                                );
                            }
                        }
                    }

                    assert_eq!(
                        owners, 1,
                        "version {v}: module ({x}, {y}) owned by {owners} tiles, expected 1"
                    );
                }
            }
        }
    }

    // Every tile must reproduce the projection of every module it owns -- not merely at those corners.
    #[test]
    fn tile_homographies_reproduce_the_projection() {
        for v in 1..=40usize {
            let ver = Version::Normal(v);
            let w = ver.width() as u32;
            let tiles = build_tiles(ver, &synthetic_centres(ver));

            for y in 0..w {
                for x in 0..w {
                    let Some(tile) = tiles.iter().flatten().flatten().find(|t| t.contains(x, y))
                    else {
                        panic!("version {v}: module ({x}, {y}) is owned by no tile");
                    };

                    // Module centres -- the coordinates an actual lookup passes in.
                    let (mx, my) = (x as f64 + 0.5, y as f64 + 0.5);
                    let (ex, ey) = project(mx, my);
                    let got = tile.map(mx, my).expect("tile projection failed");

                    assert_eq!(
                        (got.x, got.y),
                        (ex as i32, ey as i32),
                        "version {v}: module ({x}, {y}) projects to ({}, {}) but should sit at \
                         ({ex}, {ey})",
                        got.x,
                        got.y
                    );
                }
            }
        }
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
        for (x, y) in alignment_centres(self.ver) {
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

// Centres of every alignment pattern in the symbol, in module coordinates. The alignment
// coordinates form a grid, minus the three corners occupied by the finders. Version 1 has
// no alignment coordinates, so this yields nothing.
fn alignment_centres(ver: Version) -> impl Iterator<Item = (i32, i32)> {
    let aps = ver.alignment_pattern();
    let len = aps.len();
    let last = len.saturating_sub(1);

    (0..len)
        .flat_map(move |i| (0..len).map(move |j| (i, j)))
        .filter(move |ij| ![(0, 0), (0, last), (last, 0)].contains(ij))
        .map(move |(i, j)| (aps[i], aps[j]))
}

fn max_fitness_score(ver: Version) -> i32 {
    let mut total_mods = 0;

    // Finder modules
    total_mods += 49 * 3;

    // Timing modules
    let grid_size = ver.width() as i32;
    total_mods += (grid_size - 14) * 2;

    // Alignment modules
    total_mods += 25 * alignment_centres(ver).count() as i32;

    total_mods * 9 // Each module has a maximum score of 9
}

#[cfg(test)]
mod fitness_tests {
    use super::{alignment_centres, max_fitness_score};
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
                alignment_centres(Version::Normal(v)).count(),
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
            let centres: Vec<_> = alignment_centres(ver).collect();
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
        assert_eq!(alignment_centres(Version::Normal(1)).count(), 0);
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

pub const MAX_WIDTH: usize = 177;

const STONE_AREA_TOLERANCE: f64 = 2.0;

const RING_MODS: f64 = 8.0;

const RING_AREA_TOLERANCE: f64 = 2.0;

const CENTRE_DRIFT_TOLERANCE: f64 = 0.5;
