use geo::{Area, BooleanOps, Coord, Polygon};
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

use qrism::detect_qr;
use qrism::symbol::Symbol;

#[path = "utils.rs"]
mod utils;
use utils::{get_parent, is_image_file, parse_expected_bounds_result, print_table};

pub fn benchmark_detection(dataset_dir: &Path) {
    let image_paths: Vec<_> = WalkDir::new(dataset_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(is_image_file)
        .map(|e| e.path().to_path_buf())
        .collect();

    let results = Arc::new(Mutex::new(HashMap::<String, HashMap<String, f64>>::new()));
    let runtimes = Arc::new(Mutex::new(HashMap::<String, Vec<u128>>::new()));

    image_paths.par_iter().for_each(|img_path| {
        let parent = get_parent(img_path);

        let exp_path = img_path.with_extension("txt");
        let exp_symbols = parse_expected_bounds_result(&exp_path);

        let img = image::open(img_path).unwrap();

        // Filters QRs which can be decoded correctly. Measures time to decode all QRs
        let start = Instant::now();
        let mut res = detect_qr(&img);
        let symbols: Vec<&mut Symbol> = res
            .symbols()
            .iter_mut()
            .filter_map(|s| if s.decode().is_ok() { Some(s) } else { None })
            .collect();
        let time = start.elapsed().as_millis();

        let symbols = get_corners(&symbols);
        let true_pos = match_areas(&symbols, &exp_symbols);
        let false_pos = symbols.len() - true_pos;
        let false_neg = exp_symbols.len() - true_pos;

        // let path_str = img_path.to_str().unwrap();
        // println!("\x1b[1;32m[PASSED {}/{}]\x1b[0m {}", true_pos, exp_symbols.len(), path_str);

        let mut results = results.lock().unwrap();
        let mut runtimes = runtimes.lock().unwrap();

        let score = results.entry(parent.clone()).or_default();
        *score.entry("true_pos".to_string()).or_default() += true_pos as f64;
        *score.entry("false_pos".to_string()).or_default() += false_pos as f64;
        *score.entry("false_neg".to_string()).or_default() += false_neg as f64;

        runtimes.entry(parent).or_default().push(time);
    });

    // Remaining aggregation logic (same as original)
    let mut results = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
    let mut runtimes = Arc::try_unwrap(runtimes).unwrap().into_inner().unwrap();

    let mut total: HashMap<String, f64> = HashMap::new();
    for (k, v) in results.iter_mut() {
        let true_pos = *v.get("true_pos").unwrap();
        let false_pos = *v.get("false_pos").unwrap();
        let false_neg = *v.get("false_neg").unwrap();

        let precision_den = true_pos + false_pos;
        let precision = if precision_den > 0.0 { true_pos / precision_den } else { 0.0 };

        let recall_den = true_pos + false_neg;
        let recall = if recall_den > 0.0 { true_pos / recall_den } else { 0.0 };

        let fscore_den = precision + recall;
        let fscore = if fscore_den > 0.0 { 2.0 * precision * recall / fscore_den } else { 0.0 };

        v.insert("precision".to_string(), precision);
        v.insert("recall".to_string(), recall);
        v.insert("fscore".to_string(), fscore);

        let runtime = runtimes.get_mut(k).unwrap();
        runtime.sort_unstable();
        let median_time = if runtime.len() % 2 == 1 {
            runtime[runtime.len() / 2] as f64
        } else {
            let mid = runtime.len() / 2;
            (runtime[mid - 1] as f64 + runtime[mid] as f64) / 2.0
        };
        v.insert("median_time".to_string(), median_time);

        *total.entry("true_pos".to_string()).or_default() += true_pos;
        *total.entry("false_pos".to_string()).or_default() += false_pos;
        *total.entry("false_neg".to_string()).or_default() += false_neg;
        *total.entry("precision".to_string()).or_default() += precision;
        *total.entry("recall".to_string()).or_default() += recall;
        *total.entry("fscore".to_string()).or_default() += fscore;
        *total.entry("median_time".to_string()).or_default() += median_time;
    }

    let count = results.iter().filter(|(_, v)| *v.get("true_pos").unwrap() > 0.0).count() as f64;
    *total.entry("precision".to_string()).or_default() /= count;
    *total.entry("recall".to_string()).or_default() /= count;
    *total.entry("fscore".to_string()).or_default() /= count;
    *total.entry("median_time".to_string()).or_default() /= count;

    results.insert("total".to_string(), total);

    let mut rows = results.keys().map(|s| s.as_str()).collect::<Vec<_>>();
    rows.sort_unstable();
    let cols = [
        "Heurictics",
        "true_pos",
        "false_pos",
        "false_neg",
        "precision",
        "recall",
        "fscore",
        "median_time",
    ];

    print_table(&results, &rows, &cols);
}

pub fn get_corners(symbols: &[&mut Symbol]) -> Vec<Vec<f64>> {
    let mut symbol_corners = Vec::with_capacity(100);
    for sym in symbols {
        let sz = sym.ver.width() as f64;

        let bl = match sym.raw_map(0.0, sz) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let tl = match sym.raw_map(0.0, 0.0) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let tr = match sym.raw_map(sz, 0.0) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let br = match sym.raw_map(sz, sz) {
            Ok(p) => p,
            Err(_) => continue,
        };

        symbol_corners.push(vec![bl.0, bl.1, tl.0, tl.1, tr.0, tr.1, br.0, br.1])
    }

    symbol_corners
}

fn match_areas(actual: &[Vec<f64>], expected: &[Vec<f64>]) -> usize {
    let mut matched = [false; 100];
    let actual = actual.to_vec();
    let mut res = 0;
    for actual_corners in actual.iter() {
        if expected.iter().enumerate().any(|(i, exp_corners)| {
            if matched[i] {
                return false;
            }
            let exp_area = quad_area(exp_corners);
            let overlap_area = overlap_area(actual_corners, exp_corners);
            let percent = overlap_area / exp_area;
            if percent > 0.2 {
                matched[i] = true;
                true
            } else {
                false
            }
        }) {
            res += 1;
        }
    }
    res
}

fn quad_area(quad: &[f64]) -> f64 {
    assert!(quad.len() == 8, "Expected 8 coordinates (4 points)");

    let x1 = quad[0];
    let y1 = quad[1];
    let x2 = quad[2];
    let y2 = quad[3];
    let x3 = quad[4];
    let y3 = quad[5];
    let x4 = quad[6];
    let y4 = quad[7];

    0.5 * ((x1 * y2 + x2 * y3 + x3 * y4 + x4 * y1) - (y1 * x2 + y2 * x3 + y3 * x4 + y4 * x1)).abs()
}

/// Converts a flat slice of 8 f64s into a Polygon
fn to_polygon(quad: &[f64]) -> Polygon<f64> {
    assert!(quad.len() == 8);
    let points = vec![
        Coord { x: quad[0], y: quad[1] },
        Coord { x: quad[2], y: quad[3] },
        Coord { x: quad[4], y: quad[5] },
        Coord { x: quad[6], y: quad[7] },
        Coord { x: quad[0], y: quad[1] }, // close the ring
    ];
    Polygon::new(points.into(), vec![])
}

/// Returns the overlap area between two quads
fn overlap_area(actual: &[f64], expected: &[f64]) -> f64 {
    let poly1 = to_polygon(actual);
    let poly2 = to_polygon(expected);

    let intersection = poly1.intersection(&poly2);
    intersection.unsigned_area()
}

fn main() {
    println!("Running Detection Benchmarks...");
    println!("---------------------------------");
    let detection_start = Instant::now();
    benchmark_detection(Path::new("benches/dataset/detection"));
    let detection_time = detection_start.elapsed();
    println!("Detection benchmark completed in: {:?}\n", detection_time);
}
