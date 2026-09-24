use crate::{
    binarize::{BinaryImage, UNLABELED},
    reader::utils::geometry::{Direction, Point, PointF},
};

// Crack contour
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Contour {
    pub id: u16,
    pub encloses: bool,           // whether the outline encloses the probe point
    pub bailed: bool,             // Whether tracing bailed out early before completing loop
    bounds: (u32, u32, u32, u32), // Bounding box (x0, y0, x1, y1)
    perimeter: u32,               // Unit crack steps walked
    area2: i64,                   // 2 * signed enclosed area (shoelace sum)
    cx6: i64,                     // 6 * area * centroid x
    cy6: i64,                     // 6 * area * centroid y
    pub is_finder: bool,          // Whether the outline is a finder pattern
    pub visited_in: u32,          // Pass that claimed this contour, 0 = never
}

impl Contour {
    pub fn new(id: u16) -> Self {
        Self {
            id,
            perimeter: 0,
            encloses: false,
            bailed: true,
            bounds: (u32::MAX, u32::MAX, 0, 0),
            area2: 0,
            cx6: 0,
            cy6: 0,
            is_finder: false,
            visited_in: 0,
        }
    }

    pub fn area(&self) -> u32 {
        (self.area2 / 2).max(0) as u32
    }

    pub fn perimeter(&self) -> u32 {
        self.perimeter
    }

    pub fn centre(&self) -> Option<PointF> {
        if self.bailed || self.area2 <= 0 {
            return None;
        }

        let denom = 3.0 * self.area2 as f64;
        let x = self.cx6 as f64 / denom - 0.5;
        let y = self.cy6 as f64 / denom - 0.5;
        Some(PointF { x, y })
    }

    pub fn compactness(&self) -> f64 {
        let a = self.area();
        if a == 0 {
            return f64::INFINITY;
        }

        (self.perimeter as f64).powi(2) / (16.0 * a as f64)
    }

    // Longer side of the bounding box. Unlike a displacement from the caller's probe this depends
    // only on the outline itself, so the verdict it produces transfers between callers.
    pub fn extent(&self) -> u32 {
        self.bounds.2.saturating_sub(self.bounds.0).max(self.bounds.3.saturating_sub(self.bounds.1))
    }

    pub fn contains(&self, p: &Point) -> bool {
        !self.bailed
            && p.x >= 0
            && p.y >= 0
            && (self.bounds.0..self.bounds.2).contains(&(p.x as u32))
            && (self.bounds.1..self.bounds.3).contains(&(p.y as u32))
    }

    pub fn accumulate(&mut self, pt: &Point, d: Direction) {
        // Corners of a blob touching the top/left image edge sit at 0, so 0 is in range; the walk
        // never leaves [0, w] x [0, h], which is what makes the cast below safe.
        debug_assert!(pt.x >= 0 && pt.y >= 0);

        let (x, y) = (pt.x as u32, pt.y as u32);
        self.bounds.0 = self.bounds.0.min(x);
        self.bounds.1 = self.bounds.1.min(y);
        self.bounds.2 = self.bounds.2.max(x);
        self.bounds.3 = self.bounds.3.max(y);

        // Cross = x * dy - y * dx
        let (x, y) = (pt.x as i64, pt.y as i64);
        let (cross, dx, dy) = match d {
            Direction::Right => (-y, 1, 0),
            Direction::Down => (x, 0, 1),
            Direction::Left => (y, -1, 0),
            Direction::Up => (-x, 0, -1),
        };

        self.area2 += cross; // 2A = Σ cross
        self.cx6 += (2 * x - dx) * cross; // cx6 = Σ (2x - dx) · cross
        self.cy6 += (2 * y - dy) * cross; // cy6 = Σ (2y - dy) · cross
        self.perimeter += 1;
    }
}

// Trace contour
//------------------------------------------------------------------------------

// Walks the outer boundary of the blob containing `seed`, which must be a pixel whose right-hand
// neighbour is *NOT* part of the blob -- callers get there by walking outward from a known interior
// point, which guarantees landing on the outer boundary rather than a hole's. Returns `None` if the
// walk exceeds `max_steps`, which is how an implausibly large or convoluted blob is rejected
// without paying for all of it.
//
// `probe` is a pixel the caller wants to know the outline surrounds; the answer lands in
// `Contour::encloses`. It is a ray cast counted as the walk goes, so it costs a comparison per step
// rather than a second pass over a stored outline.
pub fn trace(
    img: &mut BinaryImage,
    reuse: Option<u16>,
    seed: Point,
    probe: Point,
    max_width: u32,
) -> Option<&mut Contour> {
    let id = match reuse {
        Some(id) => id,
        None => {
            let n = img.get_contours().len();
            debug_assert!(n < UNLABELED as usize, "Number of contours exceed 65,534");
            n as u16
        }
    };

    let max_perimeter = max_width * 4;
    let clr_bit = img.get_bit_bounded(seed.x, seed.y)?;

    // Start on the crack down the seed pixel's right edge, heading down. Travelling down with the
    // blob on the right (the -x side) is the invariant the whole walk maintains.
    let start = Point { x: seed.x + 1, y: seed.y };
    let start_dir = Direction::Down;

    // Pixel neighboring the boundary should not be the same color
    debug_assert!(
        !img.matches_bit(start.x, start.y, clr_bit),
        "Seed must be the last pixel of its horizontal run"
    );

    let mut cursor = start;
    let mut dir = start_dir;
    // The blob pixel the first crack runs along. Every later one falls out of the turn `next_step`
    // picks, so this is the only place the walk looks a flank up on its own.
    let (_, mut walked_in) = flanks(&cursor, dir);
    let mut contour = Contour::new(id);
    loop {
        // (cursor, dir) always name the crack about to be walked, and `walked_in` is its right
        // flank -- the blob pixel that crack runs along.
        img.set_px_contour(walked_in.0 as u32, walked_in.1 as u32, id);

        let last_y = cursor.y;

        cursor.advance(dir);
        contour.accumulate(&cursor, dir);

        if contour.perimeter > max_perimeter || contour.extent() > max_width {
            break;
        }

        // Ray cast rightward from the probe's pixel centre, which in corner coordinates sits at
        // (probe + 0.5). Only vertical cracks can cross a horizontal ray, and one does when it spans
        // the probe's row and lies to its right; an odd total means the probe is inside.
        contour.encloses ^= (dir == Direction::Down || dir == Direction::Up)
            && cursor.x > probe.x
            && cursor.y.min(last_y) == probe.y;

        // Where to head next, and the blob pixel that crack runs along. Both come off the flanks of
        // the crack straight ahead, which `next_step` reads for itself.
        (dir, walked_in) = next_step(img, dir, clr_bit, walked_in, &cursor);

        if cursor == start && dir == start_dir {
            contour.bailed = false;
            break;
        }
    }

    let contour = store(reuse, contour, img.get_contours_mut());

    // A hole boundary walks clockwise and sums negative, so the sign is what tells an outer
    // outline from an inner one when the seed lands on a hole (a stone bled into its ring).
    (contour.perimeter <= max_perimeter && contour.extent() <= max_width && contour.area2 > 0)
        .then_some(contour)
}

// Direction to leave `cursor`, keeping the blob on the right of travel, paired with the blob pixel
// the crack leaving in that direction runs along.
// Both flanks filled means the blob wraps around the corner: hug it by turning in.
//
// That pixel is fixed by the same two lookups that pick the direction, so carrying it out spares
// the caller a second `flanks` per step: going straight on keeps the right flank just read, turning
// left swings onto what was the left flank, and turning right pivots about the pixel the walk is
// already running along -- which is why `blob_px` comes in as well as out.
fn next_step(
    img: &BinaryImage,
    dir: Direction,
    blob_bit: bool,
    blob_px: (i32, i32),
    cursor: &Point,
) -> (Direction, (i32, i32)) {
    let (ahead_left, ahead_right) = flanks(cursor, dir);
    if !img.matches_bit(ahead_right.0, ahead_right.1, blob_bit) {
        (dir.turn_right(), blob_px)
    } else if !img.matches_bit(ahead_left.0, ahead_left.1, blob_bit) {
        (dir, ahead_right)
    } else {
        (dir.turn_left(), ahead_left)
    }
}

// Pixels flanking the crack that leaves `cursor` in direction `dir`.
// `right` is the blob-side pixel — the one this contour runs along.
fn flanks(cursor: &Point, dir: Direction) -> ((i32, i32), (i32, i32)) {
    match dir {
        Direction::Right => ((cursor.x, cursor.y - 1), (cursor.x, cursor.y)),
        Direction::Down => ((cursor.x, cursor.y), (cursor.x - 1, cursor.y)),
        Direction::Left => ((cursor.x - 1, cursor.y), (cursor.x - 1, cursor.y - 1)),
        Direction::Up => ((cursor.x - 1, cursor.y - 1), (cursor.x, cursor.y - 1)),
    }
}

fn store(reuse: Option<u16>, contour: Contour, contours: &mut Vec<Contour>) -> &mut Contour {
    match reuse {
        Some(id) => {
            let id = id as usize;
            contours[id] = contour;
            &mut contours[id]
        }
        None => {
            contours.push(contour);
            contours.last_mut().unwrap()
        }
    }
}

#[cfg(test)]
mod contour_tests {
    use super::*;
    use crate::reader::binarize::BinaryImage;
    use image::GrayImage;

    // Builds a binary image from an ASCII sketch, '#' dark and '.' light, and traces the blob
    // containing the '#' at `seed`, which the caller picks on the blob's right edge.
    fn trace_sketch(rows: &[&str], seed: Point) -> BinaryImage {
        trace_sketch_probe(rows, seed, Point { x: seed.x, y: seed.y })
    }

    fn trace_sketch_probe(rows: &[&str], seed: Point, probe: Point) -> BinaryImage {
        let mut bin = sketch(rows);
        trace(&mut bin, None, seed, probe, 10000);
        bin
    }

    // Builds the image from a sketch without tracing it, for tests that drive `trace` themselves
    // to exercise the caps.
    fn sketch(rows: &[&str]) -> BinaryImage {
        let h = rows.len() as u32;
        let w = rows[0].len() as u32;
        let mut img = GrayImage::new(w, h);
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate() {
                let v = if c == '#' { 0u8 } else { 255u8 };
                img.put_pixel(x as u32, y as u32, image::Luma([v]));
            }
        }
        BinaryImage::prepare(&img)
    }

    #[test]
    fn test_single_pixel() {
        let img = trace_sketch(&[".....", "..#..", "....."], Point { x: 2, y: 1 });
        let c = img.get_contours().last().unwrap();
        assert_eq!(c.perimeter, 4, "L1 perimeter of one pixel");
        assert_eq!(c.area(), 1);
        let centre = c.centre().unwrap();
        assert!(centre.approx_eq(&PointF { x: 2.0, y: 1.0 }));
        assert_eq!(c.bounds, (2, 1, 3, 2), "corner box around a single pixel");
        assert_eq!(c.compactness(), 1.0, "Compactness test failed");
    }

    #[test]
    fn test_square_area_and_centre() {
        let img =
            trace_sketch(&[".....", ".###.", ".###.", ".###.", "....."], Point { x: 3, y: 1 });
        let c = img.get_contours().last().unwrap();
        assert_eq!(c.perimeter, 12, "L1 perimeter of a 3x3 square");
        assert_eq!(c.area(), 9, "shoelace over cracks is the exact pixel count");
        let centre = c.centre().unwrap();
        assert!(centre.approx_eq(&PointF { x: 2.0, y: 2.0 }));
        assert_eq!(c.bounds, (1, 1, 4, 4), "pixels x,y in 1..=3, so corners in 1..=4");
        assert_eq!(c.compactness(), 1.0, "Compactness test failed");
    }

    #[test]
    fn test_square_area_with_notch() {
        let img =
            trace_sketch(&["......", "..###.", ".####.", "..###.", "......"], Point { x: 4, y: 1 });
        let c = img.get_contours().last().unwrap();
        assert_eq!(c.perimeter, 14, "L1 perimeter of a 3x3 square with notch");
        assert_eq!(c.area(), 10, "shoelace over cracks is the exact pixel count");
        assert_eq!(c.bounds, (1, 1, 5, 4), "the notch widens the box by one column");
    }

    #[test]
    fn test_corner_square_area() {
        let img =
            trace_sketch(&["###..", "###..", "###..", ".....", "....."], Point { x: 2, y: 0 });
        let c = img.get_contours().last().unwrap();
        assert_eq!(c.perimeter, 12, "L1 perimeter of a 3x3 square in the corner");
        assert_eq!(c.area(), 9, "shoelace over cracks is the exact pixel count");
        // A blob on the image edge walks corners at 0 on both axes — the case the cast in
        // `accumulate` has to tolerate.
        assert_eq!(c.bounds, (0, 0, 3, 3), "box clamps to the image corner");
    }

    // A diagonal touch must not merge the two blobs: the flood fill is 4-connected and the gates
    // ported onto the tracer assume the same.
    #[test]
    fn test_diagonal_touch_stays_separate() {
        let img = trace_sketch(
            &["......", ".##...", ".##...", "...##.", "...##.", "......"],
            Point { x: 2, y: 1 },
        );
        let c = img.get_contours().last().unwrap();
        assert_eq!(c.area(), 4, "only the seeded 2x2 block, not the diagonal neighbour");
        assert_eq!(c.perimeter, 8);
        // The box is the gate's cheap reject, so it must not swell to cover the neighbour that
        // 4-connectivity just excluded.
        assert_eq!(c.bounds, (1, 1, 3, 3), "box covers the seeded block alone");
    }

    // An interior hole is enclosed by the outer crack, so the area covers it. The ring gate relies
    // on this: tracing the white ring's outer edge yields ring + stone.
    // A closed ring surrounds its hole; a ring broken anywhere does not, even though both trace to
    // a closed outline. Telling them apart is what lets the finder gate decide whether the white
    // ring's area is a hole to subtract from the black ring's outline or not.
    #[test]
    fn test_encloses_probe() {
        let ring = ["......", ".####.", ".#..#.", ".#..#.", ".####.", "......"];
        let centre = Point { x: 2, y: 2 };
        let img = trace_sketch_probe(&ring, Point { x: 4, y: 1 }, centre);
        assert!(img.get_contours().last().unwrap().encloses);

        let broken = ["......", "..###.", ".#..#.", ".#..#.", ".####.", "......"];
        let img = trace_sketch_probe(&broken, Point { x: 4, y: 1 }, centre);
        let c = img.get_contours().last().unwrap();
        assert!(!c.encloses);
        // The box cannot tell the two rings apart — it still covers the probe once the ring is
        // broken open. Anything that swaps the ray cast for a box test answers `true` here.
        assert_eq!(c.bounds, (1, 1, 5, 5));
        assert!(c.contains(&centre));
    }

    #[test]
    fn test_hole_is_enclosed() {
        let img =
            trace_sketch(&[".....", ".###.", ".#.#.", ".###.", "....."], Point { x: 3, y: 1 });
        let c = img.get_contours().last().unwrap();
        assert_eq!(c.area(), 9, "outer crack encloses the hole");
        assert_eq!(c.perimeter, 12);
        let centre = c.centre().unwrap();
        assert!(centre.approx_eq(&PointF { x: 2.0, y: 2.0 }));
        assert_eq!(c.bounds, (1, 1, 4, 4), "hole leaves the outer box untouched");
    }

    #[test]
    fn test_step_cap_no_bail() {
        let mut bin = sketch(&["......", ".####.", ".####.", ".####.", ".####.", "......"]);
        let probe = Point { x: 4, y: 1 };
        assert!(
            trace(&mut bin, None, Point { x: 4, y: 1 }, probe, 4).is_some(),
            "16 step outline under an 16 step cap"
        );
        assert!(!bin.get_contours().last().unwrap().bailed, "16 step outline under an 16 step cap");
    }

    #[test]
    fn test_step_cap_bails() {
        let mut bin = sketch(&["......", ".####.", ".####.", ".####.", ".####.", "......"]);
        let probe = Point { x: 4, y: 1 };
        assert!(
            trace(&mut bin, None, Point { x: 4, y: 1 }, probe, 3).is_none(),
            "16 step outline under an 15 step cap"
        );
    }

    #[test]
    fn test_extent_cap_no_bail() {
        let mut bin = sketch(&["......", ".####.", ".####.", ".####.", ".####.", "......"]);
        let seed = Point { x: 4, y: 1 };
        assert!(trace(&mut bin, None, seed, seed, 4).is_some());
        assert!(!bin.get_contours().last().unwrap().bailed);
    }

    #[test]
    fn test_extent_cap_bails() {
        let mut bin = sketch(&["......", ".####.", ".####.", ".####.", ".####.", "......"]);
        let seed = Point { x: 4, y: 1 };
        assert!(trace(&mut bin, None, seed, seed, 3).is_none());

        let c = bin.get_contours().last().unwrap();
        assert!(c.bailed, "a extent bail is recorded like a step bail");
        assert!(c.perimeter() < 100, "max width stopped the walk well inside the step cap");
    }

    #[test]
    fn test_contour_pixels_marked() {
        let rows = ["......", ".####.", ".####.", ".####.", ".####.", "......"];
        let seed = Point { x: 4, y: 1 };
        let img = trace_sketch(&rows, seed);

        for x in 1..5 {
            for y in 1..5 {
                if x == 1 || x == 4 || y == 1 || y == 4 {
                    assert_eq!(img.get_px_contour(x, y), Some(0));
                } else {
                    assert!(img.get_px_contour(x, y).is_none());
                }
            }
        }
    }

    #[test]
    #[should_panic]
    fn test_non_run_end_pixel_contour() {
        let rows = ["......", ".####.", ".####.", ".####.", ".####.", "......"];
        let seed = Point { x: 3, y: 1 };
        let _img = trace_sketch(&rows, seed);
    }
}
