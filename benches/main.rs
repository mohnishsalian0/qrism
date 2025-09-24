use std::time::Instant;
use std::path::Path;

mod utils;
mod decoding;
mod detection;

fn main() {
    println!("🚀 Running QRism Benchmark Suite");
    println!("==================================\n");

    let total_start = Instant::now();

    // Run decoding benchmarks on blackbox dataset
    println!("📖 Running Decoding Benchmarks (Blackbox)...");
    println!("--------------------------------------------");
    let decoding1_start = Instant::now();
    decoding::benchmark_decoding(Path::new("benches/dataset/blackbox"));
    let decoding1_time = decoding1_start.elapsed();
    println!("Decoding (blackbox) benchmark completed in: {:?}\n", decoding1_time);

    // Run decoding benchmarks on decoding dataset
    println!("📖 Running Decoding Benchmarks (Decoding)...");
    println!("--------------------------------------------");
    let decoding2_start = Instant::now();
    decoding::benchmark_decoding(Path::new("benches/dataset/decoding"));
    let decoding2_time = decoding2_start.elapsed();
    println!("Decoding (decoding) benchmark completed in: {:?}\n", decoding2_time);

    // Run detection benchmarks  
    println!("🔍 Running Detection Benchmarks...");
    println!("---------------------------------");
    let detection_start = Instant::now();
    detection::benchmark_detection(Path::new("benches/dataset/detection"));
    let detection_time = detection_start.elapsed();
    println!("Detection benchmark completed in: {:?}\n", detection_time);

    let total_time = total_start.elapsed();
    println!("✅ All benchmarks completed!");
    println!("Total time elapsed: {:?}", total_time);
    println!("  - Decoding (blackbox): {:?}", decoding1_time);
    println!("  - Decoding (decoding): {:?}", decoding2_time);
    println!("  - Detection: {:?}", detection_time);
}