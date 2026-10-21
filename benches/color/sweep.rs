//! Parameter sweeps for `rda + deblur`, on both datasets.
//!
//! Two passes share this machinery:
//!
//! - `sweep`: the leak fraction `beta`, fixed from 0.00 to 0.15 in steps of 0.01, plus the
//!   per-image optical estimate, with the RDA shrink held at 0.5. `beta = 0` is exactly
//!   `mahalanobis rda (raw)`, so the first row is the no-deblur baseline.
//! - `shrink`: a grid over the RDA shrink (0 = QDA, 1 = LDA, in steps of 0.1) against the
//!   `beta`s around the optimum the first pass found, plus `beta = 0`, so the two parameters
//!   are chosen jointly rather than one at a time.
//!
//! Each photo is localized and sampled once per pass; every setting then runs on that same
//! module grid. Every rate shares one denominator per dataset: photos the localizer found at
//! the right version. The qrism funnel instead reports code% over photos where *that pipeline*
//! also read the format, so its denominator moves with the pipeline; a fixed one is what lets
//! two settings be compared photo by photo. Settings are compared against the best one on
//! their dataset with an exact McNemar test, and the per-photo CSVs keep the raw pairs for any
//! other test. `data%` is pooled over modules on both datasets.
//!
//! `BER%` is the bit error rate before Reed-Solomon correction, the metric HiQ reports: each
//! module carries one bit per channel (one per layer on HiQ), and BER counts the channel bits
//! that differ from the source over every non-function module, pooled over photos. Masking
//! drops out because the source and the prediction are compared under the same mask. The
//! per-channel split is in the rates CSVs. The decode failure rate HiQ reports alongside it is
//! `100 - code%`.
//!
//! qrism photos are taken at every module size here, whatever the qrism pass's `MODULE_SIZE`
//! filter says, and broken down by size: blur is fixed in pixels, so the best `beta` should
//! move with module size.
//!
//! Outputs, under `benches/color/`, with `{pass}` = `sweep` or `shrink`:
//! - `{pass}_hiq.csv`, `{pass}_qrism.csv`: rates per setting, overall and per breakdown group.
//! - `{pass}_hiq_photos.csv`, `{pass}_qrism_photos.csv`: one row per localized photo, one 0/1
//!   decode column per setting.
//!
//! Run with:
//!   cargo bench --features benchmark --bench color -- sweep
//!   cargo bench --features benchmark --bench color -- shrink

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use image::RgbImage;
use rayon::prelude::*;

use crate::analysis::{self, eval_direct, photos, sample_grid, FOLDERS, VERSIONS};
use crate::calibration::{sample_module_rgb, GroupedSamples};
use crate::deblur::{estimate_beta_optical, Deblur};
use crate::hiq::{decode_layers, Layer};
use crate::normalization::{Identity, Normalizer};
use crate::qrism_analysis::{self, list_photos_sized, Photo};
use crate::recovery::mahalanobis::Mahalanobis;
use qrism::bench_hooks::{decode_hc_grid, read_grid_format, Payload};
use qrism::{detect_hc_qr, Color, Version};

// Settings
//------------------------------------------------------------------------------

/// The shrink `Mahalanobis::fit_rda` uses, and so the one the `sweep` pass holds fixed.
const DEFAULT_SHRINK: f64 = 0.5;

/// `beta`s the `shrink` grid crosses with every shrink: no deblur, then the range around the
/// optimum of the `beta` sweep (0.06 on both datasets, 0.05 statistically tied).
const GRID_BETAS: [f64; 6] = [0.00, 0.04, 0.05, 0.06, 0.07, 0.08];

/// Shrinks the grid tries: 0.0 to 1.0 in steps of 0.1.
const GRID_SHRINKS: usize = 11;

#[derive(Clone, Copy, PartialEq)]
enum Beta {
    Fixed(f64),
    /// Measured per image and per channel from the finder edges.
    Optical,
}

/// One configuration of `rda + deblur`.
#[derive(Clone)]
struct Setting {
    beta: Beta,
    /// Blend of each class's own covariance toward the pooled one: 0 is QDA, 1 is LDA.
    shrink: f64,
    label: String,
}

/// Divided rather than multiplied so 0.06 is the same double as `BEST_BETA`.
fn pct(i: usize) -> f64 {
    i as f64 / 100.0
}

/// The `sweep` pass: every fixed `beta` from 0.00 to 0.15, then the optical estimate, at the
/// default shrink. Labels match the columns earlier runs wrote.
fn beta_settings() -> Vec<Setting> {
    let mut v: Vec<Setting> = (0..=15)
        .map(|i| Setting {
            beta: Beta::Fixed(pct(i)),
            shrink: DEFAULT_SHRINK,
            label: format!("b={:.2}", pct(i)),
        })
        .collect();
    v.push(Setting { beta: Beta::Optical, shrink: DEFAULT_SHRINK, label: "optical".into() });
    v
}

/// The `shrink` pass: [`GRID_BETAS`] crossed with every shrink, `beta` outermost so the
/// deblurred grid can be reused across a row.
fn grid_settings() -> Vec<Setting> {
    GRID_BETAS
        .iter()
        .flat_map(|&b| {
            (0..GRID_SHRINKS).map(move |j| {
                let s = j as f64 / 10.0;
                Setting { beta: Beta::Fixed(b), shrink: s, label: format!("b={b:.2} s={s:.1}") }
            })
        })
        .collect()
}

/// Index of the grid cell the earlier passes ran: `BEST_BETA` at the default shrink.
fn grid_default(settings: &[Setting]) -> usize {
    settings
        .iter()
        .position(|s| {
            s.beta == Beta::Fixed(analysis::BEST_BETA) && (s.shrink - DEFAULT_SHRINK).abs() < 1e-9
        })
        .expect("BEST_BETA at the default shrink is on the grid")
}

/// Runs every setting on one sampled grid. Settings sharing a `beta` sit next to each other,
/// so the deblurred grid and calibration samples are computed once per run of them.
fn eval_settings(
    settings: &[Setting],
    rgb: &[Vec<[f64; 3]>],
    groups: &[Vec<(i32, i32)>; 8],
    optical: [f64; 3],
    mut score: impl FnMut(&[Vec<Color>]) -> Outcome,
) -> Vec<Outcome> {
    let mut cache: Option<(Beta, Vec<Vec<[f64; 3]>>, GroupedSamples)> = None;
    settings
        .iter()
        .map(|s| {
            if cache.as_ref().is_none_or(|(b, _, _)| *b != s.beta) {
                let db = match s.beta {
                    Beta::Fixed(b) => Deblur::fixed(b),
                    Beta::Optical => Deblur::per_channel(optical),
                };
                cache = Some((s.beta, db.apply(rgb), db.sample_groups(rgb, groups)));
            }
            let (_, dg, samples) = cache.as_ref().unwrap();
            let norm = Identity;
            let rec = Mahalanobis::fit(&norm.normalize_groups(samples), s.shrink);
            score(&eval_direct(&norm, &rec, rgb.len(), dg))
        })
        .collect()
}

// Scoring
//------------------------------------------------------------------------------

/// What one setting made of one localized photo.
#[derive(Clone, Copy, Default)]
struct Outcome {
    data_correct: u32,
    data_total: u32,
    /// Channel bits that differ from the source over the same modules, indexed R, G, B.
    bit_err: [u32; 3],
    /// HiQ: layers delivering the right message. qrism: channels whose every Reed-Solomon
    /// block corrected. Out of three either way.
    units_ok: u8,
    /// qrism only: the format info read off the predicted grid matched the source. HiQ reads
    /// format per layer inside the layer decode, so it is always set there.
    format_ok: bool,
    decoded: bool,
}

/// One row of outcomes per localized photo, one column per setting.
type Rows = [Vec<Outcome>];

/// Scores a predicted grid's non-function modules against the source: modules right, modules
/// scored, and channel bits wrong per channel. `Color as u8` packs R<<2 | G<<1 | B.
fn score_modules(truth: &[Vec<Color>], pred: &[Vec<Color>], func: &[Vec<bool>]) -> Outcome {
    let mut o = Outcome::default();
    for (y, row) in pred.iter().enumerate() {
        for (x, &d) in row.iter().enumerate() {
            if func[y][x] {
                continue;
            }
            let t = truth[y][x];
            o.data_total += 1;
            o.data_correct += (t == d) as u32;
            let diff = t as u8 ^ d as u8;
            for (ch, e) in o.bit_err.iter_mut().enumerate() {
                *e += ((diff >> (2 - ch)) & 1) as u32;
            }
        }
    }
    o
}

#[derive(Clone, Copy, Default)]
struct Tally {
    photos: u32,
    data_correct: u64,
    data_total: u64,
    bit_err: [u64; 3],
    units_ok: u32,
    format_ok: u32,
    decoded: u32,
}

impl Tally {
    fn add(&mut self, o: &Outcome) {
        self.photos += 1;
        self.data_correct += o.data_correct as u64;
        self.data_total += o.data_total as u64;
        for (t, &e) in self.bit_err.iter_mut().zip(&o.bit_err) {
            *t += e as u64;
        }
        self.units_ok += o.units_ok as u32;
        self.format_ok += o.format_ok as u32;
        self.decoded += o.decoded as u32;
    }

    fn data(&self) -> f64 {
        100.0 * self.data_correct as f64 / self.data_total.max(1) as f64
    }

    /// Pre-correction bit error rate over all three channels.
    fn ber(&self) -> f64 {
        100.0 * self.bit_err.iter().sum::<u64>() as f64 / (3 * self.data_total).max(1) as f64
    }

    /// Pre-correction bit error rate of one channel.
    fn ber_ch(&self, ch: usize) -> f64 {
        100.0 * self.bit_err[ch] as f64 / self.data_total.max(1) as f64
    }

    fn units(&self) -> f64 {
        100.0 * self.units_ok as f64 / (3 * self.photos).max(1) as f64
    }

    fn format(&self) -> f64 {
        100.0 * self.format_ok as f64 / self.photos.max(1) as f64
    }

    fn code(&self) -> f64 {
        100.0 * self.decoded as f64 / self.photos.max(1) as f64
    }
}

fn tally(rows: &Rows, n: usize) -> Vec<Tally> {
    let mut t = vec![Tally::default(); n];
    for r in rows {
        for (ti, o) in t.iter_mut().zip(r) {
            ti.add(o);
        }
    }
    t
}

/// Wilson score interval for `k` successes in `n` trials, 95%, in percent.
fn wilson(k: u32, n: u32) -> (f64, f64) {
    if n == 0 {
        return (0.0, 0.0);
    }
    let (k, n, z) = (k as f64, n as f64, 1.96f64);
    let p = k / n;
    let denom = 1.0 + z * z / n;
    let centre = (p + z * z / (2.0 * n)) / denom;
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / denom;
    (100.0 * (centre - half).max(0.0), 100.0 * (centre + half).min(1.0))
}

/// Exact two-sided McNemar p-value from the discordant pairs: `b` photos only the first
/// setting decoded, `c` only the second. Summed in log space so large `b + c` cannot
/// underflow.
fn mcnemar_p(b: u32, c: u32) -> f64 {
    let n = b + c;
    if n == 0 {
        return 1.0;
    }
    let k = b.min(c);
    let mut ln_choose = 0.0f64;
    let mut terms = Vec::with_capacity(k as usize + 1);
    for i in 0..=k {
        if i > 0 {
            ln_choose += ((n - i + 1) as f64).ln() - (i as f64).ln();
        }
        terms.push(ln_choose - n as f64 * std::f64::consts::LN_2);
    }
    let m = terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let s: f64 = terms.iter().map(|t| (t - m).exp()).sum();
    (2.0 * (m + s.ln()).exp()).min(1.0)
}

/// The best fixed-`beta` setting by decode rate. Ties go to the earlier setting: the smaller
/// `beta`, then the smaller shrink.
fn best_fixed(t: &[Tally], settings: &[Setting]) -> usize {
    (0..settings.len())
        .filter(|&i| settings[i].beta != Beta::Optical)
        .fold(None, |best: Option<usize>, i| match best {
            Some(b) if t[b].decoded >= t[i].decoded => Some(b),
            _ => Some(i),
        })
        .expect("at least one fixed-beta setting")
}

/// Discordant pairs of setting `a` against setting `b` over the same photos.
fn discordant(rows: &Rows, a: usize, b: usize) -> (u32, u32) {
    rows.iter().fold((0, 0), |(x, y), r| {
        (x + (r[a].decoded && !r[b].decoded) as u32, y + (!r[a].decoded && r[b].decoded) as u32)
    })
}

// Output
//------------------------------------------------------------------------------

/// One row per setting, with its paired test against the best fixed-`beta` setting. `unit`
/// names the three-part column (layers on HiQ, channels on qrism).
fn print_overall(
    rows: &Rows,
    settings: &[Setting],
    attempted: usize,
    unit: &str,
    show_format: bool,
) {
    let t = tally(rows, settings.len());
    let best = best_fixed(&t, settings);
    let w = settings.iter().map(|s| s.label.len()).max().unwrap_or(0).max(8) + 2;
    println!(
        "\n  {} photos attempted, {} localized; every rate below is over localized photos except e2e%",
        attempted,
        rows.len()
    );
    println!(
        "  vs best: photos only the best setting ({}) decoded / only this row decoded, exact McNemar p\n",
        settings[best].label
    );
    print!("    {:<w$}{:>8}{:>8}{:>17}{:>9}", "setting", "data%", "BER%", "BER R/G/B", unit);
    if show_format {
        print!("{:>8}", "fmt%");
    }
    println!("{:>8}{:>16}{:>8}{:>16}{:>10}", "code%", "95% CI", "e2e%", "vs best", "p");
    for (i, ti) in t.iter().enumerate() {
        let (lo, hi) = wilson(ti.decoded, ti.photos);
        let rgb = format!("{:.2}/{:.2}/{:.2}", ti.ber_ch(0), ti.ber_ch(1), ti.ber_ch(2));
        print!(
            "    {:<w$}{:>8.2}{:>8.2}{:>17}{:>9.1}",
            settings[i].label,
            ti.data(),
            ti.ber(),
            rgb,
            ti.units()
        );
        if show_format {
            print!("{:>8.1}", ti.format());
        }
        let e2e = 100.0 * ti.decoded as f64 / attempted.max(1) as f64;
        print!("{:>8.1}{:>16}{:>8.1}", ti.code(), format!("{lo:.1}-{hi:.1}"), e2e);
        if i == best {
            println!("{:>16}{:>10}", "best", "");
        } else {
            let (b, c) = discordant(rows, best, i);
            println!("{:>16}{:>10.3}", format!("{b} / {c}"), mcnemar_p(b, c));
        }
    }
}

/// Decode rate per setting across one breakdown, with each group's own best fixed `beta`.
fn print_breakdown<K: AsRef<str>>(
    title: &str,
    groups: &[(K, Vec<Vec<Outcome>>)],
    settings: &[Setting],
) {
    println!("\n  {title}  (code% over localized photos; * = best fixed beta in that column)");
    print!("    {:<10}", "");
    for (g, _) in groups {
        print!("{:>12}", g.as_ref());
    }
    println!();
    print!("    {:<10}", "photos");
    for (_, rows) in groups {
        print!("{:>12}", rows.len());
    }
    println!();
    let tallies: Vec<Vec<Tally>> = groups.iter().map(|(_, r)| tally(r, settings.len())).collect();
    let bests: Vec<usize> = tallies.iter().map(|t| best_fixed(t, settings)).collect();
    for (i, s) in settings.iter().enumerate() {
        print!("    {:<10}", s.label);
        for (t, &b) in tallies.iter().zip(&bests) {
            if t[i].photos == 0 {
                print!("{:>12}", "-");
            } else {
                let mark = if i == b { "*" } else { " " };
                print!("{:>11.1}{mark}", t[i].code());
            }
        }
        println!();
    }
}

/// The `shrink` grid as a matrix of decode rates, `beta` down and shrink across. `*` marks the
/// best cell, `=` a cell the McNemar test cannot tell apart from it at p >= 0.05.
fn print_grid(rows: &Rows, settings: &[Setting], attempted: usize, unit: &str, show_format: bool) {
    let t = tally(rows, settings.len());
    let best = best_fixed(&t, settings);
    let default = grid_default(settings);
    println!(
        "\n  {} photos attempted, {} localized; code% over localized photos, beta down, shrink across",
        attempted,
        rows.len()
    );
    println!(
        "  (shrink 0 = QDA, 1 = LDA; * = best cell, = = not distinguishable from it, McNemar p >= 0.05)\n"
    );
    print!("    {:<8}", "beta");
    for j in 0..GRID_SHRINKS {
        print!("{:>8.1}", j as f64 / 10.0);
    }
    println!();
    for (bi, b) in GRID_BETAS.iter().enumerate() {
        print!("    {:<8.2}", b);
        for j in 0..GRID_SHRINKS {
            let i = bi * GRID_SHRINKS + j;
            let mark = if i == best {
                "*"
            } else {
                let (x, y) = discordant(rows, best, i);
                if mcnemar_p(x, y) >= 0.05 {
                    "="
                } else {
                    " "
                }
            };
            print!("{:>7.1}{mark}", t[i].code());
        }
        println!();
    }

    println!();
    for (name, i) in [("best", best), ("default", default)] {
        let (lo, hi) = wilson(t[i].decoded, t[i].photos);
        print!(
            "    {:<8}{:<16}data% {:>6.2}   BER% {:>5.2}   {unit} {:>5.1}",
            name,
            settings[i].label,
            t[i].data(),
            t[i].ber(),
            t[i].units()
        );
        if show_format {
            print!("   fmt% {:>5.1}", t[i].format());
        }
        print!("   code% {:>5.1} ({lo:.1}-{hi:.1})", t[i].code());
        if i != best {
            let (x, y) = discordant(rows, best, i);
            print!("   vs best {x} / {y}, p {:.3}", mcnemar_p(x, y));
        }
        println!();
    }
}

/// For each group of a breakdown, its own best grid cell and how the default cell does there.
fn print_group_bests<K: AsRef<str>>(
    title: &str,
    groups: &[(K, Vec<Vec<Outcome>>)],
    settings: &[Setting],
) {
    let default = grid_default(settings);
    println!(
        "\n  {title}: each group's best cell against the default ({})",
        settings[default].label
    );
    println!(
        "    {:<14}{:>8}{:>18}{:>8}{:>10}{:>14}{:>9}",
        "group", "photos", "best", "code%", "default", "best / dflt", "p"
    );
    for (g, rows) in groups {
        if rows.is_empty() {
            println!("    {:<14}{:>8}", g.as_ref(), 0);
            continue;
        }
        let t = tally(rows, settings.len());
        let best = best_fixed(&t, settings);
        let (x, y) = discordant(rows, best, default);
        println!(
            "    {:<14}{:>8}{:>18}{:>8.1}{:>10.1}{:>14}{:>9.3}",
            g.as_ref(),
            rows.len(),
            settings[best].label,
            t[best].code(),
            t[default].code(),
            format!("{x} / {y}"),
            mcnemar_p(x, y)
        );
    }
}

fn open_csv(path: &str) -> BufWriter<File> {
    BufWriter::new(File::create(path).unwrap_or_else(|e| panic!("{path}: {e}")))
}

const RATES_HEAD: &str =
    "setting,beta,shrink,group_kind,group,photos,data_pct,ber_pct,ber_r,ber_g,ber_b,unit_pct,format_pct,\
                          code_pct,code_ci_lo,code_ci_hi,only_best,only_this,mcnemar_p";

/// Rates per setting for one group, appended to a rates CSV. `beta` is empty for the optical
/// estimate.
fn write_rates(f: &mut impl Write, kind: &str, group: &str, rows: &Rows, settings: &[Setting]) {
    let t = tally(rows, settings.len());
    let best = best_fixed(&t, settings);
    for (i, (ti, s)) in t.iter().zip(settings).enumerate() {
        let (lo, hi) = wilson(ti.decoded, ti.photos);
        let (b, c) = if i == best { (0, 0) } else { discordant(rows, best, i) };
        let beta = match s.beta {
            Beta::Fixed(b) => format!("{b:.2}"),
            Beta::Optical => String::new(),
        };
        writeln!(
            f,
            "{},{},{:.1},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.3},{:.3},{:.3},{:.3},{:.3},{},{},{:.5}",
            s.label,
            beta,
            s.shrink,
            kind,
            group,
            ti.photos,
            ti.data(),
            ti.ber(),
            ti.ber_ch(0),
            ti.ber_ch(1),
            ti.ber_ch(2),
            ti.units(),
            ti.format(),
            ti.code(),
            lo,
            hi,
            b,
            c,
            mcnemar_p(b, c)
        )
        .unwrap();
    }
}

fn settings_head(settings: &[Setting]) -> String {
    settings.iter().map(|s| s.label.as_str()).collect::<Vec<_>>().join(",")
}

fn decode_flags(r: &[Outcome]) -> String {
    r.iter().map(|o| if o.decoded { "1" } else { "0" }).collect::<Vec<_>>().join(",")
}

// HiQ dataset
//------------------------------------------------------------------------------

/// Every setting on one HiQ photo, or `None` when the localizer missed it or found a version
/// with no reference render.
fn hiq_photo(
    path: &Path,
    refs: &HashMap<usize, analysis::Reference>,
    settings: &[Setting],
) -> Option<(usize, Vec<Outcome>)> {
    let s = sample_grid(path, refs).ok()?;
    let r = &refs[&s.version];
    let ver = Version::Normal(s.version);
    let out = eval_settings(settings, &s.rgb, &r.groups, s.beta_optical, |pred| {
        let scored = score_modules(&r.truth, pred, &r.func);
        let ok = decode_layers(pred, ver)
            .iter()
            .zip(&r.messages)
            .filter(|(l, want)| matches!(l, Layer::Decoded(got) if got == *want))
            .count();
        Outcome { units_ok: ok as u8, format_ok: true, decoded: ok == 3, ..scored }
    });
    Some((s.version, out))
}

/// Every HiQ photo under every setting: the number attempted, and the localized ones with
/// their version and outcomes.
fn run_hiq(settings: &[Setting]) -> (usize, Vec<(PathBuf, usize, Vec<Outcome>)>) {
    let refs = analysis::load_references();
    let files: Vec<PathBuf> = FOLDERS.iter().flat_map(|f| photos(f)).collect();
    let results: Vec<Option<(usize, Vec<Outcome>)>> =
        files.par_iter().map(|p| hiq_photo(p, &refs, settings)).collect();
    let attempted = files.len();
    let localized =
        files.into_iter().zip(results).filter_map(|(p, r)| r.map(|(v, o)| (p, v, o))).collect();
    (attempted, localized)
}

fn hiq_by_version(
    localized: &[(PathBuf, usize, Vec<Outcome>)],
) -> Vec<(String, Vec<Vec<Outcome>>)> {
    VERSIONS
        .iter()
        .map(|&v| {
            let rows =
                localized.iter().filter(|(_, pv, _)| *pv == v).map(|(_, _, o)| o.clone()).collect();
            (format!("v{v}"), rows)
        })
        .collect()
}

fn write_hiq_csvs(
    pass: &str,
    settings: &[Setting],
    rows: &Rows,
    by_version: &[(String, Vec<Vec<Outcome>>)],
    localized: &[(PathBuf, usize, Vec<Outcome>)],
) {
    let rates = format!("benches/color/{pass}_hiq.csv");
    let mut f = open_csv(&rates);
    writeln!(f, "{RATES_HEAD}").unwrap();
    write_rates(&mut f, "all", "all", rows, settings);
    for (g, r) in by_version {
        write_rates(&mut f, "version", g, r, settings);
    }

    let per_photo = format!("benches/color/{pass}_hiq_photos.csv");
    let mut f = open_csv(&per_photo);
    writeln!(f, "path,version,{}", settings_head(settings)).unwrap();
    for (p, v, o) in localized {
        writeln!(f, "{},{v},{}", p.display(), decode_flags(o)).unwrap();
    }
    println!("\n  wrote {rates} and {per_photo}");
}

// qrism dataset
//------------------------------------------------------------------------------

/// Every setting on one qrism photo, or `None` when the localizer missed it or found another
/// version. A setting that misreads the format still counts, as a failed decode.
fn qrism_photo(
    photo: &Photo,
    refs: &HashMap<(usize, char), qrism_analysis::Reference>,
    settings: &[Setting],
) -> Option<Vec<Outcome>> {
    let reference = refs
        .get(&(photo.version, photo.ec))
        .unwrap_or_else(|| panic!("{}: no source render", photo.path.display()));
    let ver = Version::Normal(photo.version);

    let dynimg = image::open(&photo.path).ok()?;
    let mut res = detect_hc_qr(&dynimg);
    let sym = res.symbols().first()?;
    if sym.version() != ver {
        return None;
    }

    let grid = ver.width();
    let img: RgbImage = dynimg.to_rgb8();
    let mut rgb = vec![vec![[0.0f64; 3]; grid]; grid];
    for (gy, row) in rgb.iter_mut().enumerate() {
        for (gx, cell) in row.iter_mut().enumerate() {
            *cell = sample_module_rgb(sym, &img, gx, gy)?;
        }
    }
    let optical = estimate_beta_optical(sym, &img, ver);

    Some(eval_settings(settings, &rgb, &reference.groups, optical, |pred| {
        let mut o = score_modules(&reference.truth, pred, &reference.func);
        if read_grid_format(pred) == Some((reference.ecl, reference.mask)) {
            o.format_ok = true;
            let dec = decode_hc_grid(pred, ver, reference.ecl, reference.mask);
            o.units_ok = dec.block_ok.iter().filter(|b| b.iter().all(|&ok| ok)).count() as u8;
            o.decoded = matches!(&dec.payload, Payload::Decoded(m) if *m == reference.message);
        }
        o
    }))
}

/// Every qrism photo at every module size, and each one's outcomes when it localized.
fn run_qrism(settings: &[Setting]) -> (Vec<Photo>, Vec<Option<Vec<Outcome>>>) {
    let refs = qrism_analysis::load_references();
    let all = list_photos_sized(None);
    let results = all.par_iter().map(|p| qrism_photo(p, &refs, settings)).collect();
    (all, results)
}

type Groups = Vec<(String, Vec<Vec<Outcome>>)>;

/// The qrism breakdowns: by module size, capture condition and version.
fn qrism_breakdowns(
    localized: &[(&Photo, &Vec<Outcome>)],
    all: &[Photo],
) -> [(&'static str, Groups); 3] {
    let group = |key: &dyn Fn(&Photo) -> String| -> Groups {
        let mut keys: Vec<String> = all.iter().map(key).collect();
        keys.sort();
        keys.dedup();
        keys.into_iter()
            .map(|k| {
                let rows = localized
                    .iter()
                    .filter(|(p, _)| key(p) == k)
                    .map(|(_, o)| (*o).clone())
                    .collect();
                (k, rows)
            })
            .collect()
    };
    // Zero-padded so sizes and versions sort numerically, then the padding is dropped.
    let by_size = group(&|p| format!("{:02}px", p.size));
    let by_heur = group(&|p| p.heuristic.clone());
    let mut by_version = group(&|p| format!("v{:02}", p.version));
    for (k, _) in by_version.iter_mut() {
        *k = format!("v{}", k[1..].trim_start_matches('0'));
    }
    [("size", by_size), ("heuristic", by_heur), ("version", by_version)]
}

fn write_qrism_csvs(
    pass: &str,
    settings: &[Setting],
    rows: &Rows,
    breakdowns: &[(&str, Groups)],
    localized: &[(&Photo, &Vec<Outcome>)],
) {
    let rates = format!("benches/color/{pass}_qrism.csv");
    let mut f = open_csv(&rates);
    writeln!(f, "{RATES_HEAD}").unwrap();
    write_rates(&mut f, "all", "all", rows, settings);
    for (kind, groups) in breakdowns {
        for (g, r) in groups {
            write_rates(&mut f, kind, g, r, settings);
        }
    }

    let per_photo = format!("benches/color/{pass}_qrism_photos.csv");
    let mut f = open_csv(&per_photo);
    writeln!(f, "path,phone,heuristic,size,version,ec,{}", settings_head(settings)).unwrap();
    for (p, o) in localized {
        writeln!(
            f,
            "{},{},{},{},{},{},{}",
            p.path.display(),
            p.phone,
            p.heuristic,
            p.size,
            p.version,
            p.ec,
            decode_flags(o)
        )
        .unwrap();
    }
    println!("\n  wrote {rates} and {per_photo}");
}

/// Both datasets under one list of settings, reported either as the `beta` sweep's long tables
/// or as the `shrink` grid. Returns each dataset's tallies for the cross-dataset table.
fn run_pass(pass: &str, settings: &[Setting], grid: bool) -> (Vec<Tally>, Vec<Tally>) {
    let what = if grid { "rda shrink x beta grid" } else { "rda + deblur beta sweep" };

    println!("\n\n########## {what}: hiq dataset ##########");
    let (attempted, localized) = run_hiq(settings);
    let rows: Vec<Vec<Outcome>> = localized.iter().map(|(_, _, o)| o.clone()).collect();
    let by_version = hiq_by_version(&localized);
    if grid {
        print_grid(&rows, settings, attempted, "layer%", false);
        print_group_bests("by version", &by_version, settings);
    } else {
        print_overall(&rows, settings, attempted, "layer%", false);
        print_breakdown("by version", &by_version, settings);
    }
    write_hiq_csvs(pass, settings, &rows, &by_version, &localized);
    let hiq = tally(&rows, settings.len());

    println!("\n\n########## {what}: qrism dataset, all module sizes ##########");
    let (all, results) = run_qrism(settings);
    let localized: Vec<(&Photo, &Vec<Outcome>)> =
        all.iter().zip(&results).filter_map(|(p, r)| r.as_ref().map(|o| (p, o))).collect();
    let rows: Vec<Vec<Outcome>> = localized.iter().map(|(_, o)| (*o).clone()).collect();
    let breakdowns = qrism_breakdowns(&localized, &all);
    if grid {
        print_grid(&rows, settings, all.len(), "chan%", true);
    } else {
        print_overall(&rows, settings, all.len(), "chan%", true);
    }
    for (kind, groups) in &breakdowns {
        let title = match *kind {
            "size" => "by module size",
            "heuristic" => "by capture condition",
            _ => "by version",
        };
        if grid {
            print_group_bests(title, groups, settings);
        } else {
            print_breakdown(title, groups, settings);
        }
    }
    write_qrism_csvs(pass, settings, &rows, &breakdowns, &localized);
    let qrism = tally(&rows, settings.len());

    (hiq, qrism)
}

/// Choosing a setting on the data it is scored on flatters it. Picking on one dataset and
/// scoring on the other is the honest number, and the gap to each dataset's own optimum is
/// what that choice costs.
fn print_cross(settings: &[Setting], hiq: &[Tally], qrism: &[Tally], current: usize) {
    let (bh, bq) = (best_fixed(hiq, settings), best_fixed(qrism, settings));
    let w = settings.iter().map(|s| s.label.len()).max().unwrap_or(0).max(8) + 2;
    println!("\n\n########## setting chosen on one dataset, scored on the other ##########");
    println!("  (code% over localized photos)\n");
    println!("    {:<12}{:<w$}{:>12}{:>12}", "chosen on", "setting", "hiq code%", "qrism code%");
    for (name, i) in [("hiq", bh), ("qrism", bq), ("current", current)] {
        println!(
            "    {:<12}{:<w$}{:>12.1}{:>12.1}",
            name,
            settings[i].label,
            hiq[i].code(),
            qrism[i].code()
        );
    }
}

// Drivers
//------------------------------------------------------------------------------

/// The `beta` sweep at the default shrink.
pub fn benchmark_sweep() {
    let settings = beta_settings();
    let (hiq, qrism) = run_pass("sweep", &settings, false);
    let current = (analysis::BEST_BETA * 100.0).round() as usize;
    print_cross(&settings, &hiq, &qrism, current);
}

/// The joint shrink x `beta` grid.
pub fn benchmark_shrink() {
    let settings = grid_settings();
    let (hiq, qrism) = run_pass("shrink", &settings, true);
    print_cross(&settings, &hiq, &qrism, grid_default(&settings));
}
