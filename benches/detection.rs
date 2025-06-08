use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

use qrism::QRReader;

mod utils;
use utils::*;

fn test_get_corners() {
    let img_path = Path::new("benches/dataset/detection/monitor/image001.jpg");

    // Corresponding expected result file
    let exp_res_path = img_path.with_extension("txt");
    let mut exp_symbols = Vec::new();

    if let Ok(contents) = std::fs::read_to_string(&exp_res_path) {
        // Collect all numbers from expected result
        let numbers: Vec<f64> = contents
            .lines()
            .flat_map(|line| line.split_whitespace())
            .filter_map(|s| s.parse::<f64>().ok())
            .collect();

        // Group into chunks of 8 (i.e., 4 points per QR)
        for chunk in numbers.chunks(8) {
            debug_assert!(chunk.len() == 8, "Less than 4 corners");
            exp_symbols.push((*chunk).to_vec());
        }
    }

    let img = image::open(img_path).unwrap().to_luma8();
    let symbols = QRReader::get_corners(img);

    let mut detected = 0;
    let mut score = [0; 3];
    for corners in symbols.iter() {
        if exp_symbols.iter().any(|exp_corners| {
            exp_corners.iter().zip(corners).all(|(&a, &e)| (a - e).abs() * 10.0 <= e)
        }) {
            detected += 1;
        }
    }
    score[0] = detected;
    score[1] = symbols.len() - detected;
    score[2] = exp_symbols.len() - detected;

    let precision = score[0] as f64 / (score[0] + score[1]) as f64;
    let recall = score[0] as f64 / (score[0] + score[2]) as f64;

    println!("Score: {:?}", score);
    println!("Precision: {}", precision);
    println!("Recall: {}", recall);
}

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
        let path_str = img_path.to_str().unwrap();
        let parent = get_parent(img_path);

        let exp_path = img_path.with_extension("txt");
        let exp_symbols = parse_expected_bounds_result(&exp_path);

        let gray = load_grayscale(img_path).unwrap();

        let start = Instant::now();
        let mut symbols = QRReader::get_corners(gray);
        let time = start.elapsed().as_millis();

        let mut true_pos = 0;
        let mut false_pos = 0;
        for symbol in symbols.iter_mut() {
            let mut corners = *symbol;
            let mut matched = false;
            for _ in 0..4 {
                if exp_symbols.iter().any(|exp_corners| {
                    exp_corners.iter().zip(corners).all(|(a, e)| (*a - e).abs() * 10.0 <= e)
                }) {
                    true_pos += 1;
                    matched = true;
                    println!("\x1b[1;32m[PASS]\x1b[0m {}", path_str);
                    break;
                }
                corners.rotate_left(2);
            }
            if !matched {
                false_pos += 1;
                println!("\x1b[1;31m[FAIL]\x1b[0m {}", path_str);
            }
        }

        let mut results = results.lock().unwrap();
        let mut runtimes = runtimes.lock().unwrap();

        let score = results.entry(parent.clone()).or_default();
        *score.entry("true_pos".to_string()).or_default() += true_pos as f64;
        *score.entry("false_pos".to_string()).or_default() += false_pos as f64;
        *score.entry("false_neg".to_string()).or_default() += (exp_symbols.len() - true_pos) as f64;

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

fn main() {
    let dataset_dir = std::path::Path::new("benches/dataset/detection");

    let start = Instant::now();
    // test_get_corners();
    benchmark(dataset_dir);
    println!("time elapsed: {:?}", start.elapsed());
}
