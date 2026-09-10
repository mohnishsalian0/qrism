use crate::metadata::Color;

use super::{
    binarize::BinaryImage,
    utils::{geometry::Point, matches_finder_ratio, verify_finder_diagonal, verify_finder_pattern},
};

#[cfg(test)]
use image::RgbImage;

// Finder line
//------------------------------------------------------------------------------

// **   ******   **  <- Finder line
// ^    ^        ^
// left |        right
//      stone
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct DatumLine {
    left: u32,
    stone: u32,
    right: u32,
    y: u32,
}

// Line scanner to detect finder line
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct LineScanner {
    pub buffer: [u32; 6], // Run length of each transition
    prev: Option<Color>,  // Last observed color
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

    pub fn advance(&mut self, color: Color) -> Option<DatumLine> {
        self.pos += 1;

        if self.prev.is_some() && self.prev == Some(color) {
            self.buffer[5] += 1;
            return None;
        }

        self.buffer.rotate_left(1);
        self.buffer[5] = 1;
        self.prev = Some(color);
        self.flips += 1;

        if self.is_finder_line() {
            Some(DatumLine {
                left: self.pos - 1 - self.buffer[..5].iter().sum::<u32>(),
                stone: self.pos - 1 - self.buffer[2..5].iter().sum::<u32>(),
                right: self.pos - 1 - self.buffer[4],
                y: self.y,
            })
        } else {
            None
        }
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

        for x in 0..w {
            let color = img.get(x, y).unwrap();
            let datum = match scanner.advance(color) {
                Some(d) => d,
                None => continue,
            };

            if let Some(centre) = verify_and_mark_finder(img, &datum) {
                finders.push(centre);
            }
        }

        // Handles an edge case where the QR is located at the right edge of the image
        if let Some(datum) = scanner.advance(Color::White) {
            if let Some(centre) = verify_and_mark_finder(img, &datum) {
                finders.push(centre);
            }
        }

        y += skip;
    }

    finders
}

// Checks multiple conditions to ensure the finder is valid
// 1. Left and right datum points are connected
// 2. The region wasn't already marked as candidate
// 3. Ring and stone regions aren't connected
// 4. Area of stone region is roughly 37.5% of ring region
// 5. Crosscheck 1:1:3:1:1 pattern along Y axis
// Finally it marks the regions are candidate and returns the centre
fn verify_and_mark_finder(img: &mut BinaryImage, datum: &DatumLine) -> Option<Finder> {
    let (l, r, s, y) = (datum.left, datum.right, datum.stone, datum.y);

    // If pixel has been visited, check if regions is already marked as finder
    if img.get_region_id(s, y).is_some() {
        let stone = img.get_region((s, y));

        // Exit if stone is already made a candidate from previous iterations
        if stone.is_finder {
            return None;
        }
    }

    let sx = r - (s - l) * 5 / 4;
    let seed = Point { x: sx as i32, y: datum.y as i32 };
    let pattern = [1.0, 1.0, 3.0, 1.0, 1.0];
    let max_run = (r - l) * 2; // Setting a loose upper limit on the run

    // Verify 1:1:3:1:1 pattern along Y axis. Returns the top and bottom pts if valid
    let (t, b) = verify_finder_pattern(img, &seed, &pattern, max_run)?;

    // Cheap reject before the expensive stone flood fill, run on the ~1 in 6 candidates that clear
    // the vertical crosscheck but are mostly not finders. Confirms the 1:1:3:1:1 ratio along the main
    // diagonal through the centre — a third independent axis a spurious candidate almost never
    // satisfies. This cuts the number of stone fills (the single biggest cost in `locate_finders`)
    // without touching the area-ratio confirmation that follows.
    let centre = Point { x: sx as i32, y: ((t + b) / 2) as i32 };
    if !verify_finder_diagonal(img, &centre, max_run) {
        return None;
    }

    // Cap both fills so a spurious candidate whose stone/ring bleeds into a large background blob
    // is rejected without filling the whole blob. Both caps derive from row-stable quantities — the
    // finder-width bound `max_run` (~2x the 7-module span, so `max_run²` comfortably exceeds a real
    // stone even when it bleeds into a few data modules) and the stone area — so the "oversized"
    // verdict is identical on every scan row crossing this finder, which the memoised sentinel needs.
    let stone_cap = max_run.saturating_mul(max_run);
    // let stone_cap = max_run * 2;
    let stone = img.get_region_capped((s, y), stone_cap)?.clone();

    // A valid ring is at most ~10x the stone area, else the area-ratio check below rejects it.
    let ring_cap = stone.area.saturating_mul(10);
    // let ring_cap = max_run * 4;
    let ring = img.get_region_capped((r, y), ring_cap)?.clone();

    // Check if left, top and bottom points lie within the ring
    let lid = img.get_region_id(l, y)? as usize;
    let tid = img.get_region_id(sx, t)? as usize;
    let bid = img.get_region_id(sx, b)? as usize;
    if lid != ring.id || tid != ring.id || bid != ring.id {
        return None;
    }

    // False if ring & stone are connected, or if ring to stone area is not roughly 37,5%
    let ratio = stone.area * 100 / ring.area;
    if stone.id == ring.id || ratio <= 10 || 70 <= ratio {
        return None;
    }

    img.get_region((r, y)).is_finder = true;
    img.get_region((s, y)).is_finder = true;

    // The stone is the central 3x3-module block, so its area is ~9 modules^2. This estimate only
    // feeds the loose scale gates in `group_finders`, so it needn't be exact.
    let mod_size = (stone.area as f32 / 9.0).sqrt();

    Some(Finder { c: stone.centre, mod_size })
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
        let img = qr.to_image(10);

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
    let right_angle = 90f64.to_radians();

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
        let min_d = (MIN_CENTRE_SPAN_MODULES * m) as f64;
        let max_d = (MAX_CENTRE_SPAN_MODULES * m) as f64;
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

                // Angle of c2-c1-c3. Gate on the cosine (no acos in the reject path): the accepted
                // window [45, 135] degrees is exactly |cos| <= cos(45).
                let ab = ((f2.c.x - f1.c.x) as f64, (f2.c.y - f1.c.y) as f64);
                let cb = ((f3.c.x - f1.c.x) as f64, (f3.c.y - f1.c.y) as f64);
                let dot = ab.0 * cb.0 + ab.1 * cb.1;
                let mag_sq = (d12 as f64) * (d13 as f64);
                if dot * dot > COS_45_SQ * mag_sq {
                    continue;
                }

                // Survivor: compute the exact angle_score so the ranking (and thus the greedy
                // selection in `locate_symbols`) is identical to the pre-refactor code. acos now
                // runs only on survivors, not on every triple.
                let angle = angle(&f2.c, &f1.c, &f3.c);
                let angle_score = ((angle / right_angle) - 1.0).abs();
                if angle_score > ANGLE_THRESHOLD {
                    continue;
                }

                let score = symmetry_score + angle_score;

                // Create and push group into groups
                let group = FinderGroup { finders: [f3.c, f1.c, f2.c], score };
                groups.push(group);
            }
        }
    }

    groups.sort_unstable_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

    groups
}

// Angle between AB & BC in radians
fn angle(a: &Point, b: &Point, c: &Point) -> f64 {
    let ab = ((a.x - b.x) as f64, (a.y - b.y) as f64);
    let cb = ((c.x - b.x) as f64, (c.y - b.y) as f64);

    let dot = ab.0 * cb.0 + ab.1 * cb.1;
    let mag_ab = (ab.0.powi(2) + ab.1.powi(2)).sqrt();
    let mag_cb = (cb.0.powi(2) + cb.1.powi(2)).sqrt();

    if mag_ab <= f64::EPSILON || mag_cb <= f64::EPSILON {
        return 0.0;
    }

    let cos_theta = (dot / (mag_ab * mag_cb)).clamp(-1.0, 1.0);

    cos_theta.acos()
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
        let img = qr.to_image(10);

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

pub const ANGLE_THRESHOLD: f64 = 0.5;

// cos(45 degrees)^2 = 0.5. The vertex-angle window [45, 135] degrees is exactly |cos| <= cos(45),
// so a triple passes the angle gate iff dot^2 <= COS_45_SQ * |ab|^2 * |cb|^2.
pub const COS_45_SQ: f64 = 0.5;

// Two finders of the same symbol share a module size; reject an arm whose module size differs from
// the vertex's by more than this ratio. Loose enough to never clip a real symbol.
pub const MOD_SIZE_RATIO: f32 = 2.0;

// Finder centre-to-centre distance spans (symbol_side - 7) modules; symbol_side in [21, 177] gives
// ~[14, 170]. These loosened bounds keep every real symbol while rejecting cross-symbol arm pairs.
pub const MIN_CENTRE_SPAN_MODULES: f32 = 10.0;
pub const MAX_CENTRE_SPAN_MODULES: f32 = 185.0;
