use std::cmp::max;

use super::{
    finder::Finder,
    prepare::PreparedImage,
    utils::geometry::{Axis, BresenhamLine, Homography, Point, Slope, X, Y},
};
use crate::{
    reader::{
        prepare::{Pixel, Region},
        utils::{accumulate::TopLeftCornerFinder, geometry::Line},
    },
    Version,
};

// Finder line
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Symbol<'a> {
    img: &'a PreparedImage,
    h: Homography,
    bounds: [Point; 4],
    ver: Version,
}

impl Symbol<'_> {
    pub fn from_group(img: &mut PreparedImage, fds: &mut [Finder; 3]) -> Option<Self> {
        let mut c0 = fds[0].center;
        let c1 = fds[1].center;
        let mut c2 = fds[2].center;
        // Hypotenuse slope
        let mut hm = Slope { dx: c2.x - c0.x, dy: c2.y - c0.y };

        // Make sure the finders are clockwise
        if (c1.y - c0.y) * hm.dx - (c1.x - c0.x) * hm.dy > 0 {
            fds.swap(0, 2);
            std::mem::swap(&mut c0, &mut c2);
            hm.dx = -hm.dx;
            hm.dy = -hm.dy;
        }

        // Rotate finders so the top left corner is first in the list
        fds.iter_mut().for_each(|f| f.rotate(&c0, &hm));

        let grid_size = measure_timing_patterns(img, fds);
        let ver = Version::from_grid_size(grid_size)?;

        let hor_line = Line::new(&fds[0].corners[0], &fds[0].corners[1]);
        let ver_line = Line::new(&fds[2].corners[0], &fds[2].corners[3]);
        let mut align_pt = hor_line.intersection(&ver_line)?;

        if grid_size > 21 {
            align_pt = locate_alignment_pattern(img, align_pt, &fds[0], &fds[2])?;
            let mut fcf = TopLeftCornerFinder::new(&align_pt, &hm);
            let px = img.get_at_point(&align_pt);
            img.repaint_and_accumulate(
                (align_pt.x as usize, align_pt.y as usize),
                px,
                Pixel::Alignment,
                &mut fcf,
            );
            align_pt = fcf.corner;
        }

        let h = setup_homography(img, fds, &align_pt, ver)?;

        todo!()
    }
}

// Locates pt on the middle line of each finder's ring band
// This pt is nearest to the center of symbol
// Traces vert and hor lines along these middle pts to count modules
pub fn measure_timing_patterns(img: &PreparedImage, fds: &[Finder; 3]) -> usize {
    let p0 = fds[0].h.map(6.5, 0.5);
    let p1 = fds[1].h.map(6.5, 6.5);
    let p2 = fds[2].h.map(0.5, 6.5);

    // Measuring horizontal timing pattern
    let dx = (p2.x - p1.x).abs();
    let dy = (p2.y - p1.y).abs();
    let hscan =
        if dx > dy { timing_scan::<X>(img, &p1, &p2) } else { timing_scan::<Y>(img, &p1, &p2) };

    // Measuring vertical timing pattern
    let dx = (p0.x - p1.x).abs();
    let dy = (p0.y - p1.y).abs();
    let vscan =
        if dx > dy { timing_scan::<X>(img, &p1, &p0) } else { timing_scan::<Y>(img, &p1, &p0) };

    let scan = max(hscan, vscan);

    // Choose nearest valid grid size
    let size = scan + 13;
    let ver = (size as f64 - 15.0).floor() as usize / 4;
    ver * 4 + 17
}

fn timing_scan<A: Axis>(img: &PreparedImage, from: &Point, to: &Point) -> usize
where
    BresenhamLine<A>: Iterator<Item = Point>,
{
    let mut transitions = 0;
    let mut last = img.get_at_point(from);
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let px = img.get_at_point(&p);
        if px != last {
            transitions += 1;
            last = px;
        }
    }

    transitions
}

fn locate_alignment_pattern(
    img: &mut PreparedImage,
    mut seed: Point,
    f0: &Finder,
    f2: &Finder,
) -> Option<Point> {
    // Get adj 2 corners of alignment pattern and compute estimate size of stome
    let (x, y) = f0.h.unmap(&seed);
    let a = f0.h.map(x, y + 1.0);
    let (x, y) = f2.h.unmap(&seed);
    let c = f2.h.map(x + 1.0, y);
    // Area of parallelogram formula used for module size of alignment stone
    let sz_est = ((a.x - seed.x) * -(c.y - seed.y) + (a.y - seed.y) * (c.x - seed.x)).unsigned_abs()
        as usize;

    // x & y increments w.r.t direction
    const DX: [i32; 4] = [1, 0, -1, 0];
    const DY: [i32; 4] = [0, -1, 0, 1];

    // Spiral outward to find stone
    let mut dir = 0;
    let mut run_len = 1;

    // WARN: 10 instead of 100 as multiplier for size estimate
    while run_len * run_len < sz_est * 10 {
        for _ in 0..run_len {
            let x = seed.x as usize;
            let y = seed.y as usize;
            let invalid = Pixel::Color([true, true, true]);

            if x < img.width() && y < img.height() && img.get_at_point(&seed) != invalid {
                let reg = img.get_region((x, y));
                let sz = match reg {
                    Region::Visited { id, area } => area,
                    _ => continue,
                };

                // Match with expected size of alignment stone
                if sz_est / 2 <= sz && sz <= sz_est * 2 {
                    return Some(seed);
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
    img: &PreparedImage,
    fds: &[Finder; 3],
    align_topleft: &Point,
    ver: Version,
) -> Option<Homography> {
    let grid_size = ver.width();
    let corners = [fds[1].corners[0], fds[2].corners[0], *align_topleft, fds[0].corners[0]];
    let initial_h = Homography::create(&corners, (grid_size - 7) as f64, (grid_size - 7) as f64)?;
    Some(jiggle_homography(img, initial_h, ver))
}

// Adjust the homography slightly to refine viewport of qr
fn jiggle_homography(img: &PreparedImage, mut h: Homography, ver: Version) -> Homography {
    let mut best = symbol_fitness(img, &h, ver);

    todo!()
}

fn symbol_fitness(img: &PreparedImage, h: &Homography, ver: Version) -> i32 {
    let mut score = 0;
    let grid_size = ver.width() as i32;

    // Score timing patterns
    // WARN: Using usize instead of i32 for i
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
    if *ver == 1 {
        return score;
    }
    let aps = ver.alignment_pattern();
    let len = aps.len();

    for i in aps[1..len - 1].iter() {
        score += alignment_fitness(img, h, 6, *i as i32);
        score += alignment_fitness(img, h, *i as i32, 6);
    }
    for i in aps[1..].iter() {
        for j in aps[1..].iter() {
            score += alignment_fitness(img, h, *i as i32, *j as i32);
        }
    }

    score
}

fn cell_fitness(img: &PreparedImage, h: &Homography, x: i32, y: i32) -> i32 {
    const OFFSETS: [f64; 3] = [0.3, 0.5, 0.7];
    let white = Pixel::Color([true, true, true]);
    let mut score = 0;

    for dy in OFFSETS.iter() {
        for dx in OFFSETS.iter() {
            let pt = h.map(x as f64 + dx, y as f64 + dy);
            if !(pt.x < 0
                || img.width() <= pt.x as usize
                || pt.y < 0
                || img.height() <= pt.y as usize)
            {
                if img.get_at_point(&pt) == white {
                    score -= 1;
                } else {
                    score += 1;
                }
            }
        }
    }
    score
}

fn finder_fitness(img: &PreparedImage, h: &Homography, x: i32, y: i32) -> i32 {
    let (x, y) = (x + 3, y + 3);
    cell_fitness(img, h, x, y) + ring_fitness(img, h, x, y, 1) - ring_fitness(img, h, x, y, 2)
        + ring_fitness(img, h, x, y, 3)
}

fn alignment_fitness(img: &PreparedImage, h: &Homography, x: i32, y: i32) -> i32 {
    cell_fitness(img, h, x, y) - ring_fitness(img, h, x, y, 1) + ring_fitness(img, h, x, y, 2)
}

fn ring_fitness(img: &PreparedImage, h: &Homography, cx: i32, cy: i32, r: i32) -> i32 {
    let mut score = 0;

    for i in 0..r * 2 {
        score += cell_fitness(img, h, cx - r + i, cy - r);
        score += cell_fitness(img, h, cx - r, cy + r - i);
        score += cell_fitness(img, h, cx + r, cy - r + 1);
        score += cell_fitness(img, h, cx + r - i, cy + r);
    }

    score
}
