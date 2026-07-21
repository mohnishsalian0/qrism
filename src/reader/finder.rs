use crate::metadata::Color;

use super::{
    binarize::BinaryImage,
    utils::{geometry::Point, matches_finder_ratio, verify_finder_pattern},
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

// ENTRY POINT FOR LOCATING FINDER
// Returns a list of centres of potential finder
pub fn locate_finders(img: &mut BinaryImage) -> Vec<Point> {
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
fn verify_and_mark_finder(img: &mut BinaryImage, datum: &DatumLine) -> Option<Point> {
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

    let stone = img.get_region((s, y)).clone();
    let ring = img.get_region((r, y)).clone();

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

    Some(stone.centre)
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
            assert_eq!(*f, cent_pt, "Finder centre doesn't match");
        }
    }
}

// Groups finders in 3, which form potential symbols
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct FinderGroup {
    pub finders: [Point; 3], // [BL, TL, TR]
    pub score: f64,          // Timing pattern score + Estimate mod count score
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

pub fn group_finders(finders: &[Point]) -> Vec<FinderGroup> {
    // Store all possible combinations of finders
    let mut groups: Vec<FinderGroup> = Vec::new();
    let right_angle = 90f64.to_radians();

    for (i1, f1) in finders.iter().enumerate() {
        for (i2, f2) in finders.iter().enumerate() {
            if i2 == i1 {
                continue;
            }

            for (i3, f3) in finders.iter().enumerate() {
                if i3 <= i2 || i3 == i1 {
                    continue;
                }

                let d12 = f1.dist_sq(f2);
                let d13 = f1.dist_sq(f3);

                // Closeness of the dist of bl and tr finders from tl finder
                let symmetry_score = ((d12 as f64 / d13 as f64).sqrt() - 1.0).abs();
                if symmetry_score > SYMMETRY_THRESHOLD {
                    continue;
                }

                // Angle of c2-c1-c3
                let angle = angle(f2, f1, f3);
                let angle_score = ((angle / right_angle) - 1.0).abs();
                if angle_score > ANGLE_THRESHOLD {
                    continue;
                }

                let score = symmetry_score + angle_score;

                // Create and push group into groups
                let finders = [*f3, *f1, *f2];
                let group = FinderGroup { finders, score };
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
