use super::{
    binarize::BinaryImage,
    utils::{geometry::Point, matches_finder_ratio, verify_finder_diagonal, verify_finder_pattern},
};

#[cfg(test)]
use image::RgbImage;

// Finder line
//------------------------------------------------------------------------------

// ***   *********   ***  <- Finder line
// ^     ^       ^     ^
// rl    |       |     rr
//       sl      sr
// rl = Ring left, rr = Ring right
// sl = Stone left, sr = Stone right
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct DatumLine {
    rl: u32,
    rr: u32,
    sl: u32,
    sr: u32,
    y: u32,
}

// Line scanner to detect finder line
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct LineScanner {
    pub buffer: [u32; 6], // Run length of each transition
    prev: Option<bool>,   // Last observed color. true = white, false = black
    flips: u32,           // Count of color changes
    pos: u32,             // Current position
    y: u32,
}

impl LineScanner {
    pub fn new() -> Self {
        Self { buffer: [0; 6], prev: None, flips: 0, pos: 0, y: 0 }
    }

    pub fn reset(&mut self, y: u32) {
        self.buffer[5] = 0;
        self.prev = None;
        self.flips = 0;
        self.pos = 0;
        self.y = y;
    }

    pub fn advance(&mut self, color: bool) -> Option<DatumLine> {
        self.advance_run(color, 1)
    }

    // Advances a whole run of pixels with the color instead of pixel-by-pixel.
    // true = white, false = black
    pub fn advance_run(&mut self, color: bool, len: u32) -> Option<DatumLine> {
        if self.prev == Some(color) {
            self.buffer[5] += len;
            self.pos += len;
            return None;
        }

        self.pos += 1;
        self.buffer.rotate_left(1);
        self.buffer[5] = 1;
        self.prev = Some(color);
        self.flips += 1;

        let datum = if self.is_finder_line() {
            Some(DatumLine {
                rl: self.pos - 1 - self.buffer[..5].iter().sum::<u32>(),
                sl: self.pos - 1 - self.buffer[2..5].iter().sum::<u32>(),
                sr: self.pos - 1 - self.buffer[3..5].iter().sum::<u32>(),
                rr: self.pos - 2,
                y: self.y,
            })
        } else {
            None
        };

        self.buffer[5] += len - 1;
        self.pos += len - 1;

        datum
    }

    // Validates whether last 5 run lengths are in the 1:1:3:1:1 ratio
    fn is_finder_line(&self) -> bool {
        if self.flips < 5 {
            return false;
        }

        matches_finder_ratio(&self.buffer[..5])
    }
}

// Locate finders
//------------------------------------------------------------------------------

// A verified finder candidate: its stone centre plus an estimated module size (px).
// The module size lets `group_finders` reject cross-symbol triples by scale, without
// which grouping degenerates into an O(n^3) explosion on dense scenes.
#[derive(Debug, Clone, Copy)]
pub struct Finder {
    pub c: Point,      // stone centre
    pub mod_size: f32, // estimated module size in px
}

// ENTRY POINT FOR LOCATING FINDER
// Returns a list of potential finders (stone centre + module size estimate)
pub fn locate_finders(img: &mut BinaryImage) -> Vec<Finder> {
    let mut finders = Vec::with_capacity(100);
    let w = img.w;
    let h = img.h;
    let mut scanner = LineScanner::new();

    // Scan only every `skip`-th row. A finder's centre band is ~3 modules (>= 3 px) tall and
    // shows a full horizontal 1:1:3:1:1 profile across every row it spans, so a stride of 3 is
    // guaranteed to land on at least one of those rows. rxing's larger `(3h)/(4*MAX_MODULES)`
    // stride assumes the symbol spans >= 1/4 of the image height; capping at 3 keeps the tiny
    // symbols in the `lots` dataset detectable while still skipping ~2/3 of rows on big images.
    let skip = ((3 * h) / (4 * MAX_FINDER_MODULES)).clamp(1, 3);

    let mut y = skip - 1;
    while y < h {
        scanner.reset(y);

        // Step by whole colour runs
        let mut x = 0;
        while x < w {
            let (color, len) = img.run(x, y).unwrap();
            x += len;

            let datum = match scanner.advance_run(color, len) {
                Some(d) => d,
                None => continue,
            };

            if let Some(centre) = verify_and_mark_finder(img, &datum) {
                finders.push(centre);
            }
        }

        // Handles an edge case where the QR is located at the right edge of the image
        if let Some(datum) = scanner.advance(true) {
            if let Some(centre) = verify_and_mark_finder(img, &datum) {
                finders.push(centre);
            }
        }

        y += skip;
    }

    finders
}

// Verifies a finder candidate by walking the outer boundary of its stone and ring, which costs
// O(perimeter) rather than O(area). Three properties of the walk shape the checks below:
// 1. A seed must be the last pixel of a horizontal run, so `trace` starts on an outer boundary
//    rather than a hole's, hence `w - 1` for the stone and `e` for the ring, not `s` and `r`.
// 2. Only boundary pixels carry a contour id, so a point can be tested against a contour only if
//    it is extreme along some axis (leftmost in its row, top/bottom-most in its column).
// 3. A traced outline encloses its holes, so `ring.area()` is the whole 7x7 block, not the annulus.
fn verify_and_mark_finder(img: &mut BinaryImage, datum: &DatumLine) -> Option<Finder> {
    let (rl, sl, sr, rr, y) = (datum.rl, datum.sl, datum.sr, datum.rr, datum.y);

    let sx = (sl + sr + 1).div_ceil(2);
    let probe = (sx, y);

    // Both caps are estimated from this row's stone run, so they shift slightly row to row. A
    // square stone of side n has a crack perimeter of 4n, so the 8x here is 2x the ideal, leaving
    // room for the staircasing a thresholded edge adds; `max_dist` bounds how far the walk may
    // stray from the centre, which catches a runaway blob long before the step cap does.
    let max_width = (sr - sl) * 3;

    // A finder spans several scan rows, so the rows after the first re-reach a stone already
    // accepted. `w - 1` is the stone's rightmost pixel on this row, which the outer walk always
    // touches, so its contour id is the memo. `get_contour_capped` returns None for a stone that
    // bailed or that this row's caps reject; that is not an accept, so fall through and re-check.
    if img.get_px_contour(sr - 1, y).is_some()
        && img.get_contour_capped((sr - 1, y), probe, max_width).is_some_and(|st| st.is_finder)
    {
        return None;
    }

    let seed = Point { x: sx as i32, y: datum.y as i32 };
    let pattern = [1.0, 1.0, 3.0, 1.0, 1.0];
    let max_run = (rr - rl) * 2; // Setting a loose upper limit on the run

    // Verify 1:1:3:1:1 pattern along Y axis. Returns the top and bottom pts if valid
    let (t, b) = verify_finder_pattern(img, &seed, &pattern, max_run)?;

    // Cheap reject before the stone trace, run on the ~1 in 6 candidates that clear the vertical
    // crosscheck but are mostly not finders. Confirms the 1:1:3:1:1 ratio along the main diagonal
    // through the centre — a third independent axis a spurious candidate almost never satisfies.
    // This keeps the two traces below off candidates a pixel walk can already rule out.
    let centre = Point { x: sx as i32, y: ((t + b) / 2) as i32 };
    if !verify_finder_diagonal(img, &centre, max_run) {
        return None;
    }

    // Cap both walks so a candidate whose stone or ring bleeds into a large background blob is
    // rejected after a few hundred steps rather than walking the blob's whole outline. A stone
    // fused to its ring is caught separately, by `trace`: it has no outer boundary of its own, so
    // the walk closes on the hole and the negative area is rejected there.
    let stone = img.get_contour_capped((sr - 1, y), probe, max_width)?.clone();
    let sc = stone.compactness();
    if !(MIN_COMPACTNESS_THRESHOLD..MAX_COMPACTNESS_THRESHOLD).contains(&sc) {
        return None;
    }

    // A ring broken anywhere (one bleached or blurred module on its border) has no hole, so the
    // walk dives through the gap and traces the inner edge as well as the outer. This doubles the
    // perimeter
    let ring_max_width = stone.perimeter().saturating_mul(RING_PERIMETER_MULT).div_ceil(4);
    let ring = img.get_contour_capped((rr, y), probe, ring_max_width)?.clone();

    // Compactness is only meaningful for a simple outline; a broken ring's doubled-back walk makes
    // it large by construction, so the check applies to closed rings alone. `ring_max_dist` is what
    // bounds a runaway blob in either case.
    if ring.encloses {
        let rc = ring.compactness();
        if !(MIN_COMPACTNESS_THRESHOLD..MAX_RING_COMPACTNESS).contains(&rc) {
            return None;
        }
    }

    // A closed ring's outline encloses its hole, so this compares the full 7x7 block against the
    // 3x3 stone: 49/9 ~= 5.4. A broken ring traces the annulus instead, so the same finder reads
    // (49 - 25)/9 ~= 2.7, and is gated against that figure rather than rejected for it.
    let ratio = ring.area() as f64 / stone.area() as f64;
    let (min_ratio, max_ratio) = if ring.encloses {
        (CLOSED_RING_MIN, CLOSED_RING_MAX)
    } else {
        (OPEN_RING_MIN, OPEN_RING_MAX)
    };
    if ratio <= min_ratio || max_ratio <= ratio {
        return None;
    }

    // Concentricity test. The ring and stone centre should be reasonably near each other
    let mod_size = stone.area() as f64 / 9.0;
    let max_drift = mod_size * FINDER_CENTRE_DRIFT_TOLERANCE;
    let rcentre = ring.centre()?;
    let scentre = stone.centre()?;
    if rcentre.dist_sq(&scentre) > max_drift.powi(2).round() as u32 {
        return None;
    }

    // Mark via the traced ids rather than a pixel lookup: the walk labels only boundary pixels,
    // so `s` and `r` are not guaranteed to resolve, and both ids are already in hand.
    img.get_contours_mut()[stone.id as usize].is_finder = true;
    img.get_contours_mut()[ring.id as usize].is_finder = true;

    // The stone is the central 3x3-module block, so its area is ~9 modules^2. This estimate only
    // feeds the loose scale gates in `group_finders`, so it needn't be exact.
    let mod_size = (stone.area() as f32 / 9.0).sqrt();

    Some(Finder { c: stone.centre()?, mod_size })
}

#[cfg(test)]
mod finder_tests {

    use crate::{
        reader::{binarize::BinaryImage, utils::geometry::Point},
        ECLevel, MaskPattern, QRBuilder, Version,
    };

    use super::locate_finders;

    #[test]
    fn test_locate_finder() {
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
        let img = qr.to_gray_image(10);

        let centres = [[75, 75], [335, 75], [75, 335]];
        let mut bin_img = BinaryImage::prepare(&img);
        let finders = locate_finders(&mut bin_img);

        for (i, f) in finders.iter().enumerate() {
            let cent_pt = Point { x: centres[i][0], y: centres[i][1] };
            assert_eq!(f.c, cent_pt, "Finder centre doesn't match");
        }
    }
}

// Groups finders in 3, which form potential symbols
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FinderGroup {
    pub finders: [Point; 3], // [BL, TL, TR]
    pub score: f64,          // symmetry_score + angle_score (lower = closer to ideal L)
}

impl FinderGroup {
    #[cfg(test)]
    pub fn highlight(&self, img: &mut RgbImage) {
        use super::utils::rnd_rgb;

        let color = rnd_rgb();
        for f in self.finders.iter() {
            f.highlight(img, color);
        }
    }
}

pub fn group_finders(finders: &[Finder]) -> Vec<FinderGroup> {
    // Store all possible combinations of finders
    let mut groups: Vec<FinderGroup> = Vec::new();

    // Reused per vertex: the arms that clear the cheap scale gates below.
    let mut arms: Vec<(&Finder, u32)> = Vec::new();

    // f1 is the candidate corner (TL); f2 and f3 are its two arms (BL/TR).
    for (i1, f1) in finders.iter().enumerate() {
        // Build the arm list for this vertex, keeping only candidates whose scale and separation
        // are consistent with belonging to the same symbol
        arms.clear();
        let m = f1.mod_size;
        // Finder centre-to-centre distance is (symbol_side - 7) modules, symbol_side in [21, 177],
        // so a valid span is ~[14, 170] modules; the loose bounds below never clip a real symbol.
        let min_d = (MIN_CENTRE_SPAN_MODULES * m * MIN_CENTRE_SPAN_FACTOR) as f64;
        let max_d = (MAX_CENTRE_SPAN_MODULES * m * MAX_CENTRE_SPAN_FACTOR) as f64;
        let (min_d_sq, max_d_sq) = ((min_d * min_d) as u32, (max_d * max_d) as u32);
        for (i2, f2) in finders.iter().enumerate() {
            if i2 == i1 {
                continue;
            }

            // Size-ratio gate: same-symbol finders share a scale; drop mismatched candidates.
            let ratio = f2.mod_size / m;
            if !(1.0 / MOD_SIZE_RATIO..MOD_SIZE_RATIO).contains(&ratio) {
                continue;
            }

            // Module-count / max_dist gate.
            let d = f1.c.dist_sq(&f2.c);
            if d < min_d_sq || d > max_d_sq {
                continue;
            }

            arms.push((f2, d));
        }

        // Pair the (few) surviving arms. `j3 > j2` dedups the unordered {arm, arm} pair.
        for (j2, &(f2, d12)) in arms.iter().enumerate() {
            for &(f3, d13) in arms.iter().skip(j2 + 1) {
                // Closeness of the dist of bl and tr finders from tl finder
                let symmetry_score = ((d12 as f64 / d13 as f64).sqrt() - 1.0).abs();
                if symmetry_score > SYMMETRY_THRESHOLD {
                    continue;
                }

                // Angle of c2-c1-c3. Gate on the cosine. The accepted window [45, 135].
                let ab = ((f2.c.x - f1.c.x) as f64, (f2.c.y - f1.c.y) as f64);
                let cb = ((f3.c.x - f1.c.x) as f64, (f3.c.y - f1.c.y) as f64);
                let dot = ab.0 * cb.0 + ab.1 * cb.1;
                let dot_sq = dot.powi(2);
                let mag_sq = (d12 as f64) * (d13 as f64);
                let angle_score_sq = dot_sq / mag_sq;
                if angle_score_sq > ANGLE_THRESHOLD {
                    continue;
                }

                let score = symmetry_score + angle_score_sq.sqrt();

                // Create and push group into groups
                let group = FinderGroup { finders: [f3.c, f1.c, f2.c], score };
                groups.push(group);
            }
        }
    }

    groups.sort_unstable_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

    groups
}

#[cfg(test)]
mod group_finders_tests {

    use crate::{reader::binarize::BinaryImage, ECLevel, MaskPattern, QRBuilder, Version};

    use super::{group_finders, locate_finders};

    #[test]
    fn test_group_finder() {
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
        let img = qr.to_gray_image(10);

        let centres = [(75, 75), (335, 75), (75, 335)];

        let mut img = BinaryImage::prepare(&img);
        let finders = locate_finders(&mut img);
        let group = group_finders(&finders);
        assert!(!group.is_empty(), "No group found");
        for f in group[0].finders.iter() {
            let c = (f.x, f.y);
            assert!(centres.contains(&c))
        }
    }
}

// Global constants
//------------------------------------------------------------------------------

// Largest QR (version 40) is 177 modules across; used to bound the row-scan stride.
pub const MAX_FINDER_MODULES: u32 = 177;

pub const SYMMETRY_THRESHOLD: f64 = 0.75;

// cos(45 degrees)^2 = 0.5. The vertex-angle window is [45, 135].
pub const ANGLE_THRESHOLD: f64 = 0.5;

// Two finders of the same symbol share a module size; reject an arm whose module size differs from
// the vertex's by more than this ratio. Loose enough to never clip a real symbol.
pub const MOD_SIZE_RATIO: f32 = 2.0;

// Finder centre-to-centre distance spans (symbol_side - 7) modules; symbol_side in [21, 177] gives
// ~[14, 170]. These loosened bounds keep every real symbol while rejecting cross-symbol arm pairs.
pub const MIN_CENTRE_SPAN_MODULES: f32 = 10.0;
pub const MIN_CENTRE_SPAN_FACTOR: f32 = 0.95;
pub const MAX_CENTRE_SPAN_MODULES: f32 = 185.0;
pub const MAX_CENTRE_SPAN_FACTOR: f32 = 1.05;

const MIN_COMPACTNESS_THRESHOLD: f64 = 1.0;
const MAX_COMPACTNESS_THRESHOLD: f64 = 2.5;

// A closed ring's outline runs ~2.3x the stone's perimeter; a broken one doubles back over the
// annulus and runs ~3.8x. 8x clears both with room for a staircased edge -- `ring_max_dist` is what
// actually bounds a runaway blob here, so this cap need not be tight.
const RING_PERIMETER_MULT: u32 = 8;

// Looser than the stone's: the ring is a thin annulus, so a staircased or blurred edge moves its
// compactness far more than it moves a solid block's.
const MAX_RING_COMPACTNESS: f64 = 3.5;

// Ring-to-stone area ratio. A closed ring's outline encloses its hole (49/9 ~= 5.4); a broken one
// traces the annulus instead ((49 - 25)/9 ~= 2.7). The upper closed bound sits well above the ideal
// because blur fattens the ring's outline while eroding the stone, so the measured ratio drifts up:
// a blurred symbol in the bench reads ~8.4. The gate's job is only to reject candidates whose ring
// and stone are wildly mismatched in scale -- across the whole detection dataset removing it
// entirely costs no precision, so a generous ceiling is free.
const CLOSED_RING_MIN: f64 = 3.0;
const CLOSED_RING_MAX: f64 = 10.0;
const OPEN_RING_MIN: f64 = 1.0;
const OPEN_RING_MAX: f64 = 4.0;

// For ring and stone centre closeness
const FINDER_CENTRE_DRIFT_TOLERANCE: f64 = 0.5;
