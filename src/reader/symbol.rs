use std::cmp::max;

use super::{
    deqr_temp::DeQR,
    finder::Finder,
    utils::geometry::{Axis, BresenhamLine, Homography, Point, Slope, X, Y},
};
use crate::Version;

// Finder line
//------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Symbol<'a> {
    deqr: &'a DeQR,
    homography: Homography,
    bounds: [Point; 4],
    ver: Version,
}

impl Symbol<'_> {
    pub fn from_group(deqr: &DeQR, group: &mut [Finder; 3]) -> Option<Self> {
        let mut c0 = group[0].center;
        let c1 = group[1].center;
        let mut c2 = group[2].center;
        // Hypotenuse slope
        let mut hm = Slope { dx: c2.x - c0.x, dy: c2.y - c0.y };

        // Make sure the finders are clockwise
        if (c1.y - c0.y) * hm.dx - (c1.x - c0.x) * hm.dy > 0 {
            group.swap(0, 2);
            std::mem::swap(&mut c0, &mut c2);
            hm.dx = -hm.dx;
            hm.dy = -hm.dy;
        }

        // Rotate finders so the top left corner is first in the list
        group.iter_mut().for_each(|f| f.rotate(&c0, &hm));

        let grid_size = measure_timing_patterns(deqr, group);

        todo!()
    }
}

// Locates pt on the middle line of each finder's ring band
// This pt is nearest to the center of symbol
// Traces vert and hor lines along these middle pts to count modules
pub fn measure_timing_patterns(deqr: &DeQR, group: &[Finder; 3]) -> usize {
    let p0 = group[0].homography.map(6.5, 0.5);
    let p1 = group[1].homography.map(6.5, 6.5);
    let p2 = group[2].homography.map(0.5, 6.5);

    // Measuring horizontal timing pattern
    let dx = (p2.x - p1.x).abs();
    let dy = (p2.y - p1.y).abs();
    let hscan =
        if dx > dy { timing_scan::<X>(deqr, &p1, &p2) } else { timing_scan::<Y>(deqr, &p1, &p2) };

    // Measuring vertical timing pattern
    let dx = (p0.x - p1.x).abs();
    let dy = (p0.y - p1.y).abs();
    let vscan =
        if dx > dy { timing_scan::<X>(deqr, &p1, &p0) } else { timing_scan::<Y>(deqr, &p1, &p0) };

    let scan = max(hscan, vscan);

    // Choose nearest valid grid size
    let size = scan + 13;
    let ver = (size as f64 - 15.0).floor() as usize / 4;
    ver * 4 + 17
}

fn timing_scan<A: Axis>(deqr: &DeQR, from: &Point, to: &Point) -> usize
where
    BresenhamLine<A>: Iterator<Item = Point>,
{
    let mut transitions = 0;
    let mut last = deqr.get_at_point(from);
    let line = BresenhamLine::<A>::new(from, to);

    for p in line {
        let px = deqr.get_at_point(&p);
        if px != last {
            transitions += 1;
            last = px;
        }
    }

    transitions
}
