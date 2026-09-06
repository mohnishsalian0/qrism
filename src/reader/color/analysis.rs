//! Benchmark harness composing colour-decode pipelines on printed colour QRs.
//!
//! Each pipeline is a combination of the three phases (normalization / recovery /
//! thresholding), all fed the *same* homography-sampled RGB per module so accuracy
//! differences are attributable to the pipeline alone. Combinations exercised:
//!   - euclid raw            : Identity      + EuclidMeasured
//!   - euclid intensity      : Intensity     + EuclidMeasured
//!   - max-channel euclid    : MaxChannel    + EuclidMeasured
//!   - black/white euclid    : BlackWhite    + EuclidMeasured
//!   - per-colorant raw      : Identity      + PerColorant + Adaptive
//!   - per-colorant intensity: Intensity     + PerColorant + Adaptive
//!
//! Runs on the `traditional_finder` dataset: ordinary black/white finders, so the normal
//! localizer ([`detect_hc_qr`]) recovers the homography with no manual anchors. The colour
//! palette is learned from the data modules — the clean reference PNG gives each module's
//! ground-truth colour; those coordinates are grouped and sampled in the photo to fit each
//! phase. Ground truth for scoring is the same reference; every module is used for both
//! calibration and scoring.
//!
//! Run with:
//!   cargo test --lib reader::color::analysis::benchmark_color_strategies -- --ignored --nocapture

use image::RgbImage;

use crate::metadata::Color;
use crate::reader::color::calibration::{
    group_by_color, reference_grid, sample_groups, sample_module_rgb,
};
use crate::reader::color::normalization::{
    absorptance::Absorptance, black_white::BlackWhite, intensity::Intensity,
    max_channel::MaxChannel, Identity, Normalizer,
};
use crate::reader::color::recovery::{
    euclid_measured::EuclidMeasured, per_colorant::PerColorant, ChannelRecovery, DirectRecovery,
};
use crate::reader::color::thresholding::{adaptive::Adaptive, local::Local, Thresholder};
use crate::reader::detect_hc_qr;
use crate::Version;

const COLORS: [Color; 8] = [
    Color::Black,
    Color::Blue,
    Color::Green,
    Color::Cyan,
    Color::Red,
    Color::Magenta,
    Color::Yellow,
    Color::White,
];

fn cname(c: Color) -> &'static str {
    match c {
        Color::Black => "Black",
        Color::Blue => "Blue",
        Color::Green => "Green",
        Color::Cyan => "Cyan",
        Color::Red => "Red",
        Color::Magenta => "Pink",
        Color::Yellow => "Yellow",
        Color::White => "White",
    }
}

// Cases
//------------------------------------------------------------------------------

struct Case {
    name: &'static str,
    reference: String,
    photo: String,
    ver: Version,
}

fn cases() -> Vec<Case> {
    let base = "benches/dataset/high_capacity/traditional_finder";
    let sizes: [(&str, Version); 4] = [
        ("xs", Version::Normal(1)),
        ("sm", Version::Normal(6)),
        ("md", Version::Normal(13)),
        ("lg", Version::Normal(20)),
    ];
    sizes
        .into_iter()
        .map(|(sz, ver)| Case {
            name: sz,
            reference: format!("{base}/{sz}.png"),
            photo: format!("{base}/{sz}1.jpeg"),
            ver,
        })
        .collect()
}

// Scoring
//------------------------------------------------------------------------------

struct Score {
    correct: usize,
    total: usize,
    lost: [u32; 3],   // truth bit 1 -> detected 0
    gained: [u32; 3], // truth bit 0 -> detected 1
    ones: [u32; 3],
    zeros: [u32; 3],
    conf: [[u32; 8]; 8],
}

fn ci(c: Color) -> usize {
    COLORS.iter().position(|&x| x == c).unwrap()
}

/// Scores a predicted grid against truth over every module.
fn score(truth: &[Vec<Color>], pred: &[Vec<Color>], grid: usize) -> Score {
    let mut s = Score {
        correct: 0,
        total: 0,
        lost: [0; 3],
        gained: [0; 3],
        ones: [0; 3],
        zeros: [0; 3],
        conf: [[0; 8]; 8],
    };
    for gy in 0..grid {
        for gx in 0..grid {
            let t = truth[gy][gx];
            let d = pred[gy][gx];
            s.total += 1;
            if t == d {
                s.correct += 1;
            }
            s.conf[ci(t)][ci(d)] += 1;
            for ch in 0..3 {
                let shift = 2 - ch;
                let tb = (t as u8 >> shift) & 1;
                let db = (d as u8 >> shift) & 1;
                if tb == 1 {
                    s.ones[ch] += 1;
                    if db == 0 {
                        s.lost[ch] += 1;
                    }
                } else {
                    s.zeros[ch] += 1;
                    if db == 1 {
                        s.gained[ch] += 1;
                    }
                }
            }
        }
    }
    s
}

fn print_summary(name: &str, s: &Score) {
    println!(
        "    {:<24} {:>5}/{:<5} = {:>5.1}%   flips R/G/B lost {:>4.1}/{:>4.1}/{:>4.1}%  gained {:>4.1}/{:>4.1}/{:>4.1}%",
        name,
        s.correct,
        s.total,
        100.0 * s.correct as f64 / s.total.max(1) as f64,
        100.0 * s.lost[0] as f64 / s.ones[0].max(1) as f64,
        100.0 * s.lost[1] as f64 / s.ones[1].max(1) as f64,
        100.0 * s.lost[2] as f64 / s.ones[2].max(1) as f64,
        100.0 * s.gained[0] as f64 / s.zeros[0].max(1) as f64,
        100.0 * s.gained[1] as f64 / s.zeros[1].max(1) as f64,
        100.0 * s.gained[2] as f64 / s.zeros[2].max(1) as f64,
    );
}

fn print_confusion(name: &str, s: &Score) {
    println!("\n  [{name}] confusion (rows=truth, cols=detected):");
    print!("    {:>8}", "");
    for c in COLORS {
        print!("{:>8}", cname(c));
    }
    println!("{:>8}{:>8}", "total", "acc%");
    for (ti, &t) in COLORS.iter().enumerate() {
        print!("    {:>8}", cname(t));
        let mut tot = 0u32;
        for di in 0..8 {
            print!("{:>8}", s.conf[ti][di]);
            tot += s.conf[ti][di];
        }
        let acc = if tot > 0 { 100.0 * s.conf[ti][ti] as f64 / tot as f64 } else { 0.0 };
        println!("{:>8}{:>7.1}%", tot, acc);
    }
}

// Grid decode
//------------------------------------------------------------------------------

fn decode_grid(grid: usize, rgb: &[Vec<[f64; 3]>], f: impl Fn([f64; 3]) -> Color) -> Vec<Vec<Color>> {
    let mut pred = vec![vec![Color::White; grid]; grid];
    for (gy, row) in pred.iter_mut().enumerate() {
        for (gx, cell) in row.iter_mut().enumerate() {
            *cell = f(rgb[gy][gx]);
        }
    }
    pred
}

/// Fits and evaluates a direct-recovery pipeline (normalize -> classify), returning its
/// prediction grid.
fn eval_direct<N: Normalizer, R: DirectRecovery>(
    norm: &N,
    rec: &R,
    grid: usize,
    rgb: &[Vec<[f64; 3]>],
) -> Vec<Vec<Color>> {
    decode_grid(grid, rgb, |c| rec.classify(norm.apply(c)))
}

/// Fits and evaluates a channel-recovery pipeline (normalize -> recover -> threshold),
/// returning its prediction grid. The recovered grid is materialised first so a spatial
/// thresholder (e.g. local) can see each module's neighbours.
fn eval_channel<N: Normalizer, R: ChannelRecovery, T: Thresholder>(
    norm: &N,
    rec: &R,
    thr: &T,
    grid: usize,
    rgb: &[Vec<[f64; 3]>],
) -> Vec<Vec<Color>> {
    let recovered: Vec<Vec<[f64; 3]>> = (0..grid)
        .map(|gy| (0..grid).map(|gx| rec.recover(norm.apply(rgb[gy][gx]))).collect())
        .collect();
    thr.decide_grid(&recovered)
}

fn analyze(case: &Case) {
    println!("\n\n========== {} (v{}) ==========", case.name, *case.ver);
    let truth = reference_grid(&case.reference, case.ver);
    let grid = case.ver.width();

    // Localize the photo with the normal pipeline; reuse its refined homography.
    let dynimg = image::open(&case.photo).unwrap();
    let mut res = detect_hc_qr(&dynimg);
    let sym = match res.symbols().first() {
        Some(s) => s,
        None => {
            println!("  localization FAILED (no symbol detected) — skipping");
            return;
        }
    };
    if sym.ver.width() != grid {
        println!(
            "  localization returned v{} (grid {}), expected v{} (grid {}) — skipping",
            *sym.ver,
            sym.ver.width(),
            *case.ver,
            grid,
        );
        return;
    }
    let h = sym.homography();
    let photo: RgbImage = dynimg.to_rgb8();

    // Shared RGB sample per module (identical input to every pipeline).
    let mut rgb = vec![vec![[0.0f64; 3]; grid]; grid];
    for gy in 0..grid {
        for gx in 0..grid {
            rgb[gy][gx] = sample_module_rgb(h, &photo, gx as i32, gy as i32);
        }
    }

    let groups = group_by_color(&truth);
    let raw = sample_groups(&groups, &rgb);

    // --- euclid raw: Identity + EuclidMeasured ---
    let euclid_raw = {
        let norm = Identity;
        let ng = norm.normalize_groups(&raw);
        let rec = EuclidMeasured::fit(&ng);
        score(&truth, &eval_direct(&norm, &rec, grid, &rgb), grid)
    };

    // --- euclid intensity: Intensity + EuclidMeasured ---
    let euclid_int = {
        let norm = Intensity::fit(&raw);
        let ng = norm.normalize_groups(&raw);
        let rec = EuclidMeasured::fit(&ng);
        score(&truth, &eval_direct(&norm, &rec, grid, &rgb), grid)
    };

    // --- max-channel euclid: MaxChannel + EuclidMeasured ---
    let (maxch_euclid, maxch_mid) = {
        let norm = MaxChannel::fit(&raw);
        let ng = norm.normalize_groups(&raw);
        let rec = EuclidMeasured::fit(&ng);
        let s = score(&truth, &eval_direct(&norm, &rec, grid, &rgb), grid);
        (s, norm.midpoints())
    };

    // --- black/white euclid: BlackWhite + EuclidMeasured ---
    let (bw_euclid, bw_black, bw_white) = {
        let norm = BlackWhite::fit(&raw);
        let ng = norm.normalize_groups(&raw);
        let rec = EuclidMeasured::fit(&ng);
        let s = score(&truth, &eval_direct(&norm, &rec, grid, &rgb), grid);
        let (black, white) = norm.references();
        (s, black, white)
    };

    // --- per-colorant raw + adaptive: Identity + PerColorant + Adaptive ---
    let pc_raw = {
        let norm = Identity;
        let ng = norm.normalize_groups(&raw);
        let rec = PerColorant::fit(&ng);
        let thr = Adaptive::fit(&rec.recover_groups(&ng));
        score(&truth, &eval_channel(&norm, &rec, &thr, grid, &rgb), grid)
    };

    // --- per-colorant intensity + adaptive: Intensity + PerColorant + Adaptive ---
    let (pc_int, pc_int_white, pc_int_thresh) = {
        let norm = Intensity::fit(&raw);
        let ng = norm.normalize_groups(&raw);
        let rec = PerColorant::fit(&ng);
        let thr = Adaptive::fit(&rec.recover_groups(&ng));
        let s = score(&truth, &eval_channel(&norm, &rec, &thr, grid, &rgb), grid);
        (s, norm.white(), thr.thresholds())
    };

    // --- per-colorant absorptance + adaptive: Absorptance + PerColorant + Adaptive ---
    let (pc_abs, pc_abs_white, pc_abs_thresh) = {
        let norm = Absorptance::fit(&raw);
        let ng = norm.normalize_groups(&raw);
        let rec = PerColorant::fit(&ng);
        let thr = Adaptive::fit(&rec.recover_groups(&ng));
        let s = score(&truth, &eval_channel(&norm, &rec, &thr, grid, &rgb), grid);
        (s, norm.white(), thr.thresholds())
    };

    // --- per-colorant intensity + local: Intensity + PerColorant + Local (indicator) ---
    let pc_int_local = {
        let norm = Intensity::fit(&raw);
        let ng = norm.normalize_groups(&raw);
        let rec = PerColorant::fit(&ng);
        let thr = Local::indicator();
        score(&truth, &eval_channel(&norm, &rec, &thr, grid, &rgb), grid)
    };

    // --- local standalone: block-threshold the sampled RGB grid directly (no recovery).
    // The most direct mirror of `prepare`, with its native brighter-is-on polarity.
    let local_raw = score(&truth, &Local::intensity().decide_grid(&rgb), grid);

    println!("  module accuracy by pipeline:");
    print_summary("euclid raw", &euclid_raw);
    print_summary("euclid intensity", &euclid_int);
    print_summary("max-channel euclid", &maxch_euclid);
    print_summary("black/white euclid", &bw_euclid);
    print_summary("per-colorant raw + adp", &pc_raw);
    print_summary("per-colorant int + adp", &pc_int);
    print_summary("per-colorant abs + adp", &pc_abs);
    print_summary("per-colorant int + local", &pc_int_local);
    print_summary("local standalone (raw)", &local_raw);

    print_confusion("euclid raw", &euclid_raw);
    print_confusion("euclid intensity", &euclid_int);
    print_confusion("max-channel euclid", &maxch_euclid);
    print_confusion("black/white euclid", &bw_euclid);
    print_confusion("per-colorant raw + adp", &pc_raw);
    print_confusion("per-colorant int + adp", &pc_int);
    print_confusion("per-colorant abs + adp", &pc_abs);
    print_confusion("per-colorant int + local", &pc_int_local);
    print_confusion("local standalone (raw)", &local_raw);

    println!(
        "\n  [per-colorant int + adp] white ref ({:>5.1}, {:>5.1}, {:>5.1}), adaptive thresh R/G/B {:>5.3} / {:>5.3} / {:>5.3}",
        pc_int_white[0], pc_int_white[1], pc_int_white[2], pc_int_thresh[0], pc_int_thresh[1], pc_int_thresh[2]
    );
    println!(
        "  [per-colorant abs + adp] white ref ({:>5.1}, {:>5.1}, {:>5.1}), adaptive thresh R/G/B {:>5.3} / {:>5.3} / {:>5.3}",
        pc_abs_white[0], pc_abs_white[1], pc_abs_white[2], pc_abs_thresh[0], pc_abs_thresh[1], pc_abs_thresh[2]
    );
    println!(
        "  [max-channel euclid] black-guard midpoints R/G/B {:>5.1} / {:>5.1} / {:>5.1}",
        maxch_mid[0], maxch_mid[1], maxch_mid[2]
    );
    println!(
        "  [black/white euclid] black ref ({:>5.1}, {:>5.1}, {:>5.1}), white ref ({:>5.1}, {:>5.1}, {:>5.1})",
        bw_black[0], bw_black[1], bw_black[2], bw_white[0], bw_white[1], bw_white[2]
    );
}

#[test]
#[ignore = "manual colour-strategy benchmark; run explicitly with --nocapture"]
fn benchmark_color_strategies() {
    for c in &cases() {
        analyze(c);
    }
}
