use std::collections::HashSet;

use crate::{metadata::Color, reader::utils::geometry::BresenhamLine};

use super::{
    binarize::BinaryImage,
    utils::{
        geometry::{Axis, Point, X, Y},
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

    let stone = img.get_region((s, y))?;

    // Exit if stone is already made a candidate from previous iterations
    if stone.is_finder {
        return None;
    }

    let stone = stone.clone();

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

    let ring = img.get_region((r, y))?.clone();

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

    img.get_region((r, y)).unwrap().is_finder = true;
    img.get_region((s, y)).unwrap().is_finder = true;

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
    pub align: Point,        // Centre of provisional alignment pattern
    pub mids: [Point; 6],    // [BLR, BLU, TLB, TLR, TRL, TRD]. Mid pts of edges
    pub size: u32,           // Grid size of potential qr
    pub score: f64,          // Timing pattern score + Estimate mod count score
}

impl FinderGroup {
    #[cfg(test)]
    pub fn highlight(&self, img: &mut RgbImage) {
        for f in self.finders.iter() {
            f.highlight(img);
        }
        self.align.highlight(img);
        for m in self.mids.iter() {
            m.highlight(img);
        }
    }
}

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
// ************m13*************              ************m24*************
// ****************************              ****************************
//
//
//
// ****************************
// ************m31*************
// ****************************
// ****                   *****
// ****                   *****
// ****                   *****
// ****    ************   *****
// ****    *****c3*****   *m34*                           c4
// ****    ************   *****
// ****                   *****
// ****                   *****
// ****                   *****
// ****************************
// ****************************
// ****************************
pub fn group_finders(img: &BinaryImage, finders: &[Point]) -> Vec<FinderGroup> {
    // Store all possible combinations of finders
    let mut all_groups: Vec<FinderGroup> = Vec::new();
    let angle_threshold = 50f64.to_radians();

    for (i1, f1) in finders.iter().enumerate() {
        for (i2, f2) in finders.iter().enumerate() {
            if i2 == i1 {
                continue;
            }

            let m12 = match find_edge_mid(img, f1, f2) {
                Some(pt) => pt,
                None => continue,
            };
            let m21 = match find_edge_mid(img, f2, f1) {
                Some(pt) => pt,
                None => continue,
            };

            for (i3, f3) in finders.iter().enumerate() {
                if i3 <= i2 || i3 == i1 {
                    continue;
                }

                if angle(f2, f1, f3) < angle_threshold {
                    continue;
                }

                let m13 = match find_edge_mid(img, f1, f3) {
                    Some(pt) => pt,
                    None => continue,
                };
                let m31 = match find_edge_mid(img, f3, f1) {
                    Some(pt) => pt,
                    None => continue,
                };

                // Compute provisional location of alignment centre (c4)
                let dx = f2.x - f1.x;
                let dy = f2.y - f1.y;
                let c4 = Point { x: f3.x + dx, y: f3.y + dy };

                // Skip if intersection pt is outside the image
                if c4.x < 0 || c4.x as u32 >= img.w || c4.y < 0 || c4.y as u32 >= img.h {
                    continue;
                }

                let m24 = match find_edge_mid(img, f2, &c4) {
                    Some(pt) => pt,
                    None => continue,
                };
                let m34 = match find_edge_mid(img, f3, &c4) {
                    Some(pt) => pt,
                    None => continue,
                };

                // Calculate score
                let t12 = measure_timing_patterns(img, &m13, &m24);
                let t13 = measure_timing_patterns(img, &m12, &m34);

                // Closeness of the 2 timing patterns
                let symmetry_score = ((t12 as f64 / t13 as f64) - 1.0).abs();

                // Skip if one timing pattern is more than twice as long as the other
                if symmetry_score > 1.0 {
                    continue;
                }

                // Estimate module count from c1 to c2
                let est_mod_count12 = estimate_mod_count(f1, &m12, f2, &m21);
                let mod_score12 = ((est_mod_count12 / (t12 + 6) as f64) - 1.0).abs();

                // Skip if one is more than twice as long as the other
                if mod_score12 > 1.0 {
                    continue;
                }

                // Estimate module count from c1 to c3
                let est_mod_count13 = estimate_mod_count(f1, &m13, f3, &m31);
                let mod_score13 = ((est_mod_count13 / (t13 + 6) as f64) - 1.0).abs();

                // Skip if one is more than twice as long as the other
                if mod_score13 > 1.0 {
                    continue;
                }

                let score = symmetry_score + mod_score12 + mod_score13;

                // Create and push group into groups
                let finders = [*f3, *f1, *f2];
                let mids = [m34, m31, m13, m12, m21, m24];
                let size = std::cmp::max(t12, t13) + 13;
                let ver = (size as f64 - 15.0).floor() as u32 / 4;
                let size = ver * 4 + 17;
                let group = FinderGroup { finders, align: c4, mids, size, score };
                all_groups.push(group);
            }
        }
    }

    all_groups.sort_unstable_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

    // If a finder is in multiple groups, reject all groups except one with highest score
    let mut res = Vec::with_capacity(finders.len() / 3);
    let mut is_grouped = HashSet::with_capacity(finders.len());

    for g in all_groups {
        if g.finders.iter().all(|f| !is_grouped.contains(f)) {
            is_grouped.extend(g.finders.iter().cloned());
            res.push(g);
        }
    }

    res
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

fn find_edge_mid(img: &BinaryImage, from: &Point, to: &Point) -> Option<Point> {
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
    let px = img.get_at_point(from).unwrap();
    let mut last = Color::from(*px);
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let px = img.get_at_point(&p).unwrap();
        let color = Color::from(*px);

        if color != last {
            flips += 1;
            last = color;
            if flips == 3 {
                let idx = buffer.len() * 6 / 7;
                let mid = buffer[idx];
                // let mid = buffer[buffer.len() / 2];
                return Some(mid);
            }
        }

        buffer.push(p);
    }

    None
}

pub fn measure_timing_patterns(img: &BinaryImage, from: &Point, to: &Point) -> u32 {
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
    let mut transitions = [0, 0, 0];
    let px = img.get_at_point(from).unwrap();
    let mut last = Color::from(*px).to_bits();
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let px = img.get_at_point(&p).unwrap();
        let color = Color::from(*px).to_bits();
        for i in 0..3 {
            if color[i] != last[i] {
                transitions[i] += 1;
                last[i] = color[i];
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
