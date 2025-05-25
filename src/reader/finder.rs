use crate::{metadata::Color, reader::utils::geometry::BresenhamLine};

use super::{
    binarize::{BinaryImage, Pixel, Region},
    utils::{
        accumulate::CenterLocator,
        geometry::{Axis, Line, Point, Slope, X, Y},
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

// Finder
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Finder {
    pub id: usize,
    pub center: Point,
}

impl Finder {
    #[cfg(test)]
    fn highlight(&self, img: &mut RgbImage) {
        self.center.highlight(img);
    }
}

// Line scanner to detect finder line
//------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct LineScanner {
    buffer: [u32; 6],    // Run length of each transition
    prev: Option<Color>, // Last observed color
    flips: u32,          // Count of color changes
    pos: u32,            // Current position
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
        let white = Color::White;
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
pub fn locate_finders(img: &mut BinaryImage) -> Vec<Finder> {
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

            if crosscheck_vertical(img, &datum) && validate_regions(img, &datum) {
                let f = construct_finder(img, &datum, finders.len());
                finders.push(f);
            }
        }

        // Covers an edge case where a qr is at the right side edge of the image
        if let Some(datum) = scanner.advance(Color::White) {
            if crosscheck_vertical(img, &datum) && validate_regions(img, &datum) {
                let f = construct_finder(img, &datum, finders.len());
                finders.push(f);
            }
        }

        scanner.reset(y + 1);
    }

    finders
}

/// Checks if the vertical run along finder line center satisfies the 1:1:3:1:1 ratio
fn crosscheck_vertical(img: &BinaryImage, datum: &DatumLine) -> bool {
    let h = img.h;
    let cx = datum.right - (datum.stone - datum.left) * 5 / 4;
    let cy = datum.y;

    if cy == 0 {
        return false;
    }

    let max_run = (datum.right - datum.left) * 2; // Setting a loose max limit on each run
    let mut run_len = [0; 5];
    run_len[2] = 1;

    // Count upwards
    let mut pos = cy - 1;
    let mut flips = 2;
    let mut initial = Color::from(img.get(cx, cy).unwrap());
    while run_len[flips] <= max_run {
        let color = Color::from(img.get(cx, pos).unwrap());
        if initial != color {
            initial = color;
            if flips == 0 {
                break;
            }
            flips -= 1;
        }
        run_len[flips] += 1;

        if pos == 0 {
            break;
        }
        pos -= 1;
    }

    // Count downwards
    let mut pos = cy + 1;
    let mut flips = 2;
    let mut initial = Color::from(img.get(cx, cy).unwrap());
    while pos < h && run_len[flips] <= max_run {
        let color = Color::from(img.get(cx, pos).unwrap());
        if initial != color {
            initial = color;
            if flips == 4 {
                break;
            }
            flips += 1;
        }
        run_len[flips] += 1;
        pos += 1;
    }

    // Verify 1:1:3:1:1 ratio
    let avg = (run_len.iter().sum::<u32>() as f64) / 7.0;
    let tol = avg * 3.0 / 4.0;

    let ratio: [f64; 5] = [1.0, 1.0, 3.0, 1.0, 1.0];
    for (i, r) in ratio.iter().enumerate() {
        let rl = run_len[i] as f64;
        if rl < r * avg - tol || rl > r * avg + tol {
            return false;
        }
    }

    true
}

/// Sweeps stone and ring regions from datum line and checks:
/// Stone area is roughly 37.5% of ring area
/// Stone and ring areas are not connected
/// Left and right points, of the row, lying inside the ring are connected
fn validate_regions(img: &mut BinaryImage, datum: &DatumLine) -> bool {
    let (l, r, s, y) = (datum.left, datum.right, datum.stone, datum.y);
    let ring = img.get_region((r, y));
    let stone = img.get_region((s, y));

    if img.get(l, y) != img.get(r, y) {
        return false;
    }

    if let (
        Some(Region { src: r_src, area: r_area, .. }),
        Some(Region { src: s_src, area: s_area, .. }),
    ) = (ring, stone)
    {
        let ratio = s_area * 100 / r_area;
        let r_color = img.get(r_src.0, r_src.1);
        let s_color = img.get(s_src.0, s_src.1);
        // r_color != s_color && (20 < ratio && ratio < 50)
        r_color != s_color && (10 < ratio && ratio < 70)
    } else {
        false
    }
}

fn construct_finder(img: &mut BinaryImage, datum: &DatumLine, id: usize) -> Finder {
    let (stone, right, y) = (datum.stone, datum.right, datum.y);
    let color = Color::from(img.get(stone, y).unwrap());
    let ref_pt = Point { x: stone as i32, y: y as i32 };

    // Locating center of finder
    let cl = CenterLocator::new();
    let to = Pixel::Candidate(color);
    let cl = img.fill_and_accumulate((stone, y), to, cl);
    let center = cl.get_center();

    // Mark the ring as candidate
    let color = Color::from(img.get(right, y).unwrap());
    let to = Pixel::Candidate(color);
    let _ = img.fill_and_accumulate((right, y), to, |_| ());

    Finder { id, center }
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
        let centers = [[75, 75], [335, 75], [75, 335]];
        let mut img = BinaryImage::prepare(&img);
        let finders = locate_finders(&mut img);
        for (i, f) in finders.iter().enumerate() {
            let cent_pt = Point { x: centers[i][0], y: centers[i][1] };
            assert_eq!(f.center, cent_pt, "Finder center doesn't match");
        }
    }
}

// Groups finders in 3, which form potential symbols
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct FinderGroup {
    pub finders: [Finder; 3], // [BL, TL, TR]
    pub mids: [Point; 6],     // [BLR, BLU, TLB, TLR, TRL, TRD]. Mid pts of edges
    pub size: u32,            // Grid size of potential qr
    pub score: f64,           // Timing pattern score + Estimate mod count score
}

impl FinderGroup {
    #[cfg(test)]
    pub fn highlight(&self, img: &mut RgbImage) {
        for f in self.finders.iter() {
            f.highlight(img);
        }
        for m in self.mids.iter() {
            m.highlight(img);
        }
    }
}

// Below diagram shows the location of all centers and edge mid points
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
pub fn group_finders(img: &BinaryImage, finders: &[Finder]) -> Vec<FinderGroup> {
    let mut groups: Vec<FinderGroup> = Vec::new();
    let len = finders.len();
    let angle_threshold = 50f64.to_radians();

    for (i1, f1) in finders.iter().enumerate() {
        for (i2, f2) in finders.iter().enumerate() {
            if i2 == i1 {
                continue;
            }

            let m12 = match find_edge_mid(img, &f1.center, &f2.center) {
                Some(pt) => pt,
                None => continue,
            };
            let m21 = match find_edge_mid(img, &f2.center, &f1.center) {
                Some(pt) => pt,
                None => continue,
            };
            let s2 = Slope::new(&f1.center, &f2.center);

            for (i3, f3) in finders.iter().enumerate() {
                if i3 <= i2 || i3 == i1 {
                    continue;
                }

                if angle(f2.center, f1.center, f3.center) < angle_threshold {
                    continue;
                }

                let m13 = match find_edge_mid(img, &f1.center, &f3.center) {
                    Some(pt) => pt,
                    None => continue,
                };
                let m31 = match find_edge_mid(img, &f3.center, &f1.center) {
                    Some(pt) => pt,
                    None => continue,
                };
                let s3 = Slope::new(&f1.center, &f3.center);

                let l24 = Line::from_point_slope(&f2.center, &s3);
                let l34 = Line::from_point_slope(&f3.center, &s2);
                let c4 = match l24.intersection(&l34) {
                    Some(pt) => pt,
                    None => continue,
                };

                // Skip if intersection pt is outside the image
                let Point { x: x4, y: y4 } = c4;
                if x4 < 0 || x4 as u32 >= img.w || y4 < 0 || y4 as u32 > img.h {
                    continue;
                }

                let m24 = match find_edge_mid(img, &f2.center, &c4) {
                    Some(pt) => pt,
                    None => continue,
                };
                let m34 = match find_edge_mid(img, &f3.center, &c4) {
                    Some(pt) => pt,
                    None => continue,
                };

                let t12 = measure_timing_patterns(img, &m13, &m24);
                let t13 = measure_timing_patterns(img, &m12, &m34);

                // Calculate score
                let symmetry_score = ((t12 as f64 / t13 as f64) - 1.0).abs();

                let est_mod_count12 = estimate_mod_count(&f1.center, &m12, &f2.center, &m21);
                let mod_score12 = ((est_mod_count12 / (t12 + 6) as f64) - 1.0).abs();

                let est_mod_count13 = estimate_mod_count(&f1.center, &m13, &f3.center, &m31);
                let mod_score13 = ((est_mod_count13 / (t13 + 6) as f64) - 1.0).abs();

                let score = symmetry_score + mod_score12 + mod_score13;

                // Create and push group into groups
                let finders = [f3.clone(), f1.clone(), f2.clone()];
                let mids = [m34, m31, m13, m12, m21, m24];
                let size = std::cmp::max(t12, t13) + 13;
                let ver = (size as f64 - 15.0).floor() as u32 / 4;
                let size = ver * 4 + 17;
                let group = FinderGroup { finders, mids, size, score };
                groups.push(group);
            }
        }
    }

    groups.sort_unstable_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

    groups
}

// Angle between AB & BC in radians
fn angle(a: Point, b: Point, c: Point) -> f64 {
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
    let d1 = c1.squared_distance(m1);
    let d2 = c2.squared_distance(m2);

    let avg_d = ((d1 + d2) / 2) as f64;
    let d12 = c1.squared_distance(c2) as f64;

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

        let centers = [(75, 75), (335, 75), (75, 335)];

        let mut img = BinaryImage::prepare(&img);
        let finders = locate_finders(&mut img);
        let group = group_finders(&img, &finders);
        assert!(!group.is_empty(), "No group found");
        for f in group[0].finders.iter() {
            let c = (f.center.x, f.center.y);
            assert!(centers.contains(&c))
        }
    }
}
