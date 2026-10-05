//! Colour pipeline report on the `qrism` dataset: photographs of this library's own
//! high-capacity symbols.
//!
//! Layout: `{phone}/{heuristic}/{version}-{ec}-{module size}_{repetition}.png`, with the clean
//! renders under `source/{version}-{ec}.png`. Versions 1/5/10/15/25/30/40 at every EC level,
//! module sizes 12 and 20 px (8 px under `small`), five shots each. Not every phone has every
//! heuristic; the report pools phones within each heuristic.
//!
//! Unlike HiQ, a qrism symbol carries one message spread over three channels. Each channel is
//! still its own run of Reed-Solomon blocks, so a channel "passes" when every one of its blocks
//! corrects, and the symbol decodes when all three do and the codec accepts the stream.
//!
//! A photo counts as *detected* when the localizer finds it at the version in its file name and
//! the pipeline reads the right format info (EC level and mask) off its predicted grid. Format
//! is the first thing a decoder reads and it comes out of the colour pipeline, so it belongs to
//! detection rather than to the payload.
//!
//! Three tables per heuristic, each written to a CSV under `benches/color/` and printed:
//!
//! - `qrism_confusion.csv`: a truth x detected colour matrix over data modules, per pipeline.
//! - `qrism_funnel.csv`: detected -> data-module accuracy -> channel pass -> decoded.
//! - `qrism_ec.csv`: channel pass and decode rate by EC level.
//!
//! [`MODULE_SIZE`] restricts the run to one module size; the CSV names then carry it as a
//! suffix (`qrism_funnel_20px.csv`), so filtered and unfiltered reports sit side by side.
//!
//! Detection is a rate over every photo attempted. Everything after it — data-module accuracy,
//! channel pass and decode rate — is over detected photos only, so those columns measure the
//! colour pipeline and decoder without the localizer's misses folded in.
//!
//! Run with:
//!   cargo bench --features benchmark --bench color -- qrism

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use image::RgbImage;
use rayon::prelude::*;

use crate::analysis::{ci, cname, eval_channel, eval_direct, BEST_BETA, COLORS};
use crate::calibration::{
    calibration_coords, function_mask, group_by_color, reference_grid, sample_groups,
    sample_module_rgb, GroupedSamples,
};
use crate::deblur::Deblur;
use crate::normalization::{black_white::BlackWhite, intensity::Intensity, Identity, Normalizer};
use crate::recovery::{
    euclid_measured::EuclidMeasured,
    jabcode::{BalanceRgb, JabCode},
    mahalanobis::Mahalanobis,
    per_colorant::PerColorant,
    ChannelRecovery,
};
use crate::thresholding::adaptive::Adaptive;
use qrism::bench_hooks::{decode_hc_grid, read_grid_format, Payload};
use qrism::{detect_hc_qr, Color, ECLevel, MaskPattern, Version};

const BASE: &str = "benches/dataset/high_capacity/qrism";

/// Only photos at this module size are reported; `None` takes every size. `small` holds 8 px
/// symbols only, so it drops out of a 12 or 20 px run.
const MODULE_SIZE: Option<usize> = Some(20);

/// Where one of the report's CSVs goes, suffixed with the module size when filtered.
fn csv_path(table: &str) -> String {
    let suffix = MODULE_SIZE.map(|s| format!("_{s}px")).unwrap_or_default();
    format!("benches/color/qrism_{table}{suffix}.csv")
}

const VERSIONS: [usize; 7] = [1, 5, 10, 15, 25, 30, 40];
const ECS: [char; 4] = ['L', 'M', 'Q', 'H'];

const PIPELINES: [&str; 11] = [
    "euclid raw",
    "euclid intensity",
    "black/white euclid",
    "per-colorant int + adp",
    "mahalanobis lda (raw)",
    "mahalanobis qda (raw)",
    "mahalanobis rda (raw)",
    "rda + deblur b=.04",
    "rda + deblur b=.05",
    "rda + deblur b=.06",
    "jabcode (balanced)",
];

fn ec_level(c: char) -> ECLevel {
    match c {
        'L' => ECLevel::L,
        'M' => ECLevel::M,
        'Q' => ECLevel::Q,
        'H' => ECLevel::H,
        _ => panic!("unknown EC level {c}"),
    }
}

// Dataset
//------------------------------------------------------------------------------

struct Photo {
    path: PathBuf,
    phone: String,
    heuristic: String,
    version: usize,
    ec: char,
}

/// Parses `{version}-{ec}-{size}_{rep}`.
fn parse_name(stem: &str) -> Option<(usize, char, usize, usize)> {
    let mut parts = stem.split('-');
    let version = parts.next()?.parse().ok()?;
    let ec = parts.next()?.chars().next()?;
    let (size, rep) = parts.next()?.split_once('_')?;
    Some((version, ec, size.parse().ok()?, rep.parse().ok()?))
}

fn list_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    out
}

fn name_of(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

fn list_photos() -> Vec<Photo> {
    let mut out = Vec::new();
    for phone_dir in list_dirs(Path::new(BASE)) {
        let phone = name_of(&phone_dir);
        if phone == "source" {
            continue;
        }
        for heur_dir in list_dirs(&phone_dir) {
            let heuristic = name_of(&heur_dir);
            let mut files: Vec<PathBuf> = std::fs::read_dir(&heur_dir)
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    matches!(
                        p.extension().map(|x| x.to_string_lossy().to_lowercase()).as_deref(),
                        Some("jpg" | "jpeg" | "png")
                    )
                })
                .collect();
            files.sort();
            for path in files {
                let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
                let Some((version, ec, size, _rep)) = parse_name(&stem) else {
                    println!("  skipping unparseable file name {}", path.display());
                    continue;
                };
                if MODULE_SIZE.is_some_and(|s| s != size) {
                    continue;
                }
                out.push(Photo {
                    path,
                    phone: phone.clone(),
                    heuristic: heuristic.clone(),
                    version,
                    ec,
                });
            }
        }
    }
    out
}

/// Calibration coordinates for a qrism symbol: the finder and alignment blocks, plus both
/// timing lines. qrism draws its alignment patterns in black and white, so version 1 — which
/// has none — would otherwise have no Black samples at all.
fn qrism_calibration_coords(ver: Version) -> Vec<(i32, i32)> {
    let grid = ver.width() as i32;
    let mut coords = calibration_coords(ver);
    for i in 8..=grid - 9 {
        coords.push((i, 6));
        coords.push((6, i));
    }
    coords
}

/// Ground truth for one (version, EC level) source render.
struct Reference {
    truth: Vec<Vec<Color>>,
    groups: [Vec<(i32, i32)>; 8],
    func: Vec<Vec<bool>>,
    ecl: ECLevel,
    mask: MaskPattern,
    message: String,
}

fn load_references() -> HashMap<(usize, char), Reference> {
    let mut refs = HashMap::new();
    for v in VERSIONS {
        for ec in ECS {
            let path = format!("{BASE}/source/{v}-{ec}.png");
            if !Path::new(&path).exists() {
                continue;
            }
            let ver = Version::Normal(v);
            let truth = reference_grid(&path, ver);
            let groups = group_by_color(&truth, &qrism_calibration_coords(ver));
            let func = function_mask(ver);

            // The clean render must read and decode, or the hook's view of the layout is wrong
            // — better to fail here than to report a mysterious 0% at the end.
            let (ecl, mask) = read_grid_format(&truth)
                .unwrap_or_else(|| panic!("{path}: reference format info unreadable"));
            assert_eq!(ecl, ec_level(ec), "{path}: format EC level disagrees with file name");
            let Payload::Decoded(message) = decode_hc_grid(&truth, ver, ecl, mask).payload else {
                panic!("{path}: reference failed to decode");
            };

            refs.insert((v, ec), Reference { truth, groups, func, ecl, mask, message });
        }
    }
    refs
}

// Per-photo scoring
//------------------------------------------------------------------------------

/// What one pipeline made of one photo. `None` when the photo was never detected: the
/// localizer missed it, found another version, or this pipeline misread the format.
struct Outcome {
    data_correct: u32,
    data_total: u32,
    /// Truth x detected colour counts over data modules.
    conf: [[u32; 8]; 8],
    ch_pass: [bool; 3],
    decoded: bool,
}

fn score(reference: &Reference, ver: Version, pred: &[Vec<Color>]) -> Option<Outcome> {
    if read_grid_format(pred) != Some((reference.ecl, reference.mask)) {
        return None;
    }

    let grid = pred.len();
    let (mut data_correct, mut data_total) = (0, 0);
    let mut conf = [[0u32; 8]; 8];
    for gy in 0..grid {
        for gx in 0..grid {
            if reference.func[gy][gx] {
                continue;
            }
            let (t, d) = (reference.truth[gy][gx], pred[gy][gx]);
            data_total += 1;
            data_correct += (t == d) as u32;
            conf[ci(t)][ci(d)] += 1;
        }
    }

    let dec = decode_hc_grid(pred, ver, reference.ecl, reference.mask);
    let ch_pass = std::array::from_fn(|ch| dec.block_ok[ch].iter().all(|&ok| ok));
    let decoded = matches!(&dec.payload, Payload::Decoded(m) if *m == reference.message);
    Some(Outcome { data_correct, data_total, conf, ch_pass, decoded })
}

fn run_pipelines(
    grid: usize,
    rgb: &[Vec<[f64; 3]>],
    raw: &GroupedSamples,
    groups: &[Vec<(i32, i32)>; 8],
    balance: &BalanceRgb,
) -> [Vec<Vec<Color>>; PIPELINES.len()] {
    let identity = |rec: &dyn Fn(&GroupedSamples) -> Mahalanobis| {
        let norm = Identity;
        let r = rec(&norm.normalize_groups(raw));
        eval_direct(&norm, &r, grid, rgb)
    };
    let rda_deblur = |beta: f64| {
        let db = Deblur::fixed(beta);
        let dg = db.apply(rgb);
        let norm = Identity;
        let rec = Mahalanobis::fit_rda(&norm.normalize_groups(&db.sample_groups(rgb, groups)));
        eval_direct(&norm, &rec, grid, &dg)
    };
    [
        {
            let norm = Identity;
            let rec = EuclidMeasured::fit(&norm.normalize_groups(raw));
            eval_direct(&norm, &rec, grid, rgb)
        },
        {
            let norm = Intensity::fit(raw);
            let rec = EuclidMeasured::fit(&norm.normalize_groups(raw));
            eval_direct(&norm, &rec, grid, rgb)
        },
        {
            let norm = BlackWhite::fit(raw);
            let rec = EuclidMeasured::fit(&norm.normalize_groups(raw));
            eval_direct(&norm, &rec, grid, rgb)
        },
        {
            let norm = Intensity::fit(raw);
            let ng = norm.normalize_groups(raw);
            let rec = PerColorant::fit(&ng);
            let thr = Adaptive::fit(&rec.recover_groups(&ng));
            eval_channel(&norm, &rec, &thr, grid, rgb)
        },
        identity(&|g| Mahalanobis::fit_lda(g)),
        identity(&|g| Mahalanobis::fit_qda(g)),
        identity(&|g| Mahalanobis::fit_rda(g)),
        rda_deblur(0.04),
        rda_deblur(0.05),
        rda_deblur(BEST_BETA),
        {
            // JABCode balances the whole image before sampling, so both the palette and the
            // modules it classifies are read from the stretched values.
            let bal: Vec<Vec<[f64; 3]>> =
                rgb.iter().map(|row| row.iter().map(|&x| balance.apply(x)).collect()).collect();
            let rec = JabCode::fit(&sample_groups(groups, &bal));
            eval_direct(&Identity, &rec, grid, &bal)
        },
    ]
}

/// One outcome per pipeline, all `None` when the localizer missed the photo.
fn analyze(photo: &Photo, refs: &HashMap<(usize, char), Reference>) -> Vec<Option<Outcome>> {
    let missed = || PIPELINES.iter().map(|_| None).collect();

    let reference = refs
        .get(&(photo.version, photo.ec))
        .unwrap_or_else(|| panic!("{}: no source render", photo.path.display()));
    let ver = Version::Normal(photo.version);

    let Ok(dynimg) = image::open(&photo.path) else {
        return missed();
    };
    let mut res = detect_hc_qr(&dynimg);
    let Some(sym) = res.symbols().first() else {
        return missed();
    };
    if sym.version() != ver {
        return missed();
    }

    let grid = ver.width();
    let img: RgbImage = dynimg.to_rgb8();
    let mut rgb = vec![vec![[0.0f64; 3]; grid]; grid];
    for (gy, row) in rgb.iter_mut().enumerate() {
        for (gx, cell) in row.iter_mut().enumerate() {
            let Some(s) = sample_module_rgb(sym, &img, gx, gy) else {
                return missed();
            };
            *cell = s;
        }
    }
    let raw = sample_groups(&reference.groups, &rgb);

    run_pipelines(grid, &rgb, &raw, &reference.groups, &BalanceRgb::fit(&img))
        .iter()
        .map(|pred| score(reference, ver, pred))
        .collect()
}

// Aggregation
//------------------------------------------------------------------------------

#[derive(Default)]
struct Agg {
    photos: u32,
    detected: u32,
    /// Sum of per-photo data-module accuracy over detected photos, averaged in [`Agg::data`].
    data_acc: f64,
    ch_pass: [u32; 3],
    decoded: u32,
    conf: [[u64; 8]; 8],
}

impl Agg {
    fn add(&mut self, o: &Option<Outcome>) {
        self.photos += 1;
        let Some(o) = o else { return };
        self.detected += 1;
        self.data_acc += o.data_correct as f64 / o.data_total.max(1) as f64;
        for ch in 0..3 {
            self.ch_pass[ch] += o.ch_pass[ch] as u32;
        }
        self.decoded += o.decoded as u32;
        for t in 0..8 {
            for d in 0..8 {
                self.conf[t][d] += o.conf[t][d] as u64;
            }
        }
    }

    /// `n` as a share of every photo attempted.
    fn of_photos(&self, n: u32) -> f64 {
        100.0 * n as f64 / self.photos.max(1) as f64
    }

    /// `n` as a share of detected photos.
    fn of_detected(&self, n: u32) -> f64 {
        100.0 * n as f64 / self.detected.max(1) as f64
    }

    fn data(&self) -> f64 {
        100.0 * self.data_acc / self.detected.max(1) as f64
    }
}

// Output
//------------------------------------------------------------------------------

/// A CSV whose rows are grouped by pipeline. The report walks heuristics in the outer loop,
/// so rows are held per pipeline and written together once every heuristic has been seen.
struct PipelineCsv {
    path: String,
    head: Vec<&'static str>,
    rows: Vec<Vec<Vec<String>>>,
}

impl PipelineCsv {
    /// `keys` are the columns after `pipeline, heuristic` that identify a row.
    fn new(path: String, keys: &[&'static str], cells: &[&'static str]) -> Self {
        let mut head = vec!["pipeline", "heuristic"];
        head.extend(keys);
        head.extend(cells);
        PipelineCsv { path, head, rows: vec![Vec::new(); PIPELINES.len()] }
    }

    fn push(&mut self, pi: usize, heur: &str, keys: &[&str], cells: &[String]) {
        let mut row = vec![PIPELINES[pi].to_string(), heur.to_string()];
        row.extend(keys.iter().map(|k| k.to_string()));
        row.extend(cells.iter().cloned());
        self.rows[pi].push(row);
    }

    fn write(&self) {
        let path = &self.path;
        let mut f = BufWriter::new(File::create(path).unwrap_or_else(|e| panic!("{path}: {e}")));
        writeln!(f, "{}", self.head.join(",")).unwrap();
        for row in self.rows.iter().flatten() {
            writeln!(f, "{}", row.join(",")).unwrap();
        }
    }
}

fn print_row(label: &str, label_w: usize, cells: &[String], col_w: usize) {
    print!("    {label:<label_w$}");
    for c in cells {
        print!("{c:>col_w$}");
    }
    println!();
}

fn f1(x: f64) -> String {
    format!("{x:.1}")
}

// Driver
//------------------------------------------------------------------------------

pub fn benchmark_qrism() {
    let sizes = MODULE_SIZE.map(|s| format!("{s} px modules only")).unwrap_or("all module sizes".into());
    println!("\n\n########## colour pipeline report: qrism dataset, {sizes} ##########");
    let refs = load_references();
    let photos = list_photos();
    let outcomes: Vec<Vec<Option<Outcome>>> =
        photos.par_iter().map(|p| analyze(p, &refs)).collect();

    // `regular` first as the baseline, the rest alphabetically.
    let mut heuristics: Vec<&str> =
        photos.iter().map(|p| p.heuristic.as_str()).collect::<BTreeSet<_>>().into_iter().collect();
    heuristics.sort_by_key(|&h| (h != "regular", h));

    let agg = |heur: &str, pi: usize, ec: Option<char>| -> Agg {
        let mut a = Agg::default();
        for (p, o) in photos.iter().zip(&outcomes) {
            if p.heuristic == heur && ec.is_none_or(|e| p.ec == e) {
                a.add(&o[pi]);
            }
        }
        a
    };

    let color_names: Vec<&'static str> = COLORS.iter().map(|&c| cname(c)).collect();
    let mut conf_cells = color_names.clone();
    conf_cells.extend(["total", "acc%"]);
    let mut conf_csv = PipelineCsv::new(csv_path("confusion"), &["truth"], &conf_cells);
    let mut funnel_csv = PipelineCsv::new(
        csv_path("funnel"),
        &[],
        &["photos", "detected%", "data%", "ch_r%", "ch_g%", "ch_b%", "code%"],
    );
    let mut ec_csv = PipelineCsv::new(csv_path("ec"), &["ec"], &["ch_r%", "ch_g%", "ch_b%", "code%"]);

    for &heur in &heuristics {
        let phones: BTreeSet<&str> = photos
            .iter()
            .filter(|p| p.heuristic == heur)
            .map(|p| p.phone.as_str())
            .collect();
        let n = photos.iter().filter(|p| p.heuristic == heur).count();
        println!(
            "\n\n==================== {heur} — {n} photos from {} ====================",
            phones.into_iter().collect::<Vec<_>>().join(", ")
        );

        // Funnel.
        println!("\n  detection -> decode funnel (detected% over all photos = right version + format; the rest over detected photos)");
        let heads: Vec<String> =
            ["detected%", "data%", "chR%", "chG%", "chB%", "code%"].map(String::from).to_vec();
        print_row("", 26, &heads, 11);
        for (pi, pipe) in PIPELINES.iter().enumerate() {
            let a = agg(heur, pi, None);
            let mut cells = vec![f1(a.of_photos(a.detected)), f1(a.data())];
            cells.extend(a.ch_pass.iter().map(|&c| f1(a.of_detected(c))));
            cells.push(f1(a.of_detected(a.decoded)));
            print_row(pipe, 26, &cells, 11);
            let mut row = vec![a.photos.to_string()];
            row.extend(cells);
            funnel_csv.push(pi, heur, &[], &row);
        }

        // EC level.
        println!("\n  channel pass and decode rate by EC level, over detected photos");
        print_row("", 30, &["chR%", "chG%", "chB%", "code%"].map(String::from), 9);
        for (pi, pipe) in PIPELINES.iter().enumerate() {
            for ec in ECS {
                let a = agg(heur, pi, Some(ec));
                let mut cells: Vec<String> =
                    a.ch_pass.iter().map(|&c| f1(a.of_detected(c))).collect();
                cells.push(f1(a.of_detected(a.decoded)));
                print_row(&format!("{pipe:<26}{ec}"), 30, &cells, 9);
                ec_csv.push(pi, heur, &[&ec.to_string()], &cells);
            }
        }

        // Confusion matrices. The CSV keeps counts; the printout shows each row as a
        // percentage of its truth colour so rows of different sizes read alike.
        for (pi, pipe) in PIPELINES.iter().enumerate() {
            let a = agg(heur, pi, None);
            println!("\n  [{pipe}] confusion over data modules, % of each truth colour (rows = truth, cols = detected)");
            let mut heads: Vec<String> = color_names.iter().map(|s| s.to_string()).collect();
            heads.push("modules".into());
            print_row("", 10, &heads, 9);
            for (t, &tname) in color_names.iter().enumerate() {
                let tot: u64 = a.conf[t].iter().sum();
                let mut cells: Vec<String> = a.conf[t]
                    .iter()
                    .map(|&c| if tot == 0 { "-".into() } else { f1(100.0 * c as f64 / tot as f64) })
                    .collect();
                cells.push(tot.to_string());
                print_row(tname, 10, &cells, 9);

                let mut row: Vec<String> = a.conf[t].iter().map(|c| c.to_string()).collect();
                row.push(tot.to_string());
                row.push(if tot == 0 { String::new() } else { f1(100.0 * a.conf[t][t] as f64 / tot as f64) });
                conf_csv.push(pi, heur, &[tname], &row);
            }
        }
    }

    conf_csv.write();
    funnel_csv.write();
    ec_csv.write();

    println!("\n  wrote {}, {} and {}", conf_csv.path, funnel_csv.path, ec_csv.path);
}
