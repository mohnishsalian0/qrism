use std::{cmp::Ordering, sync::Arc};

use super::{
    binarize::BinaryImage,
    finder::FinderGroup,
    utils::{
        geometry::{Axis, BresenhamLine, Point, Slope},
        homography::Homography,
    },
};
use crate::{
    codec::decode as codec_decode,
    ec::{rectify_info, Block},
    metadata::{
        parse_format_info_qr, Color, Metadata, FORMAT_ERROR_CAPACITY, FORMAT_INFOS_QR,
        FORMAT_INFO_COORDS_QR_MAIN, FORMAT_INFO_COORDS_QR_SIDE, FORMAT_MASK, VERSION_ERROR_BIT_LEN,
        VERSION_ERROR_CAPACITY, VERSION_INFOS, VERSION_INFO_COORDS_BL, VERSION_INFO_COORDS_TR,
    },
    reader::utils::{
        geometry::{X, Y},
        verify_alignment_pattern,
    },
    utils::{BitArray, BitStream, EncRegionIter, QRError, QRResult},
    ECLevel, MaskPattern, Version,
};

#[cfg(test)]
use image::RgbImage;

// Finder local frame
//
// An affine frame anchored at a finder centre, used to read the version info block beside the
// BL & TR finders. It exists because the symbol homography isn't available yet at that point:
// the homography needs the version, so the version has to be read first.
//
// The two basis vectors run from the finder centre to the midpoints of its black ring which sit
// exactly 3 modules out
// So the frame picks up the symbol's local rotation, scale and skew straight from the finder,
// without needing the grid. Being affine it carries no perspective term and drifts the further
// it extends, which is acceptable over the few modules between a finder and its version block.
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct LocalFrame {
    o: Point,
    x: Slope,
    y: Slope,
}

impl LocalFrame {
    // `mx` and `my` are the black ring midpoints that define the +x and +y directions; both
    // axes therefore point inward, into the symbol.
    fn new(o: &Point, mx: &Point, my: &Point) -> Self {
        Self { o: *o, x: Slope::new(o, mx), y: Slope::new(o, my) }
    }

    // Maps module coordinates, relative to the finder centre, onto image pixels. The basis
    // vectors span 3 modules each, hence scaling the offsets by 1/3.
    fn map(&self, x: f64, y: f64) -> Point {
        let (sx, sy) = (x / 3.0, y / 3.0);
        Point {
            x: (self.o.x as f64 + sx * self.x.dx as f64 + sy * self.y.dx as f64).round() as i32,
            y: (self.o.y as f64 + sx * self.x.dy as f64 + sy * self.y.dy as f64).round() as i32,
        }
    }
}

// Locates symbol based on 3 finder centres, their edge points & provisional grid size
//------------------------------------------------------------------------------

#[derive(Debug)]
pub struct SymbolLocation {
    h: Homography,
    _anchors: [Point; 4],
    ver: Version,
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
        let mut align = Point { x: c0.x + dx, y: c0.y + dy };

        // Skip if intersection pt is outside the image
        if align.x < 0 || align.x as u32 >= img.w || align.y < 0 || align.y as u32 >= img.h {
            return None;
        }

        // Hypotenuse slope
        let mut hm = Slope { dx: c2.x - c0.x, dy: c2.y - c0.y };

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
            find_edge_mid(img, &c0, &align)?,
            find_edge_mid(img, &c0, &c1)?,
            find_edge_mid(img, &c1, &c0)?,
            find_edge_mid(img, &c1, &c2)?,
            find_edge_mid(img, &c2, &c1)?,
            find_edge_mid(img, &c2, &align)?,
        ];

        let size = verify_symbol_size(img, &group.finders, &mids)?;

        let ver = Version::from_grid_size(size as usize)?;

        let app = ver.alignment_pattern();

        // For versions greater than 1, a more robust algorithm to locate align centre.
        // First, locate provisional centre from mid 1 with distance of c1 from mid 4.
        // Spiral out of provisional align pt to identify potential pt. Then compare the area of
        // black region with estimate module size to confirm alignment stone. Finally, locate the
        // centre of the stone.
        if *ver != 1 {
            align = locate_alignment_pattern(img, &group.finders, &mids, &ver)?;
        }

        let h = setup_homography(img, group, align, ver)?;

        let _anchors = [c1, c2, align, c0];

        Some(Self { h, _anchors, ver })
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
        let blf = LocalFrame::new(c0, m03, m01);
        let trf = LocalFrame::new(c2, m23, m21);

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

    dbg!(size);

    Some(size)
}

// Snaps a centre-to-centre module count to the nearest valid symbol size (always 1 mod 4),
// returning that size and the signed distance from the estimate to it.
fn nearest_valid_size(mod_count: f64) -> (i32, i32) {
    let est = mod_count.round() as i32 + 7;
    let err = 1 - (est % 4);
    (est + err, err)
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
    use crate::binarize::BinaryImage;
    use crate::metadata::{Color, ECLevel, Version};
    use crate::reader::utils::geometry::Point;
    use crate::symbol::{read_version_info, LocalFrame};
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
            let blf = LocalFrame::new(&p(c, far), &p(c + 3.0 * k, far), &p(c, far - 3.0 * k));
            let (blv, blerr) = read_version_info(&img, blf).expect("Version read failed");
            assert_eq!(blv, v);
            assert_eq!(blerr, 0);

            // TR: centre, then m23 (+3 modules down) and m21 (3 modules left)
            let trf = LocalFrame::new(&p(far, c), &p(far, c + 3.0 * k), &p(far - 3.0 * k, c));
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
            let blf = LocalFrame::new(&p(c, far), &p(c + 3.0 * k, far), &p(c, far - 3.0 * k));
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
        let blf = LocalFrame::new(&p(c, far), &p(c + 3.0 * k, far), &p(c, far - 3.0 * k));
        let blv = read_version_info(&img, blf);
        assert!(blv.is_none());
    }
}

// Symbol
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Symbol {
    img: Arc<BinaryImage>,
    h: Homography,
    _anchors: [Point; 4],
    pub ver: Version,
}

impl Symbol {
    pub fn new(img: Arc<BinaryImage>, sym_loc: SymbolLocation) -> Self {
        let SymbolLocation { h, _anchors, ver } = sym_loc;
        Self { img, h, _anchors, ver }
    }

    pub fn decode(&mut self) -> QRResult<(Metadata, String)> {
        let (ecl, mask) = self.read_format_info()?;
        let ver = self.ver;
        let hi_cap = self.read_capacity_info()?;

        let pld = self.extract_payload(&mask)?;

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let mut enc = BitStream::new(pld.len() << 3);
        let chan_cap = ver.channel_codewords();

        // Chunking channel data, deinterleaving & rectifying payload
        for c in pld.data().chunks_exact(chan_cap) {
            let mut blocks = deinterleave(c, blk_info, ec_len);
            for b in blocks.iter_mut() {
                let rectified = b.rectify()?;
                enc.extend(rectified);
            }
        }

        let msg = codec_decode(&mut enc, ver, ecl, hi_cap)?;
        let meta = Metadata::new(Some(ver), Some(ecl), Some(mask));

        Ok((meta, msg))
    }

    pub fn get(&self, x: i32, y: i32) -> Option<Color> {
        let (xp, yp) = self.wrap_coord(x, y);
        let pt = self.map(xp as f64 + 0.5, yp as f64 + 0.5).ok()?;
        self.img.get_at_point(&pt)
    }

    fn wrap_coord(&self, x: i32, y: i32) -> (i32, i32) {
        let w = self.ver.width() as i32;
        debug_assert!(-w <= x && x < w, "x shouldn't be greater than or equal to w");
        debug_assert!(-w <= y && y < w, "y shouldn't be greater than or equal to w");

        let x = if x < 0 { x + w } else { x };
        let y = if y < 0 { y + w } else { y };
        (x, y)
    }

    #[inline]
    pub fn map(&self, x: f64, y: f64) -> QRResult<Point> {
        self.h.map(x, y)
    }

    #[cfg(feature = "benchmark")]
    #[inline]
    pub fn raw_map(&self, x: f64, y: f64) -> QRResult<(f64, f64)> {
        self.h.raw_map(x, y)
    }

    #[cfg(test)]
    pub fn highlight(&self, img: &mut RgbImage) {
        use super::utils::geometry::{BresenhamLine, X, Y};
        use crate::reader::utils::rnd_rgb;

        let color = rnd_rgb();

        for p in self._anchors.iter() {
            p.highlight(img, color);
        }

        let (w, h) = img.dimensions();
        let sz = self.ver.width() as f64;
        let tl = self.map(0.0, 0.0).unwrap();
        let tr = self.map(sz, 0.0).unwrap();
        let br = self.map(sz, sz).unwrap();
        let bl = self.map(0.0, sz).unwrap();
        let bounds = [tl, tr, br, bl];

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

fn locate_alignment_pattern(
    img: &mut BinaryImage,
    finders: &[Point; 3],
    mids: &[Point; 6],
    ver: &Version,
) -> Option<Point> {
    let (w, h) = (img.w, img.h);
    let [c0, c1, c2] = finders;
    let pattern = [1.0, 1.0, 1.0];

    // Locate provisional alignment centre
    let dx = mids[4].x - c1.x;
    let dy = mids[4].y - c1.y;
    let mut seed = Point { x: mids[1].x + dx, y: mids[1].y + dy };

    // Calculate estimate width of module
    let hor_w = c0.dist_sq(&mids[0]);
    let ver_w = c2.dist_sq(&mids[5]);
    let mod_w = ((hor_w + ver_w) as f64 / 2.0).sqrt() / 3.0;
    let mod_w_i32 = mod_w as i32;

    // Calculate estimate area of module by taking cross product of vectors
    let v0 = Slope::new(c0, &mids[0]);
    let v1 = Slope::new(c2, &mids[5]);
    let area = v0.cross(&v1).unsigned_abs() / 9;
    let threshold = area * 2;

    // Directional increment for x & y: [right, down, left, up]
    const DX: [i32; 4] = [1, 0, -1, 0];
    const DY: [i32; 4] = [0, -1, 0, 1];

    // Spiral outward to find stone
    let mut dir = 0;
    let mut run_len = 1;
    let radius_increment = ver.alignment_pattern().len() as i32;
    let search_radius = mod_w_i32 * (radius_increment + 14);
    let mut rejected = Vec::with_capacity(100);

    while run_len < search_radius {
        for _ in 0..run_len {
            let x = seed.x as u32;
            let y = seed.y as u32;

            if let Some(color) = img.get_at_point(&seed) {
                if x < w && y < h && color == Color::Black {
                    let reg = img.get_region((x, y));
                    let (reg_centre, reg_area) = (reg.centre, reg.area);

                    if !rejected.contains(&reg_centre) {
                        // Check if region area is roughly equal to mod area with 100% tolerance
                        // and crosscheck 1:1:1 ratio horizontally and vertically
                        if reg_area <= threshold
                            && verify_alignment_pattern::<X>(
                                img,
                                &reg_centre,
                                &pattern,
                                mod_w,
                                threshold,
                            )
                            && verify_alignment_pattern::<Y>(
                                img,
                                &reg_centre,
                                &pattern,
                                mod_w,
                                threshold,
                            )
                        {
                            return Some(reg_centre);
                        } else {
                            rejected.push(reg_centre);
                        }
                    }
                }
            }

            seed.x += DX[dir];
            seed.y += DY[dir];
        }

        // Cycle direction
        dir = (dir + 1) & 3;
        if dir & 1 == 0 {
            run_len += 1;
        }
    }

    None
}

fn setup_homography(
    img: &BinaryImage,
    group: &FinderGroup,
    align_centre: Point,
    ver: Version,
) -> Option<Homography> {
    let size = ver.width() as f64;
    let br_off = if *ver == 1 { 3.5 } else { 6.5 };
    let src = [(3.5, 3.5), (size - 3.5, 3.5), (size - br_off, size - br_off), (3.5, size - 3.5)];

    let c0 = (group.finders[0].x as f64, group.finders[0].y as f64);
    let c1 = (group.finders[1].x as f64, group.finders[1].y as f64);
    let c2 = (group.finders[2].x as f64, group.finders[2].y as f64);
    let ca = (align_centre.x as f64, align_centre.y as f64);
    let dst = [c1, c2, ca, c0];

    let initial_h = Homography::compute(src, dst).ok()?;

    jiggle_homography(img, initial_h, ver)
}

// Adjust the homography slightly to refine projection of qr
fn jiggle_homography(img: &BinaryImage, mut h: Homography, ver: Version) -> Option<Homography> {
    let mut best = symbol_fitness(img, &h, ver);

    // Create an adjustment matrix by scaling the homography
    let mut adjustments = h.0.map(|x| x * 0.04);

    for _pass in 0..6 {
        for i in 0..8 {
            let old = h[i];
            for j in 0..2 {
                let step = adjustments[i];
                h[i] = if j & 1 == 0 { old - step } else { old + step };

                let test = symbol_fitness(img, &h, ver);
                if test > best {
                    best = test
                } else {
                    h[i] = old
                }
            }
        }

        // Halve all adjustment steps
        adjustments = adjustments.map(|x| x * 0.5);
    }
    let max_score = max_fitness_score(ver);

    // 60% tolerance
    if best >= max_score * 4 / 10 {
        Some(h)
    } else {
        None
    }
}

fn symbol_fitness(img: &BinaryImage, h: &Homography, ver: Version) -> i32 {
    let mut score = 0;
    let grid_size = ver.width() as i32;

    // Score timing patterns
    for i in 7..grid_size - 7 {
        let flip = if i & 1 == 0 { -1 } else { 1 };
        score += cell_fitness(img, h, i, 6) * flip;
        score += cell_fitness(img, h, 6, i) * flip;
    }

    // Score finders
    score += finder_fitness(img, h, 0, 0);
    score += finder_fitness(img, h, grid_size - 7, 0);
    score += finder_fitness(img, h, 0, grid_size - 7);

    // Score alignment patterns
    for (x, y) in alignment_centres(ver) {
        score += alignment_fitness(img, h, x, y);
    }

    score
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

fn finder_fitness(img: &BinaryImage, h: &Homography, x: i32, y: i32) -> i32 {
    let (x, y) = (x + 3, y + 3);
    cell_fitness(img, h, x, y) + ring_fitness(img, h, x, y, 1) - ring_fitness(img, h, x, y, 2)
        + ring_fitness(img, h, x, y, 3)
}

fn alignment_fitness(img: &BinaryImage, h: &Homography, x: i32, y: i32) -> i32 {
    cell_fitness(img, h, x, y) - ring_fitness(img, h, x, y, 1) + ring_fitness(img, h, x, y, 2)
}

fn ring_fitness(img: &BinaryImage, h: &Homography, cx: i32, cy: i32, r: i32) -> i32 {
    let mut score = 0;

    for i in 0..r * 2 {
        score += cell_fitness(img, h, cx - r + i, cy - r);
        score += cell_fitness(img, h, cx - r, cy + r - i);
        score += cell_fitness(img, h, cx + r, cy - r + i);
        score += cell_fitness(img, h, cx + r - i, cy + r);
    }

    score
}

fn cell_fitness(img: &BinaryImage, hm: &Homography, x: i32, y: i32) -> i32 {
    const OFFSETS: [f64; 3] = [0.3, 0.5, 0.7];
    let white = Color::White;
    let mut score = 0;

    for dy in OFFSETS.iter() {
        for dx in OFFSETS.iter() {
            let pt = match hm.map(x as f64 + dx, y as f64 + dy) {
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

#[cfg(test)]
mod symbol_tests {

    use crate::{
        reader::{
            binarize::BinaryImage,
            finder::{group_finders, locate_finders},
            locate_symbols,
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
        let exp_anchors = [(75, 75), (335, 75), (305, 305), (75, 335)];

        let mut img = BinaryImage::prepare(&img);
        let finders = locate_finders(&mut img);
        let groups = group_finders(&finders);
        let symbols = locate_symbols(&mut img, groups);
        for b in symbols[0]._anchors {
            assert!(exp_anchors.contains(&(b.x, b.y)), "Symbol not within bounds");
        }
    }
}

// Read format, version & capacity info
//------------------------------------------------------------------------------

impl Symbol {
    pub fn read_format_info(&self) -> QRResult<(ECLevel, MaskPattern)> {
        // Parse main format area
        if let Some(main) = self.get_number(&FORMAT_INFO_COORDS_QR_MAIN) {
            if let Ok((format, _)) = rectify_info(main, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY) {
                let format = format ^ FORMAT_MASK;
                let (ecl, mask) = parse_format_info_qr(format);
                return Ok((ecl, mask));
            }
        }

        // Parse side format area
        if let Some(side) = self.get_number(&FORMAT_INFO_COORDS_QR_SIDE) {
            if let Ok((format, _)) = rectify_info(side, &FORMAT_INFOS_QR, FORMAT_ERROR_CAPACITY) {
                let format = format ^ FORMAT_MASK;
                let (ecl, mask) = parse_format_info_qr(format);
                return Ok((ecl, mask));
            }
        }

        Err(QRError::InvalidFormatInfo)
    }

    pub fn read_version_info(&self) -> QRResult<Version> {
        // Parse bottom left version area
        if let Some(bl) = self.get_number(&VERSION_INFO_COORDS_BL) {
            if let Ok((v, _)) = rectify_info(bl, &VERSION_INFOS, VERSION_ERROR_CAPACITY) {
                return Ok(Version::Normal(v as usize >> VERSION_ERROR_BIT_LEN));
            }
        }

        // Parse top right version area
        if let Some(tr) = self.get_number(&VERSION_INFO_COORDS_TR) {
            if let Ok((v, _)) = rectify_info(tr, &VERSION_INFOS, VERSION_ERROR_CAPACITY) {
                return Ok(Version::Normal(v as usize >> VERSION_ERROR_BIT_LEN));
            }
        }

        Err(QRError::InvalidVersionInfo)
    }

    pub fn read_capacity_info(&self) -> QRResult<bool> {
        if let Some(color) = self.get(8, -8) {
            if color == Color::Black {
                return Ok(false); // Standard capacity
            } else {
                return Ok(true); // High capacity
            }
        }

        Err(QRError::InvalidCapacityInfo)
    }

    pub fn get_number(&self, coords: &[(i32, i32)]) -> Option<u32> {
        let mut num = 0;
        for &(x, y) in coords {
            let color = self.get(x, y)?;
            let bit = (color != Color::White) as u32;
            num = (num << 1) | bit;
        }
        Some(num)
    }
}

#[cfg(test)]
mod symbol_infos_tests {

    use crate::{
        metadata::Color, reader::detect_qr, ECLevel, MaskPattern, Module, QRBuilder, Version,
    };

    #[test]
    fn test_read_format_info_clean() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let fmt_info = res.symbols()[0].read_format_info().expect("Failed to read format info");
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    fn test_read_format_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(1, 8, Module::Format(Color::White));
        qr.set(2, 8, Module::Format(Color::White));
        qr.set(4, 8, Module::Format(Color::Black));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let fmt_info = res.symbols()[0].read_format_info().expect("Failed to read format info");
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    fn test_read_format_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(1, 8, Module::Format(Color::White));
        qr.set(2, 8, Module::Format(Color::White));
        qr.set(3, 8, Module::Format(Color::Black));
        qr.set(4, 8, Module::Format(Color::Black));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let fmt_info = res.symbols()[0].read_format_info().expect("Failed to read format info");
        assert_eq!(fmt_info, (ecl, mask));
    }

    #[test]
    #[should_panic]
    fn test_read_format_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(2);
        let ecl = ECLevel::L;
        let mask = MaskPattern::new(1);

        let mut qr =
            QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).mask(mask).build().unwrap();
        qr.set(1, 8, Module::Format(Color::White));
        qr.set(2, 8, Module::Format(Color::White));
        qr.set(3, 8, Module::Format(Color::Black));
        qr.set(4, 8, Module::Format(Color::Black));
        qr.set(8, -2, Module::Format(Color::White));
        qr.set(8, -3, Module::Format(Color::White));
        qr.set(8, -4, Module::Format(Color::Black));
        qr.set(8, -5, Module::Format(Color::Black));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let _ = res.symbols()[0].read_format_info().expect("Failed to read format info");
    }

    #[test]
    fn test_read_version_info() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let scanned_ver = res.symbols()[0].read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    fn test_read_version_info_one_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(5, -9, Module::Format(Color::Black));
        qr.set(5, -10, Module::Format(Color::Black));
        qr.set(5, -11, Module::Format(Color::Black));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let scanned_ver = res.symbols()[0].read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    fn test_read_version_info_one_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(5, -9, Module::Format(Color::Black));
        qr.set(5, -10, Module::Format(Color::Black));
        qr.set(5, -11, Module::Format(Color::Black));
        qr.set(4, -9, Module::Format(Color::White));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let scanned_ver = res.symbols()[0].read_version_info().expect("Failed to read format info");
        assert_eq!(scanned_ver, ver);
    }

    #[test]
    #[should_panic]
    fn test_read_version_info_both_fully_corrupted() {
        let data = "Hello, world! 🌎";
        let ver = Version::Normal(7);
        let ecl = ECLevel::L;

        let mut qr = QRBuilder::new(data.as_bytes()).version(ver).ec_level(ecl).build().unwrap();
        qr.set(5, -9, Module::Format(Color::Black));
        qr.set(5, -10, Module::Format(Color::Black));
        qr.set(5, -11, Module::Format(Color::Black));
        qr.set(4, -9, Module::Format(Color::White));
        qr.set(-9, 5, Module::Format(Color::Black));
        qr.set(-10, 5, Module::Format(Color::Black));
        qr.set(-11, 5, Module::Format(Color::Black));
        qr.set(-9, 4, Module::Format(Color::White));
        let img = image::DynamicImage::ImageRgb8(qr.to_image(3));

        let mut res = detect_qr(&img);

        let _ = res.symbols()[0].read_version_info().expect("Failed to read format info");
    }
}

// Extracts encoded data codewords and error correction codewords
//------------------------------------------------------------------------------

impl Symbol {
    pub fn extract_payload(&self, mask: &MaskPattern) -> QRResult<BitArray> {
        let ver = self.ver;
        let mask_fn = mask.mask_functions();
        let chan_bits = ver.channel_codewords() << 3;
        let offsets = [2 * chan_bits, chan_bits, 0]; // B, G, R offsets
        let mut payload = BitArray::new(chan_bits * 3);
        let mut rgn_iter = EncRegionIter::new(ver);

        for (i, (x, y)) in rgn_iter.by_ref().take(chan_bits).enumerate() {
            let color = self.get(x, y).ok_or(QRError::PixelOutOfBounds)?;
            let rgb = color as u8;
            for (j, off) in offsets.iter().enumerate() {
                let mut bit = ((rgb >> j) & 1) == 1;
                if !mask_fn(x, y) {
                    bit = !bit;
                }
                payload.put(i + off, bit);
            }
        }

        debug_assert_eq!(rgn_iter.count(), self.ver.remainder_bits(), "Remainder bits don't match");

        Ok(payload)
    }
}

fn deinterleave(data: &[u8], blk_info: (usize, usize, usize, usize), ec_len: usize) -> Vec<Block> {
    // b1s = block1_size, b1c = block1_count
    let (b1s, b1c, b2s, b2c) = blk_info;

    let total_blks = b1c + b2c;
    let spl = b1s * total_blks;
    let data_sz = b1s * b1c + b2s * b2c;

    let mut dilvd = vec![Vec::with_capacity(b2s); total_blks];

    // Deinterleaving data
    data[..spl]
        .chunks(total_blks)
        .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| dilvd[i].push(*v)));
    if b2c > 0 {
        data[spl..data_sz]
            .chunks(b2c)
            .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| dilvd[b1c + i].push(*v)));
    }

    // Deinterleaving ecc
    data[data_sz..]
        .chunks(total_blks)
        .for_each(|ch| ch.iter().enumerate().for_each(|(i, v)| dilvd[i].push(*v)));

    let mut blks: Vec<Block> = Vec::with_capacity(256);
    dilvd.iter().for_each(|b| blks.push(Block::with_encoded(b, b.len() - ec_len)));
    blks
}

#[cfg(test)]
mod reader_tests {

    use crate::{
        builder::QRBuilder,
        metadata::{ECLevel, Version},
        reader::symbol::deinterleave,
        utils::BitStream,
    };

    #[test]
    fn test_deinterleave() {
        // Data length has to match version capacity
        let data = "Hello, world!!!🌍".as_bytes();
        let ver = Version::Normal(1);
        let ecl = ECLevel::L;

        let exp_blks = QRBuilder::blockify(data, ver, ecl);

        let mut bs = BitStream::new(ver.total_codewords(false) << 3);
        QRBuilder::interleave_into(&exp_blks, &mut bs);

        let blk_info = ver.data_codewords_per_block(ecl);
        let ec_len = ver.ecc_per_block(ecl);
        let blks = deinterleave(bs.data(), blk_info, ec_len);
        assert_eq!(blks, exp_blks);
    }
}

// Global constants
//------------------------------------------------------------------------------

pub const SYMBOL_HEURISTIC_THRESHOLD: f64 = 0.5;
