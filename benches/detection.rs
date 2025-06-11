use geo::{polygon, Area, BooleanOps, Coord, Polygon};
use geo_booleanop::boolean::BooleanOp;
use qrism::reader::get_corners;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

mod utils;
use utils::*;

fn benchmark(dataset_dir: &Path) {
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

        let gray = image::open(img_path).unwrap().to_luma8();

        let start = Instant::now();
        let symbols = get_corners(gray);
        let time = start.elapsed().as_millis();

        let true_pos = matched_areas(&symbols, &exp_symbols);
        let false_pos = symbols.len() - true_pos;
        let false_neg = exp_symbols.len() - true_pos;

        let path_str = img_path.to_str().unwrap();
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

        let precision = true_pos / (true_pos + false_pos);
        let recall = true_pos / (true_pos + false_neg);
        let fscore = 2.0 * precision * recall / (precision + recall);

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

    let count = results.len() as f64;
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

fn matched_areas(actual: &[Vec<f64>], expected: &[Vec<f64>]) -> usize {
    let mut matched = [false; 100];
    let actual = actual.to_vec();
    let mut res = 0;
    for actual_corners in actual.iter() {
        if expected.iter().enumerate().any(|(i, exp_corners)| {
            if matched[i] {
                return false;
            }
            let actual_area = quad_area(actual_corners);
            let exp_area = quad_area(exp_corners);
            let overlap_area = overlap_area(actual_corners, exp_corners);
            let percent = overlap_area / actual_area.min(exp_area);
            // let percent = overlap_area / exp_area;
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
    let dataset_dir = std::path::Path::new("benches/dataset/detection");

    let start = Instant::now();
    // test_get_corners();
    benchmark(dataset_dir);
    println!("time elapsed: {:?}", start.elapsed());
}
