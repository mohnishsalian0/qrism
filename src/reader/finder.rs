use crate::metadata::Color;

use super::{
    binarize::{BinaryImage, Pixel},
    utils::{
        geometry::{Point, Y},
        verify_pattern,
    },
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

        // Verify 1:1:3:1:1 ratio
        let avg = (self.buffer[..5].iter().sum::<u32>() as f64) / 7.0;
        let tol = avg * 3.0 / 4.0;

        let ratio: [f64; 5] = [1.0, 1.0, 3.0, 1.0, 1.0];
        for (i, r) in ratio.iter().enumerate() {
            let rl = self.buffer[i] as f64;
            if rl < r * avg - tol || rl > r * avg + tol {
                return false;
            }
        }

        true
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

    for y in 0..h {
        for x in 0..w {
            let color = Color::from(img.get(x, y).unwrap());
            let datum = match scanner.advance(color) {
                Some(d) => d,
                None => continue,
            };

            if let Some(centre) = verify_and_mark_finder(img, &scanner, &datum) {
                finders.push(centre);
            }
        }

        // Handles an edge case where the QR is located at the right edge of the image
        if let Some(datum) = scanner.advance(Color::White) {
            if let Some(centre) = verify_and_mark_finder(img, &scanner, &datum) {
                finders.push(centre);
            }
        }

        scanner.reset(y + 1);
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
fn verify_and_mark_finder(
    img: &mut BinaryImage,
    scn: &LineScanner,
    datum: &DatumLine,
) -> Option<Point> {
    let (l, r, s, y) = (datum.left, datum.right, datum.stone, datum.y);

    // If pixel has been visited, check if regions is already marked as finder
    if matches!(img.get(s, y), Some(Pixel::Visited(..))) {
        let stone = img.get_region((s, y));

        // Exit if stone is already made a candidate from previous iterations
        if stone.is_finder {
            return None;
        }
    }

    let sx = r - (s - l) * 5 / 4;
    let seed = Point { x: sx as i32, y: datum.y as i32 };
    let pattern = [1.0, 1.0, 3.0, 1.0, 1.0];
    let buf = &scn.buffer;
    let thresh = (buf[0] + buf[1] + buf[3] + buf[4]) as f64 / 4.0;
    let max_run = (r - l) * 2; // Setting a loose upper limit on the run

    // Verify 1:1:3:1:1 pattern along Y axis
    if !verify_pattern::<Y>(img, &seed, &pattern, thresh, max_run) {
        return None;
    };

    let stone = img.get_region((s, y)).clone();
    let ring = img.get_region((r, y)).clone();

    // Check if left and right pts are not connected through same region
    // The id in Pixel::Visited makes the pixels unique
    if img.get(l, y) != img.get(r, y) {
        return None;
    }

    // False if ring & stone are connected, or if ring to stone area is outside limits
    let ratio = stone.area * 100 / ring.area;
    if img.get(r, y) == img.get(s, y) || ratio <= 10 || 70 <= ratio {
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
        ECLevel, MaskPattern, Palette, QRBuilder, Version,
    };

    use super::locate_finders;

    #[test]
    fn test_locate_finder() {
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

        let corners = [
            [[40, 40], [109, 40], [109, 109], [40, 109]],
            [[300, 109], [300, 40], [369, 109], [369, 40]],
            [[40, 369], [40, 300], [109, 300], [109, 369]],
        ];
        let centres = [[75, 75], [335, 75], [75, 335]];
        let mut bin_img = BinaryImage::binarize(&img);
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

pub fn group_finders(img: &BinaryImage, finders: &[Point]) -> Vec<FinderGroup> {
    // Store all possible combinations of finders
    let mut all_groups: Vec<FinderGroup> = Vec::new();
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

                // Angle of c2-c1-c3
                let angle = angle(f2, f1, f3);
                let angle_score = ((angle / right_angle) - 1.0).abs();
                if angle_score > 0.5 {
                    continue;
                }

                let score = symmetry_score + angle_score;

                // Create and push group into groups
                let finders = [*f3, *f1, *f2];
                let group = FinderGroup { finders, score };
                all_groups.push(group);
            }
        }
    }

    all_groups.sort_unstable_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

    all_groups
}

// Angle between AB & BC in radians
fn angle(a: &Point, b: &Point, c: &Point) -> f64 {
    let ab = ((a.x - b.x) as f64, (a.y - b.y) as f64);
    let cb = ((c.x - b.x) as f64, (c.y - b.y) as f64);

    let dot = ab.0 * cb.0 + ab.1 * cb.1;
    let mag_ab = (ab.0.powi(2) + ab.1.powi(2)).sqrt();
    let mag_cb = (cb.0.powi(2) + cb.1.powi(2)).sqrt();

    if mag_ab == 0.0 || mag_cb == 0.0 {
        return 0.0;
    }

    let cos_theta = (dot / (mag_ab * mag_cb)).clamp(-1.0, 1.0);

    cos_theta.acos()
}

fn estimate_mod_count(c1: &Point, m1: &Point, c2: &Point, m2: &Point) -> f64 {
    let d1 = c1.dist_sq(m1);
    let d2 = c2.dist_sq(m2);

    let avg_d = ((d1 + d2) / 2) as f64;
    let d12 = c1.dist_sq(c2) as f64;

    (d12 * 9.0 / avg_d).sqrt()
}

#[cfg(test)]
mod group_finders_tests {

    use crate::{reader::binarize::BinaryImage, ECLevel, MaskPattern, Palette, QRBuilder, Version};

    use super::{group_finders, locate_finders};

    #[test]
    fn test_group_finder() {
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

        let centres = [(75, 75), (335, 75), (75, 335)];

        let mut img = BinaryImage::binarize(&img);
        let finders = locate_finders(&mut img);
        let group = group_finders(&img, &finders);
        assert!(!group.is_empty(), "No group found");
        for f in group[0].finders.iter() {
            let c = (f.x, f.y);
            assert!(centres.contains(&c))
        }
    }
}
