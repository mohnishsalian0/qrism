use super::{
    deqr_temp::DeQR,
    finder::Finder,
    utils::geometry::{Homography, Point, Slope},
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

        todo!()
    }
}

impl Symbol<'_> {
    // Locates pt on the middle line of each finder's ring band
    // This pt is nearest to the center of symbol
    // Traces vert and hor lines along these middle pts to count modules
    pub fn measure_timing_patterns(&mut self, group: &[Finder; 3]) {
        let p0 = group[0].homography.map(6.5, 0.5);
        let p1 = group[1].homography.map(6.5, 6.5);
        let p2 = group[2].homography.map(0.5, 6.5);
        todo!()
    }

    fn timing_scan(deqr: &DeQR, p1: &Point, p2: &Point) -> usize {
        todo!()
    }
}
