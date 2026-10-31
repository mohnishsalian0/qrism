//! Parity check for the HiQ classifier ports in [`crate::recovery::hiq`].
//!
//! `hiq_parity/dump.sh` runs the HiQ decoder itself on a few CUHK-CQRC test photos and writes,
//! for every module and each of HiQ's `qda`, `qda+cmi-fix` and `lsvm-ip6plus` configurations,
//! the pixel it sampled, the feature vector it built, and the colour it decided (already
//! checked there against the decoder's own channel bits). This pass feeds the same inputs to
//! the Rust port and counts disagreements, in four steps from the classifier outward:
//!
//! 1. classifier: HiQ's feature vectors in, colours out, against HiQ's colours;
//! 2. features: HiQ's raw pixels and white point in, feature vectors out, compared bit for bit
//!    (the CMI rows with HiQ's shipped neighbour rule, which is what the dump holds);
//! 3. white point: the mean of HiQ's surviving white samples, bit for bit;
//! 4. image decode: the same pixel coordinates read from the photo as `image` decodes it,
//!    the binarizer verdict on the white samples, and the colours that result, which is how
//!    much of any later disagreement is JPEG decoding rather than the port.
//!
//! The ports must match on steps 1 to 3 exactly, or the results built on them mean nothing.
//!
//! Run with:
//!   benches/color/hiq_parity/dump.sh
//!   cargo bench --features benchmark --bench color -- hiq_parity 2>/dev/null \
//!     | sed -n '/HiQ classifier parity/,$p' > benches/color/hiq_parity/parity.txt
//!
//! `paper/scripts/make_tables.py` reads `parity.txt` and refuses to build if parity failed.

use std::collections::BTreeMap;

use crate::normalization::hiq_white::{Binarizer, HiqWhite};
use crate::normalization::Normalizer;
use crate::recovery::hiq::{HiqCmi, HiqLsvm, HiqQda, Neighbours};
use crate::recovery::DirectRecovery;

const DIR: &str = "benches/color/hiq_parity";

/// One module of one configuration's dump.
struct Module {
    y: usize,
    x: usize,
    px: u32,
    py: u32,
    rgb: [f64; 3],
    features: Vec<f64>,
    pred: u8,
}

/// One photo under one configuration: its modules and white-point samples.
#[derive(Default)]
struct Run {
    dim: usize,
    modules: Vec<Module>,
    white: [f32; 3],
    /// `(px, py, rgb, black)` for each of the twelve samples HiQ took.
    white_samples: Vec<(u32, u32, [f64; 3], bool)>,
}

fn load() -> BTreeMap<(String, String), Run> {
    let mut runs: BTreeMap<(String, String), Run> = BTreeMap::new();
    let text = std::fs::read_to_string(format!("{DIR}/modules.csv"))
        .unwrap_or_else(|e| panic!("{DIR}/modules.csv: {e} (run {DIR}/dump.sh first)"));
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        let run = runs.entry((f[0].to_string(), f[1].to_string())).or_default();
        let n = |i: usize| f[i].parse::<f64>().unwrap();
        run.dim = f[2].parse().unwrap();
        run.modules.push(Module {
            y: f[3].parse().unwrap(),
            x: f[4].parse().unwrap(),
            px: f[5].parse().unwrap(),
            py: f[6].parse().unwrap(),
            rgb: [n(7), n(8), n(9)],
            features: f[10].split(' ').map(|v| v.parse().unwrap()).collect(),
            pred: f[11].parse().unwrap(),
        });
    }
    let text = std::fs::read_to_string(format!("{DIR}/white.csv")).unwrap();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split(',').collect();
        let run = runs.get_mut(&(f[0].to_string(), f[1].to_string())).unwrap();
        run.white = [f[2].parse().unwrap(), f[3].parse().unwrap(), f[4].parse().unwrap()];
        let n = |i: usize| f[i].parse::<f64>().unwrap();
        run.white_samples.push((f[6].parse().unwrap(), f[7].parse().unwrap(), [n(8), n(9), n(10)], f[11] == "1"));
    }
    runs
}

/// The port of one HiQ configuration, on HiQ's own feature vector.
fn classify(cfg: &str, v: &[f64], qda: &HiqQda, cmi: &HiqCmi, lsvm: &HiqLsvm) -> u8 {
    let c = match cfg {
        "qda" => qda.classify(v.try_into().unwrap()),
        "qda+cmi-fix" => cmi.classify15(v.try_into().unwrap()),
        "lsvm-ip6plus" => lsvm.classify(v.try_into().unwrap()),
        _ => panic!("unknown configuration {cfg}"),
    };
    c as u8
}

/// Feature vectors for every module of a run, built by the port from raw pixels.
fn build_features(cfg: &str, run: &Run, rgb: impl Fn(&Module) -> [f64; 3], norm: &HiqWhite) -> Vec<Vec<f64>> {
    let mut grid = vec![vec![[0.0f64; 3]; run.dim]; run.dim];
    for m in &run.modules {
        grid[m.y][m.x] = norm.apply(rgb(m));
    }
    run.modules
        .iter()
        .map(|m| match cfg {
            "qda+cmi-fix" => HiqCmi::features(&grid, m.x, m.y, Neighbours::HiqShipped).to_vec(),
            _ => grid[m.y][m.x].to_vec(),
        })
        .collect()
}

fn same_bits(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

pub fn benchmark_hiq_parity() {
    println!("\n\n########## HiQ classifier parity ##########");
    let runs = load();
    let (qda, cmi, lsvm) = (HiqQda::shipped(), HiqCmi::shipped(), HiqLsvm::shipped());

    println!(
        "\n    {:<28}{:<14}{:>8}{:>10}{:>10}{:>8}{:>11}{:>8}{:>9}{:>12}{:>9}",
        "photo", "config", "modules", "classify", "features", "white", "px differ", ">4 lvl", "mean lvl",
        "bin differ", "colour"
    );
    let mut failed = false;
    let mut totals = [0usize; 7];
    for ((photo, cfg), run) in &runs {
        // 1. classifier on HiQ's inputs
        let classify_bad = run
            .modules
            .iter()
            .filter(|m| classify(cfg, &m.features, &qda, &cmi, &lsvm) != m.pred)
            .count();

        // 2. features from HiQ's pixels and white point
        let java_white = HiqWhite::from_white(run.white);
        let built = build_features(cfg, run, |m| m.rgb, &java_white);
        let feature_bad =
            run.modules.iter().zip(&built).filter(|(m, f)| !same_bits(&m.features, f)).count();

        // 3. white point from HiQ's surviving samples
        let kept: Vec<[f64; 3]> =
            run.white_samples.iter().filter(|s| !s.3).map(|s| s.2).collect();
        let white_ok = HiqWhite::fit(&kept).white().map(f32::to_bits) == run.white.map(f32::to_bits);

        // 4. the same coordinates on the photo as `image` decodes it
        let img = image::open(photo).unwrap_or_else(|e| panic!("{photo}: {e}")).to_rgb8();
        let read = |x: u32, y: u32| {
            let p = img.get_pixel(x, y);
            [p[0] as f64, p[1] as f64, p[2] as f64]
        };
        let px_bad = run.modules.iter().filter(|m| read(m.px, m.py) != m.rgb).count();
        let gap = |m: &Module| {
            let p = read(m.px, m.py);
            (0..3).map(|k| (p[k] - m.rgb[k]).abs()).fold(0.0, f64::max)
        };
        let px_far = run.modules.iter().filter(|m| gap(m) > 4.0).count();
        let px_mean = run.modules.iter().map(gap).sum::<f64>() / run.modules.len() as f64;
        let mut bin = Binarizer::new(&img);
        let bin_bad = run.white_samples.iter().filter(|s| bin.is_black(s.0, s.1) != s.3).count();
        let ours_kept: Vec<[f64; 3]> = run
            .white_samples
            .iter()
            .filter(|s| !bin.is_black(s.0, s.1))
            .map(|s| read(s.0, s.1))
            .collect();
        let ours_white = HiqWhite::fit(&ours_kept);
        let ours = build_features(cfg, run, |m| read(m.px, m.py), &ours_white);
        let colour_bad = run
            .modules
            .iter()
            .zip(&ours)
            .filter(|(m, f)| classify(cfg, f, &qda, &cmi, &lsvm) != m.pred)
            .count();

        failed |= classify_bad > 0 || feature_bad > 0 || !white_ok;
        for (t, v) in totals.iter_mut().zip([run.modules.len(), classify_bad, feature_bad, px_bad, px_far, bin_bad, colour_bad]) {
            *t += v;
        }
        let short = photo.rsplit('/').take(2).collect::<Vec<_>>();
        println!(
            "    {:<28}{:<14}{:>8}{:>10}{:>10}{:>8}{:>11}{:>8}{:>9.2}{:>12}{:>9}",
            format!("{}/{}", short[1], short[0]),
            cfg,
            run.modules.len(),
            classify_bad,
            feature_bad,
            if white_ok { "exact" } else { "DIFFERS" },
            px_bad,
            px_far,
            px_mean,
            format!("{bin_bad}/{}", run.white_samples.len()),
            colour_bad,
        );
    }
    println!(
        "    {:<28}{:<14}{:>8}{:>10}{:>10}{:>8}{:>11}{:>8}{:>9}{:>12}{:>9}",
        "all", "", totals[0], totals[1], totals[2], "", totals[3], totals[4], "", totals[5], totals[6]
    );
    println!(
        "\n  classify/features/white: the port against HiQ on HiQ's own inputs (must be 0/0/exact).\n  \
         px differ: modules whose pixel `image` decodes differently from Java ImageIO (>4 lvl: by more\n  \
         than 4 levels in some channel; mean lvl: mean largest channel gap); bin differ:\n  \
         white samples the binarizer judges differently on that decode; colour: modules whose\n  \
         colour changes when the port reads those pixels, which is JPEG decoding alone."
    );
    if failed {
        println!("\n  !! PARITY FAILED: the port does not reproduce HiQ on HiQ's inputs");
    } else {
        println!("\n  parity holds: the port reproduces HiQ's decisions on every module");
    }
}
