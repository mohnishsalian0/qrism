use super::geometry::Point;

// Local frame
//
// An affine frame: an origin plus two basis vectors, each normalised to span exactly one
// module. It maps module coordinates, taken relative to the origin, onto image pixels.
//
// A frame is built from three image points -- the origin and one endpoint per axis -- together
// with the module span each endpoint sits at. The constructor divides through by those spans,
// so the stored basis is per-module however far apart the defining points were. The two axes
// carry independent spans and so need not reach equally far.
//
// The frame picks up the symbol's local rotation, scale and skew straight from the points it
// was built on, without needing the grid. Being affine it carries no perspective term and
// drifts the further it is extrapolated, which is what confines it to short reaches. That
// same independence from the grid is what makes it usable before the homography exists.
//
// Two frames are built today:
// 1. At a finder centre, with the midpoints of its black ring as endpoints -- 3 modules out on
//    each axis -- to read the version info block beside the BL & TR finders. The homography
//    needs the version, so the version has to be read first; hence a frame that predates it.
// 2. At the TL finder centre, with the TR & BL finder centres as endpoints -- width - 7
//    modules out on each axis -- to predict the provisional alignment centres.
//------------------------------------------------------------------------------

#[derive(Debug, Copy, Clone)]
pub struct LocalFrame {
    o: Point,      // Origin
    u: (f64, f64), // Basis vector along x axis
    v: (f64, f64), // Basis vector along y axis
}

impl LocalFrame {
    // `px` and `py` are the endpoints that define the +x and +y directions, sitting `spanx`
    // and `spany` modules out from the origin along their respective axes. The two
    // directions must not be parallel: a degenerate basis leaves `map` returning
    // meaningless points rather than failing.
    pub fn new(o: &Point, px: &Point, py: &Point, spanx: f64, spany: f64) -> Self {
        debug_assert!(spanx > 0.0 && spany > 0.0, "Spans cannot be zero");

        Self {
            o: *o,
            u: ((px.x - o.x) as f64 / spanx, (px.y - o.y) as f64 / spanx),
            v: ((py.x - o.x) as f64 / spany, (py.y - o.y) as f64 / spany),
        }
    }

    // Maps module coordinates, relative to the origin, onto image pixels. Offsets are counted
    // in modules and the basis is already per-module, so they scale it directly.
    pub fn map(&self, x: f64, y: f64) -> Point {
        Point {
            x: (self.o.x as f64 + x * self.u.0 + y * self.v.0).round() as i32,
            y: (self.o.y as f64 + x * self.u.1 + y * self.v.1).round() as i32,
        }
    }

    // Side of one module in pixels, averaged over the two axes. The mean of the two lengths, not
    // the root of their mean square: the two disagree once perspective foreshortens one axis, and
    // the root mean square runs high when they do.
    pub fn mod_size(&self) -> f64 {
        (self.u.0.hypot(self.u.1) + self.v.0.hypot(self.v.1)) / 2.0
    }

    // Area one module covers in pixels: the parallelogram the two basis vectors span.
    pub fn mod_area(&self) -> f64 {
        (self.u.0 * self.v.1 - self.u.1 * self.v.0).abs()
    }
}
