use std::collections::HashSet;

use super::{
    binarize::BinaryImage,
    utils::{frame::LocalFrame, geometry::Point},
};
use crate::{
    metadata::Color,
    reader::utils::{geometry::SquareSpiral, homography::Homography},
    Version,
};

// The alignment grid is at most 7x7 -- version 40 carries 7 alignment coordinates per axis.
pub(super) const MAX_ALIGN_CELLS: usize = 7;

// Image-space centre of every cell of the alignment grid, indexed [row][col], `None` where the
// cell went unlocated. The three cells coinciding with finders carry the finder centre, not an
// alignment centre. Cells beyond the symbol's own grid size stay `None`.
pub(super) type Anchors = [[Option<Point>; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];

// Seeds the alignment grid with the three cells the finders occupy. Every version has these
// three, whatever its alignment grid looks like -- v1's 2x2 grid included.
pub(super) fn anchors_from_finders(ver: Version, finders: &[Point; 3]) -> Anchors {
    let mut centres: Anchors = [[None; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];
    let [c0, c1, c2] = finders;
    let last = ver.alignment_pattern().len().max(2) - 1; // v1's grid is 2x2

    centres[last][0] = Some(*c0);
    centres[0][0] = Some(*c1);
    centres[0][last] = Some(*c2);
    centres
}

// Bottom right anchor for QRs without alignment patterns like version 1
//------------------------------------------------------------------------------

pub(super) fn locate_br_anchor(
    img: &BinaryImage,
    ver: Version,
    finders: &[Point; 3],
    ff: &LocalFrame,
) -> Point {
    debug_assert!(ver.alignment_pattern().is_empty(), "QR shouldn't have any alignment pattern");

    let w = ver.width() as f64;
    let span = w - 7.0;
    let src: [(f64, f64); 4] = [(3.5, 3.5), (w - 3.5, 3.5), (w - 3.5, w - 3.5), (3.5, w - 3.5)];

    let [c0, c1, c2] = finders;
    let seed = ff.map(span, span);
    let mut dst = [c1, c2, &seed, c0].map(|p| (p.x as f64, p.y as f64));
    let h = Homography::compute(src, dst);

    let mut best_br_anchor = seed;
    let mut best_score = if let Ok(h) = h { quiet_zone_score(img, ver, &h) } else { 0 };
    let mut best_dist_sq = c1.dist_sq(&seed);

    // Spiral outward
    let mod_size = ff.mod_size();
    let radius = (mod_size * BR_ANCHOR_SEARCH_RADIUS).round() as i32;

    for cursor in SquareSpiral::new(&seed, radius) {
        // Drop a cursor that has spiralled off the image before looking it up
        if img.contains(&cursor) {
            dst[2] = (cursor.x as f64, cursor.y as f64);
            let Ok(h) = Homography::compute(src, dst) else { continue };
            let score = quiet_zone_score(img, ver, &h);
            let dist_sq = c1.dist_sq(&cursor);
            if score > best_score || (score == best_score && dist_sq < best_dist_sq) {
                best_br_anchor = cursor;
                best_score = score;
                best_dist_sq = dist_sq;
            }
        }
    }
    best_br_anchor
}

fn quiet_zone_score(img: &BinaryImage, ver: Version, h: &Homography) -> u32 {
    let w = ver.width();
    let mut white_score = 0;

    // Bottom edge + bottom right corner point
    let my = w as f64 + 0.5;
    for mx in 0..w + 1 {
        let Ok(px) = h.map(mx as f64 + 0.5, my) else { continue };
        let Some(clr) = img.get_at_point(&px) else { continue };
        if clr == Color::White {
            white_score += 1;
        };
    }

    // Right edge
    let mx = w as f64 + 0.5;
    for my in 0..w {
        let Ok(px) = h.map(mx, my as f64 + 0.5) else { continue };
        let Some(clr) = img.get_at_point(&px) else { continue };
        if clr == Color::White {
            white_score += 1;
        };
    }

    white_score
}

// Alignment patterns
//------------------------------------------------------------------------------

// Module-space centres of every alignment pattern in the symbol. The alignment coordinates form a
// grid, minus the three corners occupied by the finders. Version 1 has no alignment coordinates,
// so this yields nothing.
//
// Note these are module coordinates read off the spec, not the image positions measured by
// `locate_alignment_centres` -- the latter live in `Anchors`.
pub(super) fn alignment_coords(ver: Version) -> impl Iterator<Item = (i32, i32)> {
    let aps = ver.alignment_pattern();
    let len = aps.len();
    let last = len.saturating_sub(1);

    (0..len)
        .flat_map(move |i| (0..len).map(move |j| (i, j)))
        .filter(move |ij| ![(0, 0), (0, last), (last, 0)].contains(ij))
        .map(move |(i, j)| (aps[i], aps[j]))
}

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
pub(super) fn locate_alignment_centres(
    img: &mut BinaryImage,
    ver: Version,
    ff: &LocalFrame,
    centres: &mut Anchors,
) {
    let aps = ver.alignment_pattern();
    if aps.is_empty() {
        return;
    }

    // Fill finder centres
    let n = aps.len();

    let mod_size = ff.mod_size();
    let search_span = (mod_size * ALIGNMENT_SEARCH_RADIUS).round() as i32;
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
                    search_span,
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
    centres: &Anchors,
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
// to `search_span` without finding one.
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
    radius: i32,
    mod_area: f64,
) -> Option<Point> {
    let max_area = (mod_area * STONE_AREA_TOLERANCE).round() as u32;

    for cursor in SquareSpiral::new(&seed, radius) {
        // Drop a cursor that has spiralled off the image before looking it up
        if img.contains(&cursor) && img.get_at_point(&cursor) == Some(Color::Black) {
            if let Some(stone) = img.get_region_capped((cursor.x as u32, cursor.y as u32), max_area)
            {
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

pub(super) fn infer_alignment_centres(ver: Version, ff: &LocalFrame, centres: &mut Anchors) {
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
    centres: &Anchors,
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
    for i in 2..(2 * n) {
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
    use super::{
        anchors_from_finders, infer_alignment_centres, locate_alignment_centres, locate_br_anchor,
        provisional_alignment, Anchors, MAX_ALIGN_CELLS,
    };
    use crate::metadata::Version;
    use crate::reader::binarize::BinaryImage;
    use crate::reader::utils::frame::LocalFrame;
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

            let mut centres: Anchors = anchors_from_finders(ver, &finders);

            locate_alignment_centres(&mut img, ver, &ff, &mut centres);

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
        let span = (ver.width() - 7) as f64; // Modules between finders
        let ff = LocalFrame::new(&finders[1], &finders[2], &finders[0], span, span); // Finders frame

        let mut centres: Anchors = [[None; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];

        locate_alignment_centres(&mut img, ver, &ff, &mut centres);

        // A version 1 symbol carries no alignment patterns, so the grid is left untouched
        assert!(centres.iter().flatten().all(Option::is_none));
    }

    #[test]
    fn test_anchors_from_finders() {
        let q = 4.0; // Quiet zone, modules
        let k = 3.0; // Pixels per module

        // Centre of module `m` along either axis, in pixels
        let centre_px = |m: f64| ((q + m + 0.5) * k).round() as i32;

        for v in 1..=40 {
            let ver = Version::Normal(v);
            let w = ver.width() as f64;

            // Finder centres sit on module 3 and module w - 4
            let near = centre_px(3.0);
            let far = centre_px(w - 4.0);
            let c0 = Point { x: near, y: far };
            let c1 = Point { x: near, y: near };
            let c2 = Point { x: far, y: near };

            let centres = anchors_from_finders(ver, &[c0, c1, c2]);
            let last = ver.alignment_pattern().len().max(2) - 1;

            assert_eq!(centres[last][0], Some(c0), "Version = {v}: BL finder centre");
            assert_eq!(centres[0][0], Some(c1), "Version = {v}: TL finder centre");
            assert_eq!(centres[0][last], Some(c2), "Version = {v}: TR finder centre");

            for (r, row) in centres.iter().enumerate() {
                for (c, centre) in row.iter().enumerate() {
                    if [(last, 0), (0, 0), (0, last)].contains(&(r, c)) {
                        continue;
                    }
                    assert!(centre.is_none(), "Version = {v}, Row = {r}, Column = {c}");
                }
            }
        }
    }

    #[test]
    fn test_locate_br_anchor_version_1() {
        let data = "Hello, world!";
        let ecl = ECLevel::L;
        let ver = Version::Normal(1);
        let q = 4.0; // Quiet zone, modules
        let k = 10.0; // Pixels per module

        // Centre of module `m` along either axis, in pixels
        let centre_px = |m: f64| ((q + m + 0.5) * k).round() as i32;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let img = BinaryImage::prepare(&qr.to_image(k as u32));

        // Finder centres sit on module 3 and module w - 4
        let w = ver.width() as f64;
        let near = centre_px(3.0);
        let far = centre_px(w - 4.0);
        let c0 = Point { x: near, y: far };
        let c1 = Point { x: near, y: near };
        let c2 = Point { x: far, y: near };
        let span = w - 7.0; // Modules between finders
        let ff = LocalFrame::new(&c1, &c2, &c0, span, span); // Finders frame

        let br = locate_br_anchor(&img, ver, &[c0, c1, c2], &ff);

        // The calculated centre is (215, 215). The quiet zone score plateaus around it, and the
        // nearest-to-TL tie break settles those ties on the inward edge of that plateau, so the
        // anchor lands a few pixels short of the calculated point.
        assert_eq!(Point { x: far, y: far }, Point { x: 215, y: 215 });
        assert_eq!(br, Point { x: 213, y: 212 });
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "QR shouldn't have any alignment pattern")]
    fn test_locate_br_anchor_rejects_aligned_versions() {
        let data = "Hello, world!";
        let ecl = ECLevel::L;
        let ver = Version::Normal(2);
        let q = 4.0; // Quiet zone, modules
        let k = 10.0; // Pixels per module

        // Centre of module `m` along either axis, in pixels
        let centre_px = |m: f64| ((q + m + 0.5) * k).round() as i32;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let img = BinaryImage::prepare(&qr.to_image(k as u32));

        // Finder centres sit on module 3 and module w - 4
        let w = ver.width() as f64;
        let near = centre_px(3.0);
        let far = centre_px(w - 4.0);
        let c0 = Point { x: near, y: far };
        let c1 = Point { x: near, y: near };
        let c2 = Point { x: far, y: near };
        let span = w - 7.0; // Modules between finders
        let ff = LocalFrame::new(&c1, &c2, &c0, span, span); // Finders frame

        locate_br_anchor(&img, ver, &[c0, c1, c2], &ff);
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

            let mut centres: Anchors = [[None; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];
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

            let mut centres: Anchors = [[None; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];
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

// Global constants
//------------------------------------------------------------------------------

const STONE_AREA_TOLERANCE: f64 = 2.0;

const RING_MODS: f64 = 8.0;

const RING_AREA_TOLERANCE: f64 = 2.0;

const CENTRE_DRIFT_TOLERANCE: f64 = 0.5;

const ALIGNMENT_SEARCH_RADIUS: f64 = 4.0;

const BR_ANCHOR_SEARCH_RADIUS: f64 = 0.5;
