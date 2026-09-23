//! Benchmark harness composing colour-decode pipelines on photographed colour QRs.
//!
//! Each pipeline is a combination of the three phases (normalization / recovery /
//! thresholding), all fed the *same* homography-sampled RGB per module so accuracy
//! differences are attributable to the pipeline alone. Combinations exercised:
//!   - euclid raw              : Identity    + EuclidMeasured
//!   - euclid intensity        : Intensity   + EuclidMeasured
//!   - max-channel euclid      : MaxChannel  + EuclidMeasured
//!   - black/white euclid      : BlackWhite  + EuclidMeasured
//!   - per-colorant int + adp  : Intensity   + PerColorant + Adaptive
//!   - per-colorant abs + adp  : Absorptance + PerColorant + Adaptive
//!   - per-colorant int + local: Intensity   + PerColorant + Local (indicator)
//!   - local standalone (raw)  : Local (intensity) straight on the sampled RGB
//!
//! Runs on the `hiq` dataset: photographs of five printed colour QRs (versions 21/25/30/35/40)
//! spread over eight capture folders. The normal localizer ([`detect_hc_qr`]) recovers both
//! the homography and the version; the version selects the matching reference render under
//! `source/`, which supplies ground truth.
//!
//! Calibration is restricted to the function patterns (see
//! [`calibration_coords`]) — the three
//! finders and every alignment pattern — so the palette is learned only from modules a real
//! decoder could locate without first decoding the payload.
//!
//! Scoring headlines the *data* modules — every module the spec does not fix, which is what a
//! decoder actually has to read. Function modules (finders and separators, alignment, timing,
//! format info with its dark module, version info) are reported alongside as an all-module
//! figure: they overlap the calibration set, so the gap between the two columns is how much of
//! a pipeline's accuracy comes from reproducing its own fit rather than generalizing.
//!
//! Each prediction grid is then decoded into the three messages the symbol carries (see
//! [`crate::hiq`]) and scored on delivered messages as well: `layer%` over individual layers,
//! `code%` over symbols whose three layers all decoded. That is the figure that ranks
//! pipelines, because module accuracy and decode success come apart — errors clustered inside
//! one Reed-Solomon block sink a layer that the same error budget scattered would survive. The
//! reference messages come from decoding each version's clean render once, at startup.
//!
//! Run with:
//!   cargo bench --features benchmark --bench color -- accuracy

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use image::RgbImage;
use rayon::prelude::*;

use crate::calibration::{
    calibration_coords, function_mask, group_by_color, reference_grid, sample_groups,
    sample_module_rgb, GroupedSamples,
};
use crate::hiq::{decode_layers, Layer};
use crate::normalization::{
    absorptance::Absorptance, black_white::BlackWhite, intensity::Intensity,
    max_channel::MaxChannel, Identity, Normalizer,
};
use crate::recovery::{
    euclid_measured::EuclidMeasured, per_colorant::PerColorant, ChannelRecovery, DirectRecovery,
};
use crate::thresholding::{adaptive::Adaptive, local::Local, Thresholder};
use qrism::detect_hc_qr;
use qrism::Color;
use qrism::Version;

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

// Dataset
//------------------------------------------------------------------------------

const BASE: &str = "benches/dataset/high_capacity/hiq";
const FOLDERS: [&str; 8] = ["01", "02", "03", "04", "05", "06", "07", "08"];

/// The reference renders in `source/`, named by QR version. A photo whose detected version is
/// not one of these has been misread by the localizer and is skipped rather than scored
/// against the wrong ground truth.
const VERSIONS: [usize; 5] = [21, 25, 30, 35, 40];

/// Photos taken from each capture folder, spread evenly over that folder's listing so a partial
/// run still covers the mix of versions and conditions it holds. `usize::MAX` takes every photo
/// — the full 5.5k-image sweep runs in about 80s, roughly half of it in the layer decode — so
/// lower this only to iterate faster.
const SAMPLES_PER_FOLDER: usize = usize::MAX;

/// The ground truth for one version: the reference grid, the calibration coordinates already
/// bucketed by their reference colour, the mask separating function modules from data, and the
/// three messages the symbol carries. Built once and shared across every photo of that
/// version, so the reference decode costs five runs per pass rather than one per photo.
struct Reference {
    truth: Vec<Vec<Color>>,
    groups: [Vec<(i32, i32)>; 8],
    func: Vec<Vec<bool>>,
    messages: [String; 3],
}

fn load_references() -> HashMap<usize, Reference> {
    VERSIONS
        .into_iter()
        .map(|v| {
            let ver = Version::Normal(v);
            let truth = reference_grid(&format!("{BASE}/source/{v}.png"), ver);
            let groups = group_by_color(&truth, &calibration_coords(ver));
            let func = function_mask(ver);

            // The clean render must decode, or the assumptions in `hiq` about how HiQ lays out
            // its layers are wrong — which is worth failing on immediately rather than
            // discovering as a mysterious 0% decode rate at the end of the sweep.
            let messages = decode_layers(&truth, ver).map(|l| match l {
                Layer::Decoded(m) => m,
                _ => panic!("{BASE}/source/{v}.png: reference layer failed to decode"),
            });

            (v, Reference { truth, groups, func, messages })
        })
        .collect()
}

/// Lists the photos sampled from one capture folder, in a deterministic order so runs are
/// comparable.
fn photos(folder: &str) -> Vec<PathBuf> {
    let dir = format!("{BASE}/{folder}");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            matches!(
                p.extension().map(|x| x.to_string_lossy().to_lowercase()).as_deref(),
                Some("jpg" | "jpeg" | "png")
            )
        })
        .collect();
    files.sort();

    // A step of 0 or 1 means the folder holds no more than we want, so take it whole.
    match files.len().checked_div(SAMPLES_PER_FOLDER).filter(|&step| step > 1) {
        Some(step) => files.into_iter().step_by(step).take(SAMPLES_PER_FOLDER).collect(),
        None => files,
    }
}

// Scoring
//------------------------------------------------------------------------------

/// Every module field but `all_*` counts data modules only; `all_*` additionally counts the
/// function modules, so the two accuracies can be shown side by side. The `layer_*` and
/// `codes_*` fields count delivered messages instead — see [`Score::add_decode`].
struct Score {
    correct: usize,
    total: usize,
    all_correct: usize,
    all_total: usize,
    lost: [u32; 3],   // truth bit 1 -> detected 0
    gained: [u32; 3], // truth bit 0 -> detected 1
    ones: [u32; 3],
    zeros: [u32; 3],
    conf: [[u32; 8]; 8],

    // Layer decode. `layer_ok` counts layers whose message matched the reference; the four
    // failure counters below partition the rest, so they sum to `layer_total - layer_ok`.
    layer_ok: usize,
    layer_total: usize,
    layer_nosym: usize,
    layer_badfmt: usize,
    layer_badpld: usize,
    /// Decoded cleanly but disagreed with the reference — Reed-Solomon accepting a corrupted
    /// codeword. Expected to stay at zero; a non-zero count is worth investigating.
    layer_wrong: usize,
    /// Symbols whose three layers all decoded correctly: the end-to-end success rate.
    codes_ok: usize,
    codes_total: usize,
}

impl Score {
    fn new() -> Self {
        Score {
            correct: 0,
            total: 0,
            all_correct: 0,
            all_total: 0,
            lost: [0; 3],
            gained: [0; 3],
            ones: [0; 3],
            zeros: [0; 3],
            conf: [[0; 8]; 8],
            layer_ok: 0,
            layer_total: 0,
            layer_nosym: 0,
            layer_badfmt: 0,
            layer_badpld: 0,
            layer_wrong: 0,
            codes_ok: 0,
            codes_total: 0,
        }
    }

    /// Folds one symbol's three decoded layers in, against the reference messages for its
    /// version.
    fn add_decode(&mut self, layers: &[Layer; 3], truth: &[String; 3]) {
        let mut all_ok = true;
        for (layer, want) in layers.iter().zip(truth) {
            self.layer_total += 1;
            match layer {
                Layer::Decoded(got) if got == want => self.layer_ok += 1,
                Layer::Decoded(_) => {
                    self.layer_wrong += 1;
                    all_ok = false;
                }
                Layer::NoSymbol => {
                    self.layer_nosym += 1;
                    all_ok = false;
                }
                Layer::BadFormat => {
                    self.layer_badfmt += 1;
                    all_ok = false;
                }
                Layer::BadPayload => {
                    self.layer_badpld += 1;
                    all_ok = false;
                }
            }
        }
        self.codes_total += 1;
        if all_ok {
            self.codes_ok += 1;
        }
    }

    /// Folds another score in, so per-image scores accumulate into per-folder, per-version and
    /// overall totals.
    fn merge(&mut self, o: &Score) {
        self.correct += o.correct;
        self.total += o.total;
        self.all_correct += o.all_correct;
        self.all_total += o.all_total;
        for k in 0..3 {
            self.lost[k] += o.lost[k];
            self.gained[k] += o.gained[k];
            self.ones[k] += o.ones[k];
            self.zeros[k] += o.zeros[k];
        }
        for t in 0..8 {
            for d in 0..8 {
                self.conf[t][d] += o.conf[t][d];
            }
        }
        self.layer_ok += o.layer_ok;
        self.layer_total += o.layer_total;
        self.layer_nosym += o.layer_nosym;
        self.layer_badfmt += o.layer_badfmt;
        self.layer_badpld += o.layer_badpld;
        self.layer_wrong += o.layer_wrong;
        self.codes_ok += o.codes_ok;
        self.codes_total += o.codes_total;
    }

    /// Accuracy over data modules — the headline figure.
    fn accuracy(&self) -> f64 {
        100.0 * self.correct as f64 / self.total.max(1) as f64
    }

    /// Accuracy over every module, function patterns included.
    fn all_accuracy(&self) -> f64 {
        100.0 * self.all_correct as f64 / self.all_total.max(1) as f64
    }

    /// Share of individual layers that delivered the right message.
    fn layer_rate(&self) -> f64 {
        100.0 * self.layer_ok as f64 / self.layer_total.max(1) as f64
    }

    /// Share of symbols where all three layers delivered — the end-to-end figure.
    fn code_rate(&self) -> f64 {
        100.0 * self.codes_ok as f64 / self.codes_total.max(1) as f64
    }
}

fn new_scores() -> [Score; PIPELINES.len()] {
    std::array::from_fn(|_| Score::new())
}

fn ci(c: Color) -> usize {
    COLORS.iter().position(|&x| x == c).unwrap()
}

/// Scores a predicted grid against truth. Function modules feed the all-module tally only;
/// every other statistic — the headline accuracy, the channel flips and the confusion matrix —
/// counts data modules alone.
fn score(truth: &[Vec<Color>], pred: &[Vec<Color>], func: &[Vec<bool>], grid: usize) -> Score {
    let mut s = Score::new();
    for gy in 0..grid {
        for gx in 0..grid {
            let t = truth[gy][gx];
            let d = pred[gy][gx];
            s.all_total += 1;
            if t == d {
                s.all_correct += 1;
            }
            if func[gy][gx] {
                continue;
            }
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

/// Column headings for [`print_summary`]. `delta` is all-module minus data-module accuracy:
/// how much the function patterns — which the palette was fitted on — flatter the score.
fn print_summary_header() {
    println!(
        "    {:<26}{:>8}{:>8}{:>8}{:>9}{:>8}    {:>18}{:>20}",
        "pipeline", "data%", "all%", "delta", "layer%", "code%", "flips lost R/G/B", "gained R/G/B"
    );
}

/// One summary row. Flip rates are over data modules, matching the headline accuracy.
fn print_summary(name: &str, s: &Score) {
    println!(
        "    {:<26}{:>8.1}{:>8.1}{:>+8.1}{:>9.1}{:>8.1}    {:>5.1}/{:>5.1}/{:>5.1}           {:>5.1}/{:>5.1}/{:>5.1}",
        name,
        s.accuracy(),
        s.all_accuracy(),
        s.all_accuracy() - s.accuracy(),
        s.layer_rate(),
        s.code_rate(),
        100.0 * s.lost[0] as f64 / s.ones[0].max(1) as f64,
        100.0 * s.lost[1] as f64 / s.ones[1].max(1) as f64,
        100.0 * s.lost[2] as f64 / s.ones[2].max(1) as f64,
        100.0 * s.gained[0] as f64 / s.zeros[0].max(1) as f64,
        100.0 * s.gained[1] as f64 / s.zeros[1].max(1) as f64,
        100.0 * s.gained[2] as f64 / s.zeros[2].max(1) as f64,
    );
}

fn print_confusion(name: &str, s: &Score) {
    println!("\n  [{name}] confusion over data modules (rows=truth, cols=detected):");
    print!("    {:>8}", "");
    for c in COLORS {
        print!("{:>10}", cname(c));
    }
    println!("{:>12}{:>8}", "total", "acc%");
    for (ti, &t) in COLORS.iter().enumerate() {
        print!("    {:>8}", cname(t));
        let mut tot = 0u32;
        for di in 0..8 {
            print!("{:>10}", s.conf[ti][di]);
            tot += s.conf[ti][di];
        }
        let acc = if tot > 0 { 100.0 * s.conf[ti][ti] as f64 / tot as f64 } else { 0.0 };
        println!("{:>12}{:>7.1}%", tot, acc);
    }
}

/// Prints a matrix of pipelines (rows) against a breakdown dimension (columns), e.g. per
/// version or per capture folder. `cell` renders one pipeline's figures for one column.
fn print_breakdown(
    title: &str,
    cols: &[String],
    by_col: &[&[Score; PIPELINES.len()]],
    cell: impl Fn(&Score) -> String,
) {
    println!("\n  {title}");
    print!("    {:<26}", "");
    for c in cols {
        print!("{:>14}", c);
    }
    println!();
    for (pi, name) in PIPELINES.iter().enumerate() {
        print!("    {:<26}", name);
        for s in by_col {
            if s[pi].all_total == 0 {
                print!("{:>14}", "-");
            } else {
                print!("{}", cell(&s[pi]));
            }
        }
        println!();
    }
}

/// Partitions a pipeline's failed layers by where the decode gave out. `wrong` counts layers
/// that decoded to the wrong message, which should not happen — Reed-Solomon accepting a
/// corrupted codeword — so it is printed only when non-zero.
fn print_decode_failures(name: &str, s: &Score) {
    let failed = s.layer_total - s.layer_ok;
    let pct = |n: usize| 100.0 * n as f64 / s.layer_total.max(1) as f64;
    print!(
        "    {:<26}{:>10}{:>14.1}{:>14.1}{:>14.1}",
        name,
        failed,
        pct(s.layer_nosym),
        pct(s.layer_badfmt),
        pct(s.layer_badpld)
    );
    if s.layer_wrong > 0 {
        print!("   !! {} decoded to the wrong message", s.layer_wrong);
    }
    println!();
}

// Grid decode
//------------------------------------------------------------------------------

fn decode_grid(
    grid: usize,
    rgb: &[Vec<[f64; 3]>],
    f: impl Fn([f64; 3]) -> Color,
) -> Vec<Vec<Color>> {
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

// Pipelines
//------------------------------------------------------------------------------

const PIPELINES: [&str; 8] = [
    "euclid raw",
    "euclid intensity",
    "max-channel euclid",
    "black/white euclid",
    "per-colorant int + adp",
    "per-colorant abs + adp",
    "per-colorant int + local",
    "local standalone (raw)",
];

/// Fits every pipeline on this image's calibration samples and decodes the whole grid with
/// each, returning prediction grids in [`PIPELINES`] order.
/// Runs `f`, recording how long it took into `slot`.
fn timed<T>(slot: &mut Duration, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    *slot = start.elapsed();
    out
}

#[allow(clippy::type_complexity)]
fn run_pipelines(
    grid: usize,
    rgb: &[Vec<[f64; 3]>],
    raw: &GroupedSamples,
) -> ([Vec<Vec<Color>>; PIPELINES.len()], [Duration; PIPELINES.len()]) {
    let mut times = [Duration::ZERO; PIPELINES.len()];
    let euclid_raw = timed(&mut times[0], || {
        let norm = Identity;
        let ng = norm.normalize_groups(raw);
        let rec = EuclidMeasured::fit(&ng);
        eval_direct(&norm, &rec, grid, rgb)
    });

    let euclid_int = timed(&mut times[1], || {
        let norm = Intensity::fit(raw);
        let ng = norm.normalize_groups(raw);
        let rec = EuclidMeasured::fit(&ng);
        eval_direct(&norm, &rec, grid, rgb)
    });

    let maxch_euclid = timed(&mut times[2], || {
        let norm = MaxChannel::fit(raw);
        let ng = norm.normalize_groups(raw);
        let rec = EuclidMeasured::fit(&ng);
        eval_direct(&norm, &rec, grid, rgb)
    });

    let bw_euclid = timed(&mut times[3], || {
        let norm = BlackWhite::fit(raw);
        let ng = norm.normalize_groups(raw);
        let rec = EuclidMeasured::fit(&ng);
        eval_direct(&norm, &rec, grid, rgb)
    });

    let pc_int = timed(&mut times[4], || {
        let norm = Intensity::fit(raw);
        let ng = norm.normalize_groups(raw);
        let rec = PerColorant::fit(&ng);
        let thr = Adaptive::fit(&rec.recover_groups(&ng));
        eval_channel(&norm, &rec, &thr, grid, rgb)
    });

    let pc_abs = timed(&mut times[5], || {
        let norm = Absorptance::fit(raw);
        let ng = norm.normalize_groups(raw);
        let rec = PerColorant::fit(&ng);
        let thr = Adaptive::fit(&rec.recover_groups(&ng));
        eval_channel(&norm, &rec, &thr, grid, rgb)
    });

    let pc_int_local = timed(&mut times[6], || {
        let norm = Intensity::fit(raw);
        let ng = norm.normalize_groups(raw);
        let rec = PerColorant::fit(&ng);
        let thr = Local::indicator();
        eval_channel(&norm, &rec, &thr, grid, rgb)
    });

    // Block-threshold the sampled RGB grid directly (no recovery). The most direct mirror of
    // `prepare`, with its native brighter-is-on polarity.
    let local_raw = timed(&mut times[7], || Local::intensity().decide_grid(rgb));

    (
        [euclid_raw, euclid_int, maxch_euclid, bw_euclid, pc_int, pc_abs, pc_int_local, local_raw],
        times,
    )
}

/// Dumps the parameters each stage fitted on one image, as a sanity check that a bad score is
/// a strategy failing rather than a fit going degenerate.
fn print_fit_params(label: &str, raw: &GroupedSamples) {
    let counts: Vec<String> =
        COLORS.iter().map(|&c| format!("{}={}", cname(c), raw[c as usize].len())).collect();
    println!("\n  [{label}] calibration samples: {}", counts.join(" "));

    let int = Intensity::fit(raw);
    let abs = Absorptance::fit(raw);
    let maxch = MaxChannel::fit(raw);
    let bw = BlackWhite::fit(raw);
    let (black, white) = bw.references();

    let thresh = |n: &dyn Normalizer| {
        let ng = n.normalize_groups(raw);
        let rec = PerColorant::fit(&ng);
        Adaptive::fit(&rec.recover_groups(&ng)).thresholds()
    };
    let ti = thresh(&int);
    let ta = thresh(&abs);
    let mid = maxch.midpoints();
    let iw = int.white();

    println!(
        "    intensity white ref ({:>5.1}, {:>5.1}, {:>5.1})   adaptive thresh R/G/B {:>6.3} / {:>6.3} / {:>6.3}",
        iw[0], iw[1], iw[2], ti[0], ti[1], ti[2]
    );
    println!(
        "    absorptance white ref ({:>5.1}, {:>5.1}, {:>5.1}) adaptive thresh R/G/B {:>6.3} / {:>6.3} / {:>6.3}",
        abs.white()[0],
        abs.white()[1],
        abs.white()[2],
        ta[0],
        ta[1],
        ta[2]
    );
    println!(
        "    max-channel black-guard midpoints R/G/B {:>5.1} / {:>5.1} / {:>5.1}",
        mid[0], mid[1], mid[2]
    );
    println!(
        "    black/white refs: black ({:>5.1}, {:>5.1}, {:>5.1})  white ({:>5.1}, {:>5.1}, {:>5.1})",
        black[0], black[1], black[2], white[0], white[1], white[2]
    );
}

// Per-photo run
//------------------------------------------------------------------------------

/// Everything downstream of image decoding for one photo: the version the localizer reported,
/// the per-module RGB, and the calibration samples drawn from the function patterns. Shared by
/// the accuracy pass and the timing pass so both measure the same inputs.
struct Sampled {
    version: usize,
    rgb: Vec<Vec<[f64; 3]>>,
    raw: GroupedSamples,
}

/// Why a photo contributed nothing.
enum Skip {
    Unreadable,
    NoSymbol,
    /// Localized at a version with no reference render — a misread, not a colour failure.
    UnknownVersion(usize),
    /// The localizer left a hole in its tile grid, so a module could not be sampled.
    NoTile,
}

/// Localizes a photo and samples its module grid. Depends on nothing but its arguments, so
/// photos run in parallel.
fn sample_grid(path: &Path, refs: &HashMap<usize, Reference>) -> Result<Sampled, Skip> {
    let Ok(dynimg) = image::open(path) else {
        return Err(Skip::Unreadable);
    };

    let mut res = detect_hc_qr(&dynimg);
    let Some(sym) = res.symbols().first() else {
        return Err(Skip::NoSymbol);
    };

    let ver = sym.version();
    let version = *ver;
    let Some(reference) = refs.get(&version) else {
        return Err(Skip::UnknownVersion(version));
    };
    let grid = ver.width();
    let photo: RgbImage = dynimg.to_rgb8();

    // Shared RGB sample per module (identical input to every pipeline). Each module is
    // projected through the homography of the tile owning it, so the samples carry the same
    // local perspective correction the normal decode path uses.
    let mut rgb = vec![vec![[0.0f64; 3]; grid]; grid];
    for (gy, rgb_row) in rgb.iter_mut().enumerate() {
        for (gx, rgb_item) in rgb_row.iter_mut().enumerate() {
            let Some(sample) = sample_module_rgb(sym, &photo, gx, gy) else {
                return Err(Skip::NoTile);
            };
            *rgb_item = sample;
        }
    }

    let raw = sample_groups(&reference.groups, &rgb);
    Ok(Sampled { version, rgb, raw })
}

/// What one photo contributed: a score per pipeline, plus the calibration samples it fitted on
/// — kept so the driver can dump one representative fit per version.
struct PhotoResult {
    scores: [Score; PIPELINES.len()],
    raw: GroupedSamples,
}

enum Outcome {
    Scored(usize, Box<PhotoResult>),
    Skipped(Skip),
}

/// Decodes one photo with every pipeline and scores each against the reference for the version
/// the localizer reported.
fn analyze(path: &Path, refs: &HashMap<usize, Reference>) -> Outcome {
    let sampled = match sample_grid(path, refs) {
        Ok(s) => s,
        Err(skip) => return Outcome::Skipped(skip),
    };

    let reference = &refs[&sampled.version];
    let grid = sampled.rgb.len();
    let ver = Version::Normal(sampled.version);
    let (preds, _times) = run_pipelines(grid, &sampled.rgb, &sampled.raw);
    let scores = std::array::from_fn(|pi| {
        let mut s = score(&reference.truth, &preds[pi], &reference.func, grid);
        s.add_decode(&decode_layers(&preds[pi], ver), &reference.messages);
        s
    });

    Outcome::Scored(sampled.version, Box::new(PhotoResult { scores, raw: sampled.raw }))
}

// Accuracy pass
//------------------------------------------------------------------------------

pub fn benchmark_accuracy() {
    println!("\n\n########## colour pipeline accuracy ##########");
    let refs = load_references();
    let mut dumped = HashSet::new();

    let mut overall = new_scores();
    let mut by_version: HashMap<usize, [Score; PIPELINES.len()]> =
        VERSIONS.into_iter().map(|v| (v, new_scores())).collect();
    let mut by_folder: HashMap<&str, [Score; PIPELINES.len()]> =
        FOLDERS.into_iter().map(|f| (f, new_scores())).collect();

    let (mut attempted, mut scored) = (0usize, 0usize);
    let (mut unreadable, mut no_symbol, mut no_tile) = (0usize, 0usize, 0usize);
    let mut bad_version: HashMap<usize, usize> = HashMap::new();

    for folder in FOLDERS {
        let files = photos(folder);
        println!("\n===== folder {folder} ({} photos) =====", files.len());

        // Decoding dominates the runtime and photos are independent, so they run across cores.
        // The fold below stays serial and in listing order: every total is an integer sum, so
        // the output is identical to a serial run rather than merely equivalent.
        let outcomes: Vec<Outcome> = files.par_iter().map(|p| analyze(p, &refs)).collect();

        for (path, outcome) in files.iter().zip(outcomes) {
            attempted += 1;
            match outcome {
                Outcome::Scored(v, res) => {
                    scored += 1;
                    if dumped.insert(v) {
                        print_fit_params(&format!("v{v} sample: {}", path.display()), &res.raw);
                    }
                    for (pi, s) in res.scores.iter().enumerate() {
                        overall[pi].merge(s);
                        by_version.get_mut(&v).unwrap()[pi].merge(s);
                        by_folder.get_mut(folder).unwrap()[pi].merge(s);
                    }
                }
                Outcome::Skipped(Skip::Unreadable) => unreadable += 1,
                Outcome::Skipped(Skip::NoSymbol) => no_symbol += 1,
                Outcome::Skipped(Skip::NoTile) => no_tile += 1,
                Outcome::Skipped(Skip::UnknownVersion(v)) => {
                    *bad_version.entry(v).or_default() += 1
                }
            }
        }

        println!(
            "  done ({} data modules scored so far in this folder)",
            by_folder[folder][0].total
        );
    }

    println!("\n\n========== summary ==========");
    println!(
        "  photos: {attempted} attempted, {scored} scored | skipped: {no_symbol} no symbol, \
         {no_tile} missing tile, {unreadable} unreadable"
    );
    if !bad_version.is_empty() {
        let mut v: Vec<_> = bad_version.iter().collect();
        v.sort();
        let detail: Vec<String> = v.iter().map(|(ver, n)| format!("v{ver}x{n}")).collect();
        println!("  localized at a version with no reference (misread): {}", detail.join(" "));
    }
    if scored == 0 {
        println!("  nothing scored — stopping");
        return;
    }

    let (data_mods, all_mods) = (overall[0].total, overall[0].all_total);
    println!(
        "\n  modules: {data_mods} data scored, {} function excluded from the data columns",
        all_mods - data_mods
    );
    println!(
        "\n  module accuracy and layer decode by pipeline (all folders, all versions):\n  \
         layer% = layers delivering the right message, code% = symbols where all three did"
    );
    print_summary_header();
    for (pi, name) in PIPELINES.iter().enumerate() {
        print_summary(name, &overall[pi]);
    }

    println!("\n  layer decode failures by pipeline (percentages are of all layers attempted):");
    println!(
        "    {:<26}{:>10}{:>14}{:>14}{:>14}",
        "pipeline", "failed", "no symbol%", "bad format%", "bad payload%"
    );
    for (pi, name) in PIPELINES.iter().enumerate() {
        print_decode_failures(name, &overall[pi]);
    }

    let acc_cell = |s: &Score| format!("{:>8.1}/{:<5.1}", s.accuracy(), s.all_accuracy());
    let dec_cell = |s: &Score| format!("{:>8.1}/{:<5.1}", s.layer_rate(), s.code_rate());

    let vcols: Vec<String> = VERSIONS.iter().map(|v| format!("v{v}")).collect();
    let vscores: Vec<&[Score; PIPELINES.len()]> = VERSIONS.iter().map(|v| &by_version[v]).collect();
    print_breakdown("accuracy by version:  (cells are data% / all%)", &vcols, &vscores, acc_cell);
    print_breakdown("decode by version:  (cells are layer% / code%)", &vcols, &vscores, dec_cell);

    let fcols: Vec<String> = FOLDERS.iter().map(|f| f.to_string()).collect();
    let fscores: Vec<&[Score; PIPELINES.len()]> = FOLDERS.iter().map(|f| &by_folder[*f]).collect();
    print_breakdown(
        "accuracy by capture folder:  (cells are data% / all%)",
        &fcols,
        &fscores,
        acc_cell,
    );
    print_breakdown(
        "decode by capture folder:  (cells are layer% / code%)",
        &fcols,
        &fscores,
        dec_cell,
    );

    for (pi, name) in PIPELINES.iter().enumerate() {
        print_confusion(name, &overall[pi]);
    }
}

// Timing pass
//------------------------------------------------------------------------------

/// Photos sampled for the timing pass, per capture folder.
const TIMING_PHOTOS_PER_FOLDER: usize = 2;
/// Repetitions over the whole sampled set, to average out scheduling noise.
const TIMING_REPS: usize = 5;

/// Measures what each pipeline costs per module, excluding JPEG decode and localization.
///
/// Those two are ~91% of wall time but are shared by every pipeline and would be paid once by a
/// real decoder regardless, so timing them here would drown the differences being measured. The
/// pass runs single-threaded on pre-sampled grids: what is being compared is the per-module cost
/// of fit + normalize + recover + threshold.
pub fn benchmark_timing() {
    println!("\n\n########## colour pipeline timing ##########");
    let refs = load_references();

    let grids: Vec<Sampled> = FOLDERS
        .iter()
        .flat_map(|f| photos(f).into_iter().take(TIMING_PHOTOS_PER_FOLDER))
        .filter_map(|p| sample_grid(&p, &refs).ok())
        .collect();

    if grids.is_empty() {
        println!("  no symbols sampled — skipping");
        return;
    }

    let modules: usize = grids.iter().map(|s| s.rgb.len() * s.rgb.len()).sum();
    let mut totals = [Duration::ZERO; PIPELINES.len()];
    for _ in 0..TIMING_REPS {
        for s in &grids {
            let (_preds, times) = run_pipelines(s.rgb.len(), &s.rgb, &s.raw);
            for (slot, t) in totals.iter_mut().zip(times) {
                *slot += t;
            }
        }
    }

    let samples = (modules * TIMING_REPS) as f64;
    let v40 = qrism::Version::Normal(40).width();
    let v40_modules = (v40 * v40) as f64;
    println!(
        "\n  {} symbols x {TIMING_REPS} reps, {modules} modules per rep, single-threaded",
        grids.len()
    );
    println!("  excludes image decode and localization (~91% of end-to-end wall time)\n");
    println!(
        "    {:<26}{:>12}{:>14}{:>16}{:>10}",
        "pipeline", "total ms", "ns/module", "ms per v40 sym", "rel"
    );

    let fastest = totals.iter().min().copied().unwrap_or(Duration::ZERO).as_secs_f64();
    for (pi, name) in PIPELINES.iter().enumerate() {
        let ns_per_module = totals[pi].as_secs_f64() * 1e9 / samples;
        println!(
            "    {:<26}{:>12.1}{:>14.1}{:>16.2}{:>9.1}x",
            name,
            totals[pi].as_secs_f64() * 1e3,
            ns_per_module,
            ns_per_module * v40_modules / 1e6,
            if fastest > 0.0 { totals[pi].as_secs_f64() / fastest } else { 0.0 },
        );
    }
}
