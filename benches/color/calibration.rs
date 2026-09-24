//! Shared calibration inputs for the colour-decode pipeline.
//!
//! The pipeline learns its parameters per image from the *function patterns*, whose module
//! positions the spec fixes — so a decoder knows where to look before it knows anything about
//! the payload. The clean reference render gives each module's ground-truth colour
//! ([`reference_grid`]), [`calibration_coords`] picks out the finder and alignment modules,
//! those coordinates are bucketed by colour ([`group_by_color`]) and sampled in the photo
//! ([`sample_groups`]) to produce grouped RGB samples. Each pipeline stage then fits on those
//! grouped samples. Robust RGB sampling of a single module and a couple of median helpers
//! used across stages also live here.

use image::RgbImage;

use qrism::Color;
use qrism::symbol::Symbol;
use qrism::Version;

/// Grouped RGB samples: for each palette colour (indexed by colour bits 0..8), the
/// measured RGB of every module known to carry that colour. The calibration target that
/// each pipeline stage fits on. Values are raw sampled RGB; a normalizer maps them into
/// whatever space downstream stages fit in.
pub(crate) type GroupedSamples = [Vec<[f64; 3]>; 8];

// Reference grid & colour grouping
//------------------------------------------------------------------------------

/// Ground-truth module colours, read from a clean builder-rendered reference PNG. The
/// render uses pure 0/255 channels and a whole number of pixels per module.
pub(crate) fn reference_grid(ref_path: &str, ver: Version) -> Vec<Vec<Color>> {
    let img = image::open(ref_path).unwrap().to_rgb8();
    let (w, _) = img.dimensions();
    let grid = ver.width() as u32;
    let (module_sz, qz) = reference_geometry(w, grid, ref_path);
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

/// Recovers a reference render's pixels-per-module and quiet-zone width in pixels. Renders
/// differ in quiet zone — the builder emits 4 modules, the HiQ sources 1 — so rather than
/// assume one, solve `w = (grid + 2 * qz) * module_sz` for the widest quiet zone that divides
/// evenly.
fn reference_geometry(w: u32, grid: u32, path: &str) -> (u32, u32) {
    (0..=8)
        .rev()
        .find_map(|qz| {
            let span = grid + 2 * qz;
            let module_sz = w / span;
            (module_sz > 0 && w % span == 0).then_some((module_sz, qz * module_sz))
        })
        .unwrap_or_else(|| {
            panic!("{path}: {w}px does not divide evenly over a {grid}-module grid")
        })
}

/// Module coordinates the palette is calibrated from: every function-pattern module whose
/// position the spec fixes, so a real decoder could sample the same set without knowing the
/// payload.
///
/// Each finder contributes the 8x8 block at its corner — coloured ring, white gap between ring
/// and stone, coloured stone, and the white separator along the block's two inner edges (the
/// format-info row/column sits just past it, at row/column 8). Each alignment pattern
/// contributes its full 5x5 — coloured ring, white gap, coloured centre.
///
/// Colours come from the reference rather than from constants. The HiQ finder colours are
/// fixed (TL green ring / magenta stone, TR red / cyan, BL blue / yellow), but the alignment
/// colours vary by position and by version, and the bottom-right alignment is the only black
/// function pattern — the sole source of the Black group.
/// Module-space centres of every alignment pattern, minus the three cells the finders occupy.
/// The library has its own copy, but it is crate-private; the coordinate table it reads is
/// public on `Version`, so this rebuilds it rather than widening the library's API.
fn alignment_coords(ver: Version) -> impl Iterator<Item = (i32, i32)> {
    let aps = ver.alignment_pattern();
    let len = aps.len();
    let last = len.saturating_sub(1);

    (0..len)
        .flat_map(move |i| (0..len).map(move |j| (i, j)))
        .filter(move |ij| ![(0, 0), (0, last), (last, 0)].contains(ij))
        .map(move |(i, j)| (aps[i], aps[j]))
}

pub(crate) fn calibration_coords(ver: Version) -> Vec<(i32, i32)> {
    let grid = ver.width() as i32;
    let mut out = Vec::new();

    for (ox, oy) in [(0, 0), (grid - 8, 0), (0, grid - 8)] {
        for dy in 0..8 {
            for dx in 0..8 {
                out.push((ox + dx, oy + dy));
            }
        }
    }

    // Alignment centres exclude the three cells the finders occupy, so no 5x5 here can
    // overlap a finder block.
    for (cx, cy) in alignment_coords(ver) {
        for dy in -2..=2 {
            for dx in -2..=2 {
                out.push((cx + dx, cy + dy));
            }
        }
    }

    out
}

/// Marks every module the spec fixes, so scoring can tell payload from furniture: the three
/// finder blocks including their white separators, every alignment pattern, both timing lines,
/// the format-info region with its always-dark module, and — from version 7 — the two
/// version-info blocks. `true` means function module, `false` means payload.
///
/// The library marks the same set while rendering a symbol, but its grid type is crate-private,
/// so this rebuilds the positions from the public `Version` table — same reasoning as
/// [`alignment_coords`].
pub(crate) fn function_mask(ver: Version) -> Vec<Vec<bool>> {
    let grid = ver.width() as i32;
    let mut mask = vec![vec![false; grid as usize]; grid as usize];

    {
        let mut mark = |x: i32, y: i32| {
            if (0..grid).contains(&x) && (0..grid).contains(&y) {
                mask[y as usize][x as usize] = true;
            }
        };

        // Finders, each an 8x8 block: the 7x7 pattern plus the separator along its two inner
        // edges.
        for (ox, oy) in [(0, 0), (grid - 8, 0), (0, grid - 8)] {
            for dy in 0..8 {
                for dx in 0..8 {
                    mark(ox + dx, oy + dy);
                }
            }
        }

        // Timing lines, running between the finder blocks along row and column 6.
        for i in 8..=grid - 9 {
            mark(i, 6);
            mark(6, i);
        }

        // Alignment patterns, each a full 5x5 about its centre.
        for (cx, cy) in alignment_coords(ver) {
            for dy in -2..=2 {
                for dx in -2..=2 {
                    mark(cx + dx, cy + dy);
                }
            }
        }

        // Format info: row 8 and column 8, both copies. The column's bottom run starts at the
        // always-dark module, so it is covered here rather than marked separately.
        for i in 0..=8 {
            mark(i, 8);
            mark(8, i);
        }
        for i in grid - 8..grid {
            mark(i, 8);
            mark(8, i);
        }

        // Version info, from version 7 on: a 3x6 block left of the top-right finder and its
        // transpose above the bottom-left one.
        if matches!(ver, Version::Normal(7..)) {
            for i in 0..6 {
                for j in 1..=3 {
                    mark(i, grid - 8 - j);
                    mark(grid - 8 - j, i);
                }
            }
        }
    }

    mask
}

/// Buckets the given module coordinates by their ground-truth colour, indexed by colour bits
/// (0..8). These coordinates are later sampled in the *camera* image to learn how each colour
/// actually appears.
pub(crate) fn group_by_color(
    truth: &[Vec<Color>],
    coords: &[(i32, i32)],
) -> [Vec<(i32, i32)>; 8] {
    let mut out: [Vec<(i32, i32)>; 8] = Default::default();
    for &(gx, gy) in coords {
        out[truth[gy as usize][gx as usize] as usize].push((gx, gy));
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

/// Samples a module's colour at its centre, projected through the homography of the tile
/// owning that module. The median below is kept so more offsets can be added back without
/// touching the rest.
///
/// `None` when no offset mapped — the localizer left no tile for this module. Coordinates are
/// rounded to match `Homography::map`, which the library's own read path samples through.
pub(crate) fn sample_module_rgb(
    sym: &Symbol,
    img: &RgbImage,
    gx: usize,
    gy: usize,
) -> Option<[f64; 3]> {
    const OFFS: [f64; 1] = [0.5];
    let (w, ht) = img.dimensions();
    let (mut rs, mut gs, mut bs) =
        (Vec::with_capacity(9), Vec::with_capacity(9), Vec::with_capacity(9));
    for &oy in &OFFS {
        for &ox in &OFFS {
            if let Some((px, py)) = sym.exact_map(gx, gy, ox, oy) {
                let x = (px.round() as i64).clamp(0, w as i64 - 1) as u32;
                let y = (py.round() as i64).clamp(0, ht as i64 - 1) as u32;
                let p = img.get_pixel(x, y);
                rs.push(p[0]);
                gs.push(p[1]);
                bs.push(p[2]);
            }
        }
    }
    if rs.is_empty() {
        return None;
    }
    Some([median_u8(&mut rs), median_u8(&mut gs), median_u8(&mut bs)])
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
