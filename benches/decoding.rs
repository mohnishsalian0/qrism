use rayon::{prelude::*, ThreadPoolBuilder};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

use qrism::reader::binarize::BinaryImage;
use qrism::QRReader;

mod utils;
use utils::*;

fn benchmark(dataset_dir: &Path, rows: &[&str], cols: &[&str]) {
    let image_paths: Vec<_> = WalkDir::new(dataset_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(is_image_file)
        .map(|e| e.path().to_path_buf())
        .collect();

    let results = Arc::new(Mutex::new(HashMap::<String, HashMap<String, u128>>::new()));
    let runtimes = Arc::new(Mutex::new(HashMap::<String, Vec<u128>>::new()));

    image_paths.par_iter().for_each(|img_path| {
        let parent = get_parent(img_path);
        let path_str = img_path.to_str().unwrap();

        let gray = load_grayscale(img_path).unwrap();
        for angle in [0, 90, 180, 270].iter() {
            let img = match angle {
                90 => image::imageops::rotate90(&gray),
                180 => image::imageops::rotate180(&gray),
                270 => image::imageops::rotate270(&gray),
                _ => gray.clone(),
            };

            let start = Instant::now();
            let mut img = BinaryImage::binarize(&img);
            let mut symbols = QRReader::detect(&mut img);

            if symbols.is_empty() {
                println!("\x1b[1;31m[FAIL]\x1b[0m {}", path_str);
                continue;
            }

            match symbols[0].decode() {
                Ok((_meta, msg)) => {
                    let elapsed = start.elapsed();

                    let mut runtimes = runtimes.lock().unwrap();
                    runtimes.entry(parent.clone()).or_default().push(elapsed.as_micros());

                    let msg = msg.lines().map(String::from).collect::<Vec<_>>();

                    // Corresponding expected result file
                    let exp_path = img_path.with_extension("txt");
                    let exp_msg = parse_expected_decode_result(&exp_path);

                    if msg == exp_msg {
                        let mut results = results.lock().unwrap();
                        *results
                            .entry(parent.clone())
                            .or_default()
                            .entry(angle.to_string())
                            .or_default() += 1;

                        println!("\x1b[1;32m[PASS]\x1b[0m {}", path_str);
                    } else {
                        println!("\x1b[1;31m[FAIL]\x1b[0m {}", path_str);
                    };
                }
                Err(_) => {
                    println!("\x1b[1;31m[FAIL]\x1b[0m {}", path_str);
                    continue;
                }
            }
        }
    });

    // Remaining aggregation logic (same as original)
    let mut results = Arc::try_unwrap(results).unwrap().into_inner().unwrap();
    let mut runtimes = Arc::try_unwrap(runtimes).unwrap().into_inner().unwrap();

    // Calculate total successes and median time for each folder/heuristic
    let mut total: HashMap<String, u128> = HashMap::new();
    for (k, v) in results.iter_mut() {
        let total_for_folder = v.values().sum::<u128>();
        *v.entry("total".to_string()).or_default() = total_for_folder;

        let runtime = runtimes.get_mut(k).unwrap();
        runtime.sort_unstable();
        let median_time = if runtime.len() % 2 == 1 {
            runtime[runtime.len() / 2]
        } else {
            let mid = runtime.len() / 2;
            (runtime[mid - 1] + runtime[mid]) / 2
        };
        let avg_time = runtime.iter().sum::<u128>() / runtime.len() as u128;
        v.insert("median_time".to_string(), median_time);
        v.insert("avg_time".to_string(), avg_time);

        for (kc, vc) in v.iter() {
            *total.entry(kc.to_string()).or_default() += vc;
        }
    }
    *total.get_mut("median_time").unwrap() /= results.len() as u128;
    *total.get_mut("avg_time").unwrap() /= results.len() as u128;
    results.insert("total".to_string(), total);

    println!("\nResult:");
    print_table(&results, rows, cols);
}

fn main() {
    ThreadPoolBuilder::new().num_threads(8).build_global().unwrap();

    let dataset_dir = std::path::Path::new("benches/dataset/blackbox");
    let rows = ["qrcode-1", "qrcode-2", "qrcode-3", "qrcode-4", "qrcode-5", "qrcode-6", "total"];
    // let dataset_dir = std::path::Path::new("benches/dataset/decoding");
    // let rows = ["decoding"];
    let cols = ["Angles", "0", "90", "180", "270", "total", "median_time", "avg_time"];

    let start = Instant::now();
    benchmark(dataset_dir, &rows, &cols);
    println!("Time elapsed: {:?}", start.elapsed());
}
