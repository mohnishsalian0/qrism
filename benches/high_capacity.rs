use rayon::prelude::*;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};
use walkdir::WalkDir;

use qrism::detect_hc_qr;

#[path = "utils/mod.rs"]
mod utils;
use utils::{get_parent, is_image_file, print_table};

use crate::utils::get_file_name;

const COLORS: [&str; 8] = ["black", "blue", "green", "cyan", "red", "magenta", "yellow", "white"];

const BLUE: usize = 0;
const GREEN: usize = 1;
const RED: usize = 2;

type Matrix = Vec<Vec<u8>>;

#[derive(Default)]
struct Stat {
    heuristic: String,
    ver: String,
    ec: String,
    size: String,
    detected: bool,
    decoded: bool,
    chan_conf: [[[u64; 2]; 2]; 3],
    conf: [[u64; 8]; 8],
}

impl Stat {
    fn evaluate(img_path: &Path, source: &HashMap<String, Matrix>) -> Self {
        let mut stat = Stat::default();

        let parent = get_parent(img_path);
        stat.heuristic = parent;

        let name = get_file_name(img_path);
        let mut name_parts = name.split('_').next().unwrap().split('-');
        stat.ver = name_parts.next().unwrap().to_owned();
        stat.ec = name_parts.next().unwrap().to_owned();
        stat.size = name_parts.next().unwrap().to_owned();

        let img = image::open(img_path).unwrap();

        if let Some(symbol) = detect_hc_qr(&img).symbols().first_mut() {
            stat.detected = true;
            if let Ok(recons) = symbol.reconstruct() {
                let source_name = format!("{}-{}", stat.ver, stat.ec);
                let source_symbol = source.get(&source_name).unwrap();

                color_accuracy(&recons, source_symbol, &mut stat);
            };
            if symbol.decode().is_ok() {
                stat.decoded = true;
            }
        };

        stat
    }
}

fn color_accuracy(actual: &[Vec<u8>], exp: &[Vec<u8>], stat: &mut Stat) {
    let Stat { chan_conf, conf, .. } = stat;
    for (&a, &e) in actual.iter().flatten().zip(exp.iter().flatten()) {
        conf[e as usize][a as usize] += 1;
        for (i, ch) in chan_conf.iter_mut().enumerate() {
            let (ab, eb) = ((a >> i) & 1, (e >> i) & 1);
            ch[eb as usize][ab as usize] += 1;
        }
    }
}

#[derive(Default)]
struct Aggregate {
    total: u64,
    detected: u64,
    decoded: u64,
    chan_conf: [[[u64; 2]; 2]; 3],
    conf: [[u64; 8]; 8],
}

impl Aggregate {
    fn accumulate(&mut self, stat: &Stat) {
        self.total += 1;
        self.detected += stat.detected as u64;
        self.decoded += stat.decoded as u64;

        for i in 0..3 {
            for j in 0..2 {
                for k in 0..2 {
                    self.chan_conf[i][j][k] += stat.chan_conf[i][j][k];
                }
            }
        }

        for i in 0..8 {
            for j in 0..8 {
                self.conf[i][j] += stat.conf[i][j];
            }
        }
    }

    fn merge(&mut self, other: &Aggregate) {
        self.total += other.total;
        self.detected += other.detected;
        self.decoded += other.decoded;

        for i in 0..3 {
            for j in 0..2 {
                for k in 0..2 {
                    self.chan_conf[i][j][k] += other.chan_conf[i][j][k];
                }
            }
        }

        for i in 0..8 {
            for j in 0..8 {
                self.conf[i][j] += other.conf[i][j];
            }
        }
    }

    fn to_table_row(&self) -> HashMap<String, f64> {
        let mut row = HashMap::new();

        row.insert("total".to_string(), self.total as f64);
        row.insert("detected".to_string(), self.detected as f64);
        row.insert("decoded".to_string(), self.decoded as f64);

        row.insert("red".to_string(), channel_accuracy(&self.chan_conf[RED]));
        row.insert("green".to_string(), channel_accuracy(&self.chan_conf[GREEN]));
        row.insert("blue".to_string(), channel_accuracy(&self.chan_conf[BLUE]));

        row
    }
}

fn channel_accuracy(m: &[[u64; 2]; 2]) -> f64 {
    (m[0][0] + m[1][1]) as f64 * 100.0 / (m[0][0] + m[0][1] + m[1][0] + m[1][1]) as f64
}

fn channel_recall(m: &[[u64; 2]; 2]) -> f64 {
    m[1][1] as f64 * 100.0 / (m[1][0] + m[1][1]) as f64
}

pub fn benchmark_high_capacity(dataset_dir: &Path) {
    let (source_paths, image_paths): (Vec<PathBuf>, Vec<PathBuf>) = WalkDir::new(dataset_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(is_image_file)
        .map(|e| e.into_path())
        .partition(|p| get_parent(p) == "source");

    let source: HashMap<String, Matrix> = source_paths
        .par_iter()
        .map(|src_path| {
            let img = image::open(src_path).unwrap();
            let recons = detect_hc_qr(&img)
                .symbols()
                .first_mut()
                .unwrap_or_else(|| panic!("No symbol in source {}", src_path.display()))
                .reconstruct()
                .unwrap_or_else(|e| {
                    panic!("source {} failed to reconstruct: {e:?}", src_path.display())
                });
            (get_file_name(src_path), recons)
        })
        .collect();

    // Change to iter() for sequential, change to par_iter() for parallel
    let stats: Vec<Stat> =
        image_paths.par_iter().map(|img_path| Stat::evaluate(img_path, &source)).collect();

    let heuristic_pivot = pivot(&stats, |stat: &Stat| stat.heuristic.clone());

    let total_agg = heuristic_pivot.values().fold(Aggregate::default(), |mut acc, a| {
        acc.merge(a);
        acc
    });

    let conf_table = confusion_matrix_to_table(&total_agg.conf);

    let cols = ["Heuristics", "total", "detected", "decoded", "red", "green", "blue"];

    let table: HashMap<String, HashMap<String, f64>> =
        heuristic_pivot.iter().map(|(k, v)| (k.to_string(), v.to_table_row())).collect();
    let mut rows = table.keys().map(|s| s.as_str()).collect::<Vec<_>>();
    rows.sort_unstable();
    print_table(&table, &rows, &cols);

    let mut total_table: HashMap<String, HashMap<String, f64>> = HashMap::new();
    total_table.insert("total".to_string(), total_agg.to_table_row());
    print_table(&total_table, &["total"], &cols);

    let cols: Vec<_> = std::iter::once("Confusion").chain(COLORS).collect();
    print_table(&conf_table, &COLORS, &cols);
}

fn pivot(stats: &[Stat], key: impl Fn(&Stat) -> String) -> HashMap<String, Aggregate> {
    stats.iter().fold(HashMap::new(), |mut acc, s| {
        let attribute = acc.entry(key(s)).or_default();
        attribute.accumulate(s);
        acc
    })
}

fn confusion_matrix_to_table(conf: &[[u64; 8]; 8]) -> HashMap<String, HashMap<String, f64>> {
    let row_agg: Vec<u64> = conf.iter().map(|row| row.iter().sum()).collect();
    conf.iter()
        .enumerate()
        .map(|(r, row)| {
            (
                COLORS[r].to_string(),
                row.iter()
                    .enumerate()
                    .map(|(c, v)| {
                        (COLORS[c].to_string(), ((*v as f64 * 100.0) / row_agg[r] as f64))
                    })
                    .collect(),
            )
        })
        .collect()
}

fn main() {
    println!("Running High Capacity Benchmarks...");
    println!("-----------------------------------");
    let benchmark_start = Instant::now();
    benchmark_high_capacity(Path::new("benches/dataset/high_capacity/qrism"));
    let benchmark_time = benchmark_start.elapsed();
    println!("High capacity benchmark completed in: {:?}\n", benchmark_time);
}
