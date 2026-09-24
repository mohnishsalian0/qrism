use super::{
    binarize::BinaryImage,
    utils::{
        frame::LocalFrame,
        geometry::{Point, PointF},
    },
};
use crate::{
    reader::utils::{geometry::SquareSpiralLeg, homography::Homography},
    Version,
};

// The alignment grid is at most 7x7 -- version 40 carries 7 alignment coordinates per axis.
pub(super) const MAX_ALIGN_CELLS: usize = 7;

// Image-space centre of every cell of the alignment grid, indexed [row][col], `None` where the
// cell went unlocated. The three cells coinciding with finders carry the finder centre, not an
// alignment centre. Cells beyond the symbol's own grid size stay `None`.
pub(super) type Anchors = [[Option<PointF>; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];

// Seeds the alignment grid with the three cells the finders occupy. Every version has these
// three, whatever its alignment grid looks like -- v1's 2x2 grid included.
pub(super) fn anchors_from_finders(ver: Version, finders: &[PointF; 3]) -> Anchors {
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
    finders: &[PointF; 3],
    ff: &LocalFrame,
) -> PointF {
    debug_assert!(ver.alignment_pattern().is_empty(), "QR shouldn't have any alignment pattern");

    let w = ver.width() as f64;
    let span = w - 7.0;
    let src: [(f64, f64); 4] = [(3.5, 3.5), (w - 3.5, 3.5), (w - 3.5, w - 3.5), (3.5, w - 3.5)];

    let [c0, c1, c2] = finders;
    let seed = ff.exact_map(span, span);
    let mut dst = [c1, c2, &seed, c0].map(|p| (p.x, p.y));
    let h = Homography::compute(src, dst);

    let mut best_br_anchor = (seed.x, seed.y);
    let mut best_score = if let Ok(h) = h { quiet_zone_score(img, ver, &h) } else { 0 };
    let mut best_dist_sq = (c1.x - seed.x).powi(2) + (c1.y - seed.y).powi(2);

    let mod_size = ff.mod_size();
    let reach = (mod_size * BR_ANCHOR_SEARCH_RADIUS).round() as i32;

    let seedr = (seed.x.round() as i32, seed.y.round() as i32);
    let (l, r) = (seedr.0 - reach, seedr.0 + reach);
    let (t, b) = (seedr.1 - reach, seedr.1 + reach);
    for cx in l..=r {
        for cy in t..=b {
            if img.contains(cx, cy) {
                let (cx, cy) = (cx as f64, cy as f64);
                dst[2] = (cx, cy);
                let Ok(h) = Homography::compute(src, dst) else { continue };
                let score = quiet_zone_score(img, ver, &h);
                let dist_sq = ((c1.x - cx).powi(2) + (c1.y - cy).powi(2)) as f64;
                if score > best_score || (score == best_score && dist_sq < best_dist_sq) {
                    best_br_anchor = dst[2];
                    best_score = score;
                    best_dist_sq = dist_sq;
                }
            }
        }
    }
    PointF { x: best_br_anchor.0, y: best_br_anchor.1 }
}

fn quiet_zone_score(img: &BinaryImage, ver: Version, h: &Homography) -> u32 {
    let w = ver.width();
    let mut white_score = 0u32;

    // Bottom edge + bottom right corner point
    let my = w as f64 + 0.5;
    for mx in 0..w + 1 {
        let Ok(px) = h.map(mx as f64 + 0.5, my) else { continue };
        let Some(bit) = img.get_bit_at_point(&px) else { continue };
        white_score += bit as u32;
    }

    // Right edge
    let mx = w as f64 + 0.5;
    for my in 0..w {
        let Ok(px) = h.map(mx, my as f64 + 0.5) else { continue };
        let Some(bit) = img.get_bit_at_point(&px) else { continue };
        white_score += bit as u32;
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
// Each stone in the symbol can likewise be claimed only once, so a cell whose spiral reaches a
// stone another cell has already taken passes over it, and is left `None` if it finds nothing
// else. A claim is recorded on the contour itself, stamped with the pass number `next_pass` hands
// out here. Contours outlive a single symbol -- a second symbol in the same image sees every
// outline traced for the first -- and a stamp only ever matches the call that wrote it, so claims
// made for an earlier symbol read as stale and leave those stones free to be claimed again.
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
    let pass = img.next_pass();

    for r in 0..n {
        for c in 0..n {
            if centres[r][c].is_none() {
                let seed = provisional_alignment(r, c, ver, ff, centres);

                let exact_centre = pinpoint_alignment_centre(
                    img,
                    &Point::from(&seed),
                    mod_size,
                    search_span,
                    pass,
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
) -> PointF {
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
            return PointF { x: left.x + top.x - top_left.x, y: left.y + top.y - top_left.y };
        }
    }

    let aps = ver.alignment_pattern();
    ff.exact_map(aps[col] as f64 - 3.0, aps[row] as f64 - 3.0)
}

// Locates the centre of the alignment pattern nearest `seed`, or `None` if the spiral runs out
// to `search_span` without finding one.
//
// The search walks a square spiral outward from `seed`. At each black pixel it traces contour of
// the region within and tests it as a candidate centre stone, by tracing the white ring that
// encircles it -- see `verify_alignment_centre`.
//
// A stone is claimed by stamping its contour with `pass`, whether it went on to verify or not, and
// a stone already carrying this pass is passed over. `pass` is the same for every cell of the grid,
// so each stone can be claimed only once: a cell whose spiral reaches a stone that another cell has
// already taken keeps searching. Stamps left by an earlier symbol carry a different pass and never
// match -- see `locate_alignment_centres`.
fn pinpoint_alignment_centre(
    img: &mut BinaryImage,
    seed: &Point,
    mod_size: f64,
    radius: i32,
    pass: u32,
) -> Option<PointF> {
    let max_width = (mod_size * ALIGNMENT_TRACE_SLACK).round() as u32;
    let (mut cx, mut cy) = (seed.x, seed.y);
    let ssl = SquareSpiralLeg::new(radius);

    for (leg, dx, dy) in ssl {
        for _ in 0..leg {
            cx += dx;
            cy += dy;
            // Drop a cursor that has spiralled off the image before looking it up
            if img.contains(cx, cy) {
                let (x, y) = (cx as u32, cy as u32);
                if !img.get_bit_unbounded(x, y)
                    && (x + 1 == img.w || img.get_bit_unbounded(x + 1, y))
                {
                    if let Some(stone) = img.get_contour_capped((x, y), (x, y), max_width) {
                        if stone.visited_in != pass {
                            stone.visited_in = pass;
                            let Some(stone_centre) = stone.centre() else {
                                continue;
                            };
                            if verify_alignment_centre(img, &stone_centre, mod_size) {
                                return Some(stone_centre);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

// Sweeps the white ring encircling a candidate centre stone and reports whether it reads as
// the middle band of an alignment pattern. And a closed white ring shares its centroid with
// what it encloses, so the two centres must very nearly agree.

fn verify_alignment_centre(img: &mut BinaryImage, stone_centre: &PointF, mod_size: f64) -> bool {
    let sc = Point::from(stone_centre);

    debug_assert!(img.contains(sc.x, sc.y));

    let w = img.w;
    let mut step = 0;
    let max_steps = (mod_size * 3.0).round() as u32;
    let (mut x, y) = (sc.x as u32, sc.y as u32);
    let mut prev = img.get_bit_unbounded(x, y);
    if prev {
        return false;
    }
    let mut flips = 0;
    while step <= max_steps && flips < 2 {
        x += 1;
        step += 1;
        if x == w {
            return false;
        }

        let cur = img.get_bit_unbounded(x, y);
        if prev != cur {
            flips += 1;
        }
        prev = cur;
    }

    if flips < 2 {
        return false;
    }

    x -= 1;
    if img.get_bit(x, y) != Some(true) {
        return false;
    }

    let max_width = (mod_size * 3.0 * ALIGNMENT_TRACE_SLACK).round() as u32;
    let Some(ring) = img.get_contour_capped((x, y), (sc.x as u32, sc.y as u32), max_width) else {
        return false;
    };

    if !ring.contains(&sc) {
        return false;
    }

    // Concentricity test. The ring and stone centre should be reasonably near each other
    let max_drift = mod_size * ALIGNMENT_CENTRE_DRIFT_TOLERANCE;
    stone_centre.dist_sq(&ring.centre().unwrap()) <= max_drift * max_drift
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
                centres[r][c] = line_intersection(xn, xsn, yn, ysn);
            }
            if centres[r][c].is_none() {
                centres[r][c] = Some(ff.exact_map(aps[c] as f64 - 3.0, aps[r] as f64 - 3.0));
            }
        }
    }
}

fn nearest_pair(
    row: usize,
    col: usize,
    ver: Version,
    centres: &Anchors,
) -> Option<(PointF, PointF, PointF, PointF)> {
    let (sr, sc) = (row as i32, col as i32);
    let n = ver.alignment_pattern().len();

    debug_assert_ne!(n, 0);

    let finders = [(0, n - 1), (0, 0), (n - 1, 0)];
    let n = n as i32;

    // Zigzagging along row
    let mut nearest: Option<PointF> = None;
    let mut second_nearest: Option<PointF> = None;
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
    let mut nearest: Option<PointF> = None;
    let mut second_nearest: Option<PointF> = None;
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
fn line_intersection(p1: PointF, p2: PointF, p3: PointF, p4: PointF) -> Option<PointF> {
    let (x1, y1) = (p1.x, p1.y);
    let (x2, y2) = (p2.x, p2.y);
    let (x3, y3) = (p3.x, p3.y);
    let (x4, y4) = (p4.x, p4.y);

    let dx1 = x2 - x1;
    let dy1 = y2 - y1;
    let dx2 = x4 - x3;
    let dy2 = y4 - y3;

    let denom = dx1 * dy2 - dy1 * dx2;

    // Parallel / collinear
    if denom.abs() < 1e-9 {
        return None;
    }

    let t = ((x3 - x1) * dy2 - (y3 - y1) * dx2) / denom;

    let x = x1 + t * dx1;
    let y = y1 + t * dy1;

    Some(PointF { x, y })
}

#[cfg(test)]
mod alignment_pattern_tests {
    use super::{
        anchors_from_finders, infer_alignment_centres, line_intersection, locate_alignment_centres,
        locate_br_anchor, nearest_pair, provisional_alignment, Anchors, MAX_ALIGN_CELLS,
    };
    use crate::metadata::Version;
    use crate::reader::binarize::BinaryImage;
    use crate::reader::utils::frame::LocalFrame;
    use crate::reader::utils::geometry::PointF;
    use crate::{ECLevel, QRBuilder};

    fn p(x: f64, y: f64) -> PointF {
        PointF { x, y }
    }

    #[test]
    fn test_locate_alignment_centres() {
        let data = "Hello, world! 🌎";
        let ecl = ECLevel::L;
        let k = 3.0; // Pixels per module
        let q = 4.0; // Quiet zone, modules

        // Centre of module `m` along either axis, in pixel indices -- the middle pixel of the
        // module, which is where a contour centroid lands
        let centre_px = |m: f64| (q + m + 0.5) * k - 0.5;

        for v in 2..=40u32 {
            let ver = Version::Normal(v as usize);
            let w = ver.width();
            let ap_coords = ver.alignment_pattern();
            let n = ap_coords.len();

            let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
            let mut img = BinaryImage::prepare(&qr.to_gray_image(k as u32));

            // Finder centres sit on module 3 and module w - 4
            let p = |x: f64, y: f64| PointF { x, y };
            let near = centre_px(3.0);
            let far = centre_px(w as f64 - 4.0);
            let finders = [p(near, far), p(near, near), p(far, near)]; // BL, TL, TR
            let span = (ver.width() - 7) as f64; // Modules between finders
            let ff = LocalFrame::new(&finders[1], &finders[2], &finders[0], span, span); // Finders frame

            let mut centres: Anchors = anchors_from_finders(ver, &finders);

            locate_alignment_centres(&mut img, ver, &ff, &mut centres);

            // The measured centroid is a sub-pixel figure now, so it is checked against the
            // exact placement rather than a rounded one. Half a pixel is the slack a
            // thresholded 3x3-module stone leaves at 3 px per module.
            for (row, &apy) in ap_coords.iter().enumerate() {
                for (col, &apx) in ap_coords.iter().enumerate() {
                    if [(0, 0), (0, n - 1), (n - 1, 0)].contains(&(row, col)) {
                        continue;
                    }
                    let actual = p(centre_px(apx as f64), centre_px(apy as f64));
                    let pred = centres[row][col];
                    assert!(
                        pred.is_some_and(|pred| pred.approx_eq(&actual)),
                        "Version = {v}, Row = {row}, Column = {col}: {pred:?}"
                    );
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
        let mut img = BinaryImage::prepare(&qr.to_gray_image(k as u32));

        // Finder centres for a version 1 symbol at 3 px per module with a 4 module quiet zone
        let finders = [p(22.0, 64.0), p(22.0, 22.0), p(64.0, 22.0)];
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
        let centre_px = |m: f64| (q + m + 0.5) * k;

        for v in 1..=40 {
            let ver = Version::Normal(v);
            let w = ver.width() as f64;

            // Finder centres sit on module 3 and module w - 4
            let near = centre_px(3.0);
            let far = centre_px(w - 4.0);
            let c0 = p(near, far);
            let c1 = p(near, near);
            let c2 = p(far, near);

            let centres = anchors_from_finders(ver, &[c0, c1, c2]);
            let last = ver.alignment_pattern().len().max(2) - 1;

            assert!(
                centres[last][0].is_some_and(|c| c.approx_eq(&c0)),
                "Version = {v}: BL finder centre"
            );
            assert!(
                centres[0][0].is_some_and(|c| c.approx_eq(&c1)),
                "Version = {v}: TL finder centre"
            );
            assert!(
                centres[0][last].is_some_and(|c| c.approx_eq(&c2)),
                "Version = {v}: TR finder centre"
            );

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
        let centre_px = |m: f64| (q + m + 0.5) * k;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let img = BinaryImage::prepare(&qr.to_gray_image(k as u32));

        // Finder centres sit on module 3 and module w - 4
        let w = ver.width() as f64;
        let near = centre_px(3.0);
        let far = centre_px(w - 4.0);
        let c0 = p(near, far);
        let c1 = p(near, near);
        let c2 = p(far, near);
        let span = w - 7.0; // Modules between finders
        let ff = LocalFrame::new(&c1, &c2, &c0, span, span); // Finders frame

        let br = locate_br_anchor(&img, ver, &[c0, c1, c2], &ff);

        // The calculated centre is (215, 215). The quiet zone score plateaus around it, and the
        // nearest-to-TL tie break settles those ties on the inward edge of that plateau, so the
        // anchor lands a few pixels short of the calculated point.
        assert!(p(far, far).approx_eq(&p(215.0, 215.0)), "Calculated centre");
        assert!(br.approx_eq(&p(212.0, 213.0)), "BR anchor: {br:?}");
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
        let centre_px = |m: f64| (q + m + 0.5) * k;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let img = BinaryImage::prepare(&qr.to_gray_image(k as u32));

        // Finder centres sit on module 3 and module w - 4
        let w = ver.width() as f64;
        let near = centre_px(3.0);
        let far = centre_px(w - 4.0);
        let c0 = p(near, far);
        let c1 = p(near, near);
        let c2 = p(far, near);
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
            let near = (q + 3.5) * k;
            let far = (q + w - 3.5) * k;
            let c0 = p(near, far);
            let c1 = p(near, near);
            let c2 = p(far, near);
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
                        let exp_centre =
                            p((q + aps[c] as f64 + 0.5) * k, (q + aps[r] as f64 + 0.5) * k);
                        let centre = centres[r][c];
                        assert!(
                            centre.is_some_and(|centre| centre.approx_eq(&exp_centre)),
                            "Version = {v}, Row = {r}, Column = {c}: {centre:?}"
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
            let near = (q + 3.5) * k;
            let far = (q + w - 3.5) * k;
            let c0 = p(near, far);
            let c1 = p(near, near);
            let c2 = p(far, near);
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
                    let exp_centre =
                        p((q + aps[c] as f64 + 0.5) * k, (q + aps[r] as f64 + 0.5) * k);
                    let centre = centres[r][c];
                    assert!(
                        centre.is_some_and(|centre| centre.approx_eq(&exp_centre)),
                        "Version = {v}, Row = {r}, Column = {c}: {centre:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn test_line_intersection_perpendicular() {
        // Horizontal line y = 0 crossed by the vertical line x = 4
        let res = line_intersection(p(0.0, 0.0), p(10.0, 0.0), p(4.0, -5.0), p(4.0, 5.0));
        assert!(res.is_some_and(|r| r.approx_eq(&p(4.0, 0.0))), "Horizontal x vertical: {res:?}");

        // The two diagonals of a square meet at its centre
        let res = line_intersection(p(0.0, 0.0), p(10.0, 10.0), p(0.0, 10.0), p(10.0, 0.0));
        assert!(res.is_some_and(|r| r.approx_eq(&p(5.0, 5.0))), "Square diagonals: {res:?}");
    }

    #[test]
    fn test_line_intersection_is_order_independent() {
        let (a, b, c, d) = (p(-6.0, -2.0), p(6.0, 10.0), p(-6.0, 10.0), p(6.0, -2.0));

        let meets = |res: Option<PointF>| res.is_some_and(|r| r.approx_eq(&p(0.0, 4.0)));
        assert!(meets(line_intersection(a, b, c, d)), "As given");
        assert!(meets(line_intersection(b, a, c, d)), "First line reversed");
        assert!(meets(line_intersection(a, b, d, c)), "Second line reversed");
        assert!(meets(line_intersection(c, d, a, b)), "Lines swapped");
    }

    #[test]
    fn test_line_intersection_extends_beyond_endpoints() {
        // The lines are infinite, so the meeting point need not lie on either segment
        let res = line_intersection(p(0.0, 0.0), p(1.0, 1.0), p(10.0, 0.0), p(10.0, 1.0));
        assert!(res.is_some_and(|r| r.approx_eq(&p(10.0, 10.0))), "Beyond the far end: {res:?}");

        // Meeting point behind the start of the first segment
        let res = line_intersection(p(0.0, 0.0), p(1.0, 1.0), p(-4.0, 0.0), p(-4.0, 1.0));
        assert!(res.is_some_and(|r| r.approx_eq(&p(-4.0, -4.0))), "Behind the start: {res:?}");
    }

    #[test]
    fn test_line_intersection_parallel_and_collinear() {
        // Parallel horizontals
        let res = line_intersection(p(0.0, 0.0), p(10.0, 0.0), p(0.0, 5.0), p(10.0, 5.0));
        assert!(res.is_none(), "Parallel horizontals");

        // Parallel diagonals
        let res = line_intersection(p(0.0, 0.0), p(10.0, 10.0), p(3.0, 0.0), p(13.0, 10.0));
        assert!(res.is_none(), "Parallel diagonals");

        // Collinear, overlapping
        let res = line_intersection(p(0.0, 0.0), p(10.0, 0.0), p(2.0, 0.0), p(5.0, 0.0));
        assert!(res.is_none(), "Collinear, overlapping");

        // Collinear, disjoint
        let res = line_intersection(p(0.0, 0.0), p(4.0, 4.0), p(10.0, 10.0), p(20.0, 20.0));
        assert!(res.is_none(), "Collinear, disjoint");
    }

    #[test]
    fn test_line_intersection_degenerate_lines() {
        // A line given by a single repeated point has no direction
        let res = line_intersection(p(3.0, 3.0), p(3.0, 3.0), p(0.0, 0.0), p(10.0, 10.0));
        assert!(res.is_none(), "First line degenerate");
        let res = line_intersection(p(0.0, 0.0), p(10.0, 10.0), p(7.0, 1.0), p(7.0, 1.0));
        assert!(res.is_none(), "Second line degenerate");
        let res = line_intersection(p(3.0, 3.0), p(3.0, 3.0), p(7.0, 1.0), p(7.0, 1.0));
        assert!(res.is_none(), "Both lines degenerate");
    }

    // The meeting point is where a missing alignment centre is placed, and it is what a tile
    // homography is then fitted to, so the fraction is kept rather than snapped to a pixel.
    #[test]
    fn test_line_intersection_keeps_sub_pixel_precision() {
        // Meeting points off the pixel grid come back as is, not rounded
        let res = line_intersection(p(0.0, 0.0), p(2.0, 1.0), p(1.0, -5.0), p(1.0, 5.0));
        assert!(res.is_some_and(|r| r.approx_eq(&p(1.0, 0.5))), "Half pixel: {res:?}");

        let res = line_intersection(p(0.0, 0.0), p(2.0, -1.0), p(1.0, -5.0), p(1.0, 5.0));
        assert!(res.is_some_and(|r| r.approx_eq(&p(1.0, -0.5))), "Negative half pixel: {res:?}");

        let res = line_intersection(p(0.0, 0.0), p(4.0, 1.0), p(1.0, -5.0), p(1.0, 5.0));
        assert!(res.is_some_and(|r| r.approx_eq(&p(1.0, 0.25))), "Quarter pixel: {res:?}");

        // Endpoints off the pixel grid too
        let res = line_intersection(p(0.5, 0.5), p(2.5, 2.5), p(0.5, 2.5), p(2.5, 0.5));
        assert!(res.is_some_and(|r| r.approx_eq(&p(1.5, 1.5))), "Fractional endpoints: {res:?}");
    }

    #[test]
    fn test_line_intersection_near_parallel() {
        // Equal slopes over a long span never meet, however close the endpoints are
        let res = line_intersection(p(0.0, 0.0), p(1000.0, 1000.0), p(0.0, 1.0), p(1000.0, 1001.0));
        assert!(res.is_none(), "Same slope, offset by one -- parallel");

        // Slopes differing by the least integer endpoints allow still cross, out at the far end
        let res = line_intersection(p(0.0, 0.0), p(1000.0, 1000.0), p(0.0, 1.0), p(1000.0, 1000.0));
        assert!(res.is_some_and(|r| r.approx_eq(&p(1000.0, 1000.0))), "Barely converging: {res:?}");
    }

    #[test]
    fn test_line_intersection_on_alignment_grid() {
        // Four located centres of an alignment grid: the row through (r, c) and the column
        // through it should meet at the missing centre, (60, 30)
        let (row_near, row_far) = (p(30.0, 30.0), p(6.0, 30.0)); // Same row as the target
        let (col_near, col_far) = (p(60.0, 54.0), p(60.0, 78.0)); // Same column as the target

        let res = line_intersection(row_near, row_far, col_near, col_far);
        assert!(res.is_some_and(|r| r.approx_eq(&p(60.0, 30.0))), "Missing centre: {res:?}");
    }

    // The centre stored in cell (row, col) of a test grid. Each cell carries a distinct point,
    // so a returned centre names the cell it was taken from.
    fn cell(row: usize, col: usize) -> PointF {
        p(col as f64 * 10.0, row as f64 * 10.0)
    }

    // Every cell of the whole `MAX_ALIGN_CELLS` square filled, the version's own grid included.
    // Cells outside the version's grid are filled too -- `nearest_pair` must not reach them.
    fn filled_grid() -> Anchors {
        let mut centres: Anchors = [[None; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];
        for (r, row) in centres.iter_mut().enumerate() {
            for (c, centre) in row.iter_mut().enumerate() {
                *centre = Some(cell(r, c));
            }
        }
        centres
    }

    #[test]
    fn test_nearest_pair_picks_nearest_on_each_axis() {
        let ver = Version::Normal(21); // 5x5 alignment grid
        assert_eq!(ver.alignment_pattern().len(), 5);

        let centres = filled_grid();

        // Target sits in the middle, so both axes have neighbours on either side. Equidistant
        // candidates are settled by the zigzag's leading direction -- left before right, above
        // before below -- so the nearer of each pair is the one at the lower index.
        let (xn, xsn, yn, ysn) = nearest_pair(2, 2, ver, &centres).unwrap();
        assert!(xn.approx_eq(&cell(2, 1)), "Nearest along the row");
        assert!(xsn.approx_eq(&cell(2, 3)), "Second nearest along the row");
        assert!(yn.approx_eq(&cell(1, 2)), "Nearest along the column");
        assert!(ysn.approx_eq(&cell(3, 2)), "Second nearest along the column");
    }

    #[test]
    fn test_nearest_pair_skips_finder_cells() {
        let ver = Version::Normal(21); // 5x5 alignment grid
        let centres = filled_grid();

        // The row of a top edge cell opens on the TL finder at (0, 0). That cell holds a finder
        // centre, not an alignment centre, so the sweep passes it and reaches further right.
        let (xn, xsn, yn, ysn) = nearest_pair(0, 1, ver, &centres).unwrap();
        assert!(xn.approx_eq(&cell(0, 2)), "Nearest along the row");
        assert!(xsn.approx_eq(&cell(0, 3)), "Second nearest along the row");
        assert!(yn.approx_eq(&cell(1, 1)), "Nearest along the column");
        assert!(ysn.approx_eq(&cell(2, 1)), "Second nearest along the column");

        // Same along a column, where the sweep opens on the TL finder going up
        let (xn, xsn, yn, ysn) = nearest_pair(1, 0, ver, &centres).unwrap();
        assert!(xn.approx_eq(&cell(1, 1)), "Nearest along the row");
        assert!(xsn.approx_eq(&cell(1, 2)), "Second nearest along the row");
        assert!(yn.approx_eq(&cell(2, 0)), "Nearest along the column");
        assert!(ysn.approx_eq(&cell(3, 0)), "Second nearest along the column");
    }

    #[test]
    fn test_nearest_pair_reaches_past_unlocated_cells() {
        let ver = Version::Normal(21); // 5x5 alignment grid
        let mut centres = filled_grid();

        // Knock out both immediate neighbours along the row, and the one above along the column
        centres[2][1] = None;
        centres[2][3] = None;
        centres[1][2] = None;

        // Row falls back to distance 2. Down at distance 1 beats up at distance 2
        let (xn, xsn, yn, ysn) = nearest_pair(2, 2, ver, &centres).unwrap();
        assert!(xn.approx_eq(&cell(2, 0)), "Nearest along the row");
        assert!(xsn.approx_eq(&cell(2, 4)), "Second nearest along the row");
        assert!(yn.approx_eq(&cell(3, 2)), "Nearest along the column");
        assert!(ysn.approx_eq(&cell(0, 2)), "Second nearest along the column");
    }

    #[test]
    fn test_nearest_pair_ignores_cells_outside_the_version_grid() {
        let ver = Version::Normal(7); // 3x3 alignment grid
        assert_eq!(ver.alignment_pattern().len(), 3);

        let centres = filled_grid(); // Cells 3..7 are filled but out of this version's grid

        let (xn, xsn, yn, ysn) = nearest_pair(1, 1, ver, &centres).unwrap();
        assert!(xn.approx_eq(&cell(1, 0)), "Nearest along the row");
        assert!(xsn.approx_eq(&cell(1, 2)), "Second nearest along the row");
        assert!(yn.approx_eq(&cell(0, 1)), "Nearest along the column");
        assert!(ysn.approx_eq(&cell(2, 1)), "Second nearest along the column");

        // Cell (2, 2) has (2, 1) to its left, the BL finder at (2, 0) beyond that, and nothing
        // to its right within the 3x3 grid -- the filled cells at (2, 3) onward are out of reach
        assert!(nearest_pair(2, 2, ver, &centres).is_none());
    }

    #[test]
    fn test_nearest_pair_needs_two_centres_on_both_axes() {
        // A 2x2 grid is all finders but for cell (1, 1), which has no company on either axis
        let ver = Version::Normal(2);
        assert_eq!(ver.alignment_pattern().len(), 2);
        assert!(nearest_pair(1, 1, ver, &filled_grid()).is_none());

        let ver = Version::Normal(21); // 5x5 alignment grid

        // Row is one centre short
        let mut centres = filled_grid();
        for c in [0, 1, 3] {
            centres[2][c] = None;
        }
        assert!(nearest_pair(2, 2, ver, &centres).is_none(), "Only (2, 4) left in the row");

        // Row has its pair, column is one centre short
        let mut centres = filled_grid();
        for r in [0, 3, 4] {
            centres[r][2] = None;
        }
        assert!(nearest_pair(2, 2, ver, &centres).is_none(), "Only (1, 2) left in the column");

        // Nothing located at all
        let centres: Anchors = [[None; MAX_ALIGN_CELLS]; MAX_ALIGN_CELLS];
        assert!(nearest_pair(2, 2, ver, &centres).is_none());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "assertion `left != right` failed")]
    fn test_nearest_pair_rejects_versions_without_alignment_patterns() {
        // Version 1 carries no alignment coordinates, so there is no grid to sweep
        nearest_pair(0, 0, Version::Normal(1), &filled_grid());
    }
}

// Global constants
//------------------------------------------------------------------------------

const ALIGNMENT_CENTRE_DRIFT_TOLERANCE: f64 = 0.5;

const ALIGNMENT_SEARCH_RADIUS: f64 = 4.0;

const BR_ANCHOR_SEARCH_RADIUS: f64 = 0.5;

const ALIGNMENT_TRACE_SLACK: f64 = 3.0;
