use image::imageops::{resize, FilterType};
use image::{open, GrayImage, ImageBuffer, Pixel};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use walkdir::WalkDir;

use qrism::QRReader;

#[derive(Default)]
struct Score {
    pub true_pos: u128,
    pub false_pos: u128,
    pub false_neg: u128,
    pub time: u128,
}

impl Score {
    pub fn precision(&self) -> f64 {
        self.true_pos as f64 / (self.true_pos + self.false_pos) as f64
    }

    pub fn recall(&self) -> f64 {
        self.true_pos as f64 / (self.true_pos + self.false_neg) as f64
    }

    pub fn fscore(&self) -> f64 {
        2.0 * self.precision() * self.recall() / (self.precision() + self.recall())
    }
}

fn get_parent(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

fn is_image_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file()
        && entry
            .path()
            .extension()
            .map(|e| matches!(e.to_str(), Some("png" | "jpg" | "jpeg" | "bmp")))
            .unwrap_or(false)
}

fn load_grayscale<P: AsRef<Path>>(path: P) -> Option<GrayImage> {
    match open(&path) {
        Ok(img) => {
            let gray = img.to_luma8();
            let downscaled = downscale(gray);
            Some(downscaled)
        }
        Err(e) => {
            eprintln!("Failed to open {}: {}", path.as_ref().display(), e);
            None
        }
    }
}

// Downscales image if bigger than 1000px x 1000px
fn downscale<I>(img: ImageBuffer<I, Vec<I::Subpixel>>) -> ImageBuffer<I, Vec<I::Subpixel>>
where
    I: Pixel<Subpixel = u8> + 'static,
{
    let (max_w, max_h) = (1000, 1000);
    let (w, h) = img.dimensions();

    if w <= max_w && h <= max_h {
        return img;
    }

    let scale = f32::min(max_w as f32 / w as f32, max_h as f32 / h as f32);
    let new_w = (w as f32 * scale).round() as u32;
    let new_h = (h as f32 * scale).round() as u32;

    resize(&img, new_w, new_h, FilterType::Triangle)
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

fn print_table(result: &HashMap<String, Score>) {
    let cell_w = 15;
    let columns = [
        "Heurictics",
        "True Pos",
        "False Pos",
        "False Neg",
        "Precision",
        "Recall",
        "Fscore",
        "Time",
    ];
    let divider = "-".repeat(columns.len() * (cell_w + 1) + 1);

    println!("{divider}");
    let mut header = String::from("|");
    for c in columns {
        header.push_str(&format!("{c:<cell_w$}|"));
    }
    println!("{header}");
    println!("{divider}");

    for (h, s) in result.iter() {
        let mut row = format!("|{h:<cell_w$}|");

        row.push_str(&format!("{:<cell_w$}|", s.true_pos));
        row.push_str(&format!("{:<cell_w$}|", s.false_pos));
        row.push_str(&format!("{:<cell_w$}|", s.false_neg));
        row.push_str(&format!("{:<cell_w$.2}|", s.precision()));
        row.push_str(&format!("{:<cell_w$.2}|", s.recall()));
        row.push_str(&format!("{:<cell_w$.2}|", s.fscore()));
        row.push_str(&format!("{:<cell_w$}|", s.time));

        println!("{row}");
    }

    println!("{divider}");
}

fn test_get_corners() {
    let img_path = Path::new("benches/dataset/detection/blurred/image022.jpg");

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
    let symbols = QRReader::get_corners(img).expect("Couldnt detect QR");

    let mut detected = 0;
    let mut score = [0; 3];
    for corners in symbols.iter() {
        if exp_symbols.iter().any(|exp_corners| {
            exp_corners.iter().zip(corners).all(|(&a, &e)| (a - e).abs() * 10.0 <= e)
        }) {
            detected += 1;
            score[0] += 1;
        } else {
            score[2] += 1;
        }
        score[1] += symbols.len() - detected;
    }

    let precision = score[0] as f64 / (score[0] + score[1]) as f64;
    let recall = score[0] as f64 / (score[0] + score[2]) as f64;

    println!("Score: {:?}", score);
    println!("Precision: {}", precision);
    println!("Recall: {}", recall);
}

fn benchmark_detection() {
    let dataset_dir = "benches/dataset/detection";
    let mut results: HashMap<String, Score> = HashMap::new();

    for entry in WalkDir::new(dataset_dir).into_iter().filter_map(Result::ok).filter(is_image_file)
    {
        let img_path = entry.path();
        let parent = get_parent(img_path);
        let score = results.entry(parent).or_default();

        println!("Reading {:?}", img_path);

        // Corresponding expected result file
        let exp_path = img_path.with_extension("txt");
        let exp_symbols = parse_expected_result(&exp_path);

        // Benchmark image
        let gray = load_grayscale(img_path).unwrap();
        let start = Instant::now();
        let res = QRReader::get_corners(gray);
        let time = start.elapsed().as_millis();

        score.time += time;

        // Comparing results if detection succesful
        let mut detected = 0;
        if let Ok(symbols) = res {
            for corners in symbols.iter() {
                if exp_symbols.iter().any(|exp_corners| {
                    exp_corners.iter().zip(corners).all(|(&a, &e)| (a - e).abs() * 10.0 <= e)
                }) {
                    detected += 1;
                    score.true_pos += 1;
                } else {
                    score.false_neg += 1;
                }
            }
            score.false_pos += symbols.len() as u128 - detected;
        }
    }
    print_table(&results);
}

fn main() {
    benchmark_detection();
}
