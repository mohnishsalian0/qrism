use std::collections::HashMap;
use std::fmt::{Debug, Display};
use std::path::Path;

pub fn is_image_file(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_file()
        && entry
            .path()
            .extension()
            .map(|e| matches!(e.to_str(), Some("png" | "jpg" | "jpeg" | "bmp")))
            .unwrap_or(false)
}

pub fn get_parent(path: &Path) -> String {
    path.parent().and_then(|p| p.file_name()).and_then(|s| s.to_str()).unwrap().to_string()
}

pub fn parse_expected_decode_result(path: &Path) -> Vec<String> {
    let exp_msg = std::fs::read_to_string(path).unwrap();
    exp_msg.lines().map(String::from).collect()
}

pub fn parse_expected_bounds_result(path: &Path) -> Vec<Vec<f64>> {
    let mut exp_symbols = Vec::new();
    let content = std::fs::read_to_string(path).unwrap();

    // Collect all numbers from expected result
    let numbers: Vec<f64> = content
        .lines()
        .flat_map(|line| line.split_whitespace())
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();

    // Group into chunks of 8 (i.e., 4 points per QR)
    for chunk in numbers.chunks(8) {
        debug_assert!(chunk.len() == 8, "Less than 4 corners");
        exp_symbols.push((*chunk).to_vec());
    }
    exp_symbols
}

pub fn print_table<N>(result: &HashMap<String, HashMap<String, N>>, rows: &[&str], columns: &[&str])
where
    N: Display + Debug + Default,
{
    let cell_w = 15;
    let df = N::default();
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
            let cell = r.get(&c.to_string()).unwrap_or(&df);
            row.push_str(&format!("{:<cell_w$.2}| ", cell));
        }

        println!("{row}");
    }

    println!("{divider}");
}

fn main() {}
