use image::imageops::{self};
use image::{open, GrayImage};
use qrism::reader::binarize::BinaryImage;
use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::path::Path;
use std::time::Instant;
use walkdir::WalkDir;

use qrism::QRReader;

fn is_image_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file()
        && entry
            .path()
            .extension()
            .map(|e| matches!(e.to_str(), Some("png" | "jpg" | "jpeg" | "bmp")))
            .unwrap_or(false)
}

fn get_parent(path: &Path) -> String {
    path.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap().to_string()
}

fn load_grayscale<P: AsRef<Path>>(path: P) -> Option<GrayImage> {
    match open(&path) {
        Ok(img) => Some(img.to_luma8()),
        Err(e) => {
            eprintln!("Failed to open {}: {}", path.as_ref().display(), e);
            None
        }
    }
}

fn parse_decode_expected_result(path: &Path) -> Vec<String> {
    let exp_msg = std::fs::read_to_string(path).unwrap();
    exp_msg.lines().map(String::from).collect()
}

fn print_table<N>(result: &HashMap<String, HashMap<String, N>>, rows: &[&str], columns: &[&str])
where
    N: Display + Debug + Default,
{
    let cell_w = 15;
    let divider = "-".repeat(columns.len() * (cell_w + 2) + 1);

    println!("{divider}");
    let mut header = String::from("| ");
    for c in columns {
        header.push_str(&format!("{c:<cell_w$}| "));
    }
    println!("{header}");
    println!("{divider}");

    for hr in rows {
        let r = result.get(&hr.to_string()).unwrap();
        let mut row = format!("| {hr:<cell_w$}| ");

        for c in columns.iter().skip(1) {
            let cell = r.get(&c.to_string()).unwrap();
            row.push_str(&format!("{:<cell_w$.2}| ", cell));
        }

        println!("{row}");
    }

    println!("{divider}");
}

fn test_qr_detection() {
    use std::io::Write;

    let dataset_dir = "benches/dataset/blackbox";
    let mut out_file = std::fs::File::create("benches/dataset/blackbox_result.txt").unwrap();
    let mut time: HashMap<String, HashMap<String, u128>> = HashMap::new();
    let mut passed: HashMap<String, HashMap<String, u128>> = HashMap::new();
    let mut last_folder = "None".to_string();

    for entry in WalkDir::new(dataset_dir).into_iter().filter_map(Result::ok).filter(is_image_file)
    {
        let img_path = entry.path();
        let parent = get_parent(img_path);
        let file_name = img_path.file_stem().unwrap().to_str().unwrap().to_string();

        if parent != last_folder {
            last_folder = parent.clone();
            println!("Reading QRs from {}...", last_folder);
        }

        let gray = load_grayscale(img_path).unwrap();
        for angle in [0, 90, 180, 270].iter() {
            let img = match angle {
                90 => imageops::rotate90(&gray),
                180 => imageops::rotate180(&gray),
                270 => imageops::rotate270(&gray),
                _ => gray.clone(),
            };

            write!(out_file, "[{}/{}-{}] ", parent, file_name, angle).unwrap();

            let start = Instant::now();
            let mut img = BinaryImage::prepare(&img);
            let mut symbols = QRReader::detect(&mut img);

            if symbols.is_empty() {
                write!(out_file, "QR not found").unwrap();
                continue;
            }

            match symbols[0].decode() {
                Ok((_meta, msg)) => {
                    let elapsed = start.elapsed();
                    *time
                        .entry(parent.clone())
                        .or_default()
                        .entry(angle.to_string())
                        .or_default() += elapsed.as_millis();
                    *time
                        .entry("total".to_string())
                        .or_default()
                        .entry(angle.to_string())
                        .or_default() += elapsed.as_millis();

                    let msg = msg.lines().map(String::from).collect::<Vec<_>>();

                    // Corresponding expected result file
                    let exp_path = img_path.with_extension("txt");
                    let exp_msg = parse_decode_expected_result(&exp_path);

                    if msg == exp_msg {
                        *passed
                            .entry(parent.clone())
                            .or_default()
                            .entry(angle.to_string())
                            .or_default() += 1;
                        *passed
                            .entry("total".to_string())
                            .or_default()
                            .entry(angle.to_string())
                            .or_default() += 1;

                        write!(out_file, "PASSED").unwrap();
                    } else {
                        write!(out_file, "DECODED").unwrap();
                    };
                }
                Err(e) => {
                    write!(out_file, "{}", e).unwrap();
                    continue;
                }
            }
            writeln!(out_file).unwrap();
        }
    }

    let rows = ["qrcode-1", "qrcode-2", "qrcode-3", "qrcode-4", "qrcode-5", "qrcode-6", "total"];
    let columns = ["Angles", "0", "90", "180", "270", "total"];

    // Print results table
    println!("Success result for 720 images:");
    for v in passed.values_mut() {
        let total_for_folder = v.values().sum::<u128>();
        *v.entry("total".to_string()).or_default() = total_for_folder;
    }
    print_table(&passed, &rows, &columns);

    // Print bench table
    println!("Benchmark result for 720 images:");
    for (kr, vr) in time.iter_mut() {
        let mut total_for_folder = 0;
        for (kc, vc) in vr.iter_mut() {
            *vc /= *passed.get(kr).unwrap().get(kc).unwrap();
            total_for_folder += *vc;
        }
        *vr.entry("total".to_string()).or_default() = total_for_folder / vr.len() as u128;
    }
    print_table(&time, &rows, &columns);
}

fn parse_expected_result(path: &Path) -> Vec<Vec<f64>> {
    let mut exp_symbols = Vec::new();

    if let Ok(contents) = std::fs::read_to_string(path) {
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
    exp_symbols
}

fn test_get_corners() {
    let img_path = Path::new("benches/dataset/detection/rotations/image001.jpg");

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

fn benchmark_detection() {
    let dataset_dir = "benches/dataset/detection";
    let mut results: HashMap<String, HashMap<String, f64>> = HashMap::new();
    let mut last_folder = "None".to_string();

    for entry in WalkDir::new(dataset_dir).into_iter().filter_map(Result::ok).filter(is_image_file)
    {
        let img_path = entry.path();
        let parent = get_parent(img_path);
        let score = results.entry(parent.clone()).or_default();

        if parent != last_folder {
            last_folder = parent.clone();
            println!("Reading QRs from {}...", last_folder);
        }

        // Corresponding expected result file
        let exp_path = img_path.with_extension("txt");
        let exp_symbols = parse_expected_result(&exp_path);

        // Benchmark image
        let gray = load_grayscale(img_path).unwrap();
        let start = Instant::now();
        let symbols = QRReader::get_corners(gray);
        let time = start.elapsed().as_millis();

        *score.entry("time".to_string()).or_default() += time as f64;

        // Comparing results if detection succesful
        let mut true_pos = 0;
        let mut false_pos = 0;
        for corners in symbols.iter() {
            if exp_symbols.iter().any(|exp_corners| {
                exp_corners.iter().zip(corners).all(|(&a, &e)| (a - e).abs() * 10.0 <= e)
            }) {
                true_pos += 1;
            } else {
                false_pos += 1;
            }
        }
        *score.entry("true_pos".to_string()).or_default() += true_pos as f64;
        *score.entry("false_pos".to_string()).or_default() += false_pos as f64;
        *score.entry("false_neg".to_string()).or_default() += (exp_symbols.len() - true_pos) as f64;
    }
    for (_k, v) in results.iter_mut() {
        let true_pos = *v.get("true_pos").unwrap();
        let false_pos = *v.get("false_pos").unwrap();
        let false_neg = *v.get("false_neg").unwrap();

        let precision = true_pos / (true_pos + false_pos);
        let recall = true_pos / (true_pos + false_neg);
        let fscore = 2.0 * precision * recall / (precision + recall);

        *v.entry("precision".to_string()).or_default() = precision;
        *v.entry("recall".to_string()).or_default() = recall;
        *v.entry("fscore".to_string()).or_default() = fscore;
    }

    let rows = [
        "blurred",
        "bright_spots",
        "brightness",
        "close",
        "curved",
        "damaged",
        "glare",
        "lots",
        "monitor",
        "nominal",
        "noncompliant",
        "pathological",
        "rotations",
        "shadows",
    ];
    let columns = [
        "Heurictics",
        "true_pos",
        "false_pos",
        "false_neg",
        "precision",
        "recall",
        "fscore",
        "time",
    ];

    print_table(&results, &rows, &columns);
}

fn main() {
    benchmark_detection();
    // test_get_corners();
    // test_qr_detection();
}
