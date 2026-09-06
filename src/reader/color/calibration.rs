//! Shared calibration inputs for the colour-decode pipeline.
//!
//! The pipeline learns its parameters per image from the *data modules themselves*: the
//! clean reference render gives each module's ground-truth colour ([`reference_grid`]),
//! those coordinates are bucketed by colour ([`group_by_color`]) and sampled in the photo
//! ([`sample_groups`]) to produce grouped RGB samples. Each pipeline stage then fits on
//! those grouped samples. Robust RGB sampling of a single module and a couple of median
//! helpers used across stages also live here.

use image::RgbImage;

use crate::metadata::Color;
use crate::reader::utils::homography::Homography;
use crate::Version;

/// Grouped RGB samples: for each palette colour (indexed by colour bits 0..8), the
/// measured RGB of every module known to carry that colour. The calibration target that
/// each pipeline stage fits on. Values are raw sampled RGB; a normalizer maps them into
/// whatever space downstream stages fit in.
pub(crate) type GroupedSamples = [Vec<[f64; 3]>; 8];

// Reference grid & colour grouping
//------------------------------------------------------------------------------

/// Ground-truth module colours, read from a clean builder-rendered reference PNG. The
/// render uses pure 0/255 channels, `module_sz` px per module and a 4-module quiet zone.
pub(crate) fn reference_grid(ref_path: &str, ver: Version) -> Vec<Vec<Color>> {
    let img = image::open(ref_path).unwrap().to_rgb8();
    let (w, _) = img.dimensions();
    let grid = ver.width() as u32;
    let module_sz = w / (grid + 8);
    let qz = 4 * module_sz;
    let half = module_sz / 2;

    let mut out = vec![vec![Color::White; grid as usize]; grid as usize];
    for gy in 0..grid {
        for gx in 0..grid {
            let px = qz + gx * module_sz + half;
            let py = qz + gy * module_sz + half;
            let p = img.get_pixel(px, py);
            let bits = ((p[0] > 127) as u8) << 2 | ((p[1] > 127) as u8) << 1 | (p[2] > 127) as u8;
            out[gy as usize][gx as usize] = Color::try_from(bits).unwrap();
        }
    }
    out
}

/// Buckets every module coordinate of a reference grid by its ground-truth colour,
/// indexed by colour bits (0..8). These coordinates are later sampled in the *camera*
/// image to learn how each colour actually appears.
pub(crate) fn group_by_color(truth: &[Vec<Color>]) -> [Vec<(i32, i32)>; 8] {
    let mut out: [Vec<(i32, i32)>; 8] = Default::default();
    for (gy, row) in truth.iter().enumerate() {
        for (gx, &c) in row.iter().enumerate() {
            out[c as usize].push((gx as i32, gy as i32));
        }
    }
    out
}

/// Gathers the sampled RGB of every grouped coordinate into per-colour buckets, reading
/// from an already-sampled per-module RGB grid (both indexed in module coordinates).
pub(crate) fn sample_groups(
    groups: &[Vec<(i32, i32)>; 8],
    rgb: &[Vec<[f64; 3]>],
) -> GroupedSamples {
    std::array::from_fn(|c| {
        groups[c].iter().map(|&(gx, gy)| rgb[gy as usize][gx as usize]).collect()
    })
}

// RGB sampling
//------------------------------------------------------------------------------

/// Robustly samples a module's colour: projects a 3x3 grid over the central ~30% of the
/// module through the homography and takes the per-channel median, rejecting edge/bleed
/// pixels and specular outliers.
pub(crate) fn sample_module_rgb(h: &Homography, img: &RgbImage, gx: i32, gy: i32) -> [f64; 3] {
    const OFFS: [f64; 3] = [0.35, 0.5, 0.65];
    let (w, ht) = img.dimensions();
    let (mut rs, mut gs, mut bs) =
        (Vec::with_capacity(9), Vec::with_capacity(9), Vec::with_capacity(9));
    for &oy in &OFFS {
        for &ox in &OFFS {
            if let Ok(pt) = h.map(gx as f64 + ox, gy as f64 + oy) {
                let x = pt.x.clamp(0, w as i32 - 1) as u32;
                let y = pt.y.clamp(0, ht as i32 - 1) as u32;
                let p = img.get_pixel(x, y);
                rs.push(p[0]);
                gs.push(p[1]);
                bs.push(p[2]);
            }
        }
    }
    [median_u8(&mut rs), median_u8(&mut gs), median_u8(&mut bs)]
}

fn median_u8(v: &mut [u8]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_unstable();
    v[v.len() / 2] as f64
}

// Median helpers shared across stages
//------------------------------------------------------------------------------

/// Per-channel median of a set of RGB samples.
pub(crate) fn median_rgb(v: &[[f64; 3]]) -> [f64; 3] {
    std::array::from_fn(|k| {
        let mut c: Vec<f64> = v.iter().map(|p| p[k]).collect();
        median_f(&mut c)
    })
}

/// Median of a scalar sample (sorts in place); 0.0 for an empty slice.
pub(crate) fn median_f(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}
