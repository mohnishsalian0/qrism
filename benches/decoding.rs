use rayon::prelude::*;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use walkdir::WalkDir;

use qrism::reader::detect_qr;
use crate::utils::{get_parent, is_image_file, parse_expected_decode_result, print_table};

pub fn benchmark_decoding(dataset_dir: &Path) {
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
        let _path_str = img_path.to_str().unwrap();

        let img = image::open(img_path).unwrap().to_luma8();
        for angle in [0, 90, 180, 270].iter() {
            let img = image::DynamicImage::ImageLuma8(match angle {
                90 => image::imageops::rotate90(&img),
                180 => image::imageops::rotate180(&img),
                270 => image::imageops::rotate270(&img),
                _ => img.clone(),
            });

            let start = Instant::now();
            let mut res = detect_qr(&img);
            let mut _passed = false;

            if !res.symbols().is_empty() {
                if let Ok((_meta, msg)) = res.symbols()[0].decode() {
                    let elapsed = start.elapsed();

                    let mut runtimes = runtimes.lock().unwrap();
                    runtimes.entry(parent.clone()).or_default().push(elapsed.as_millis());

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

                        _passed = true;
                    }
                }
            }

            if _passed {
                println!("\x1b[1;32m[PASS]\x1b[0m {} at {}deg", _path_str, angle);
            } else {
                println!("\x1b[1;31m[FAIL]\x1b[0m {} at {}deg", _path_str, angle);
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

    let mut rows = results.keys().map(|s| s.as_str()).collect::<Vec<_>>();
    rows.sort_unstable();
    let cols = ["Angles", "0", "90", "180", "270", "total", "median_time", "avg_time"];

    println!("\nResult:");
    print_table(&results, &rows, &cols);
}
