//! LLR recording and error-correction replay.
//!
//! Answers "would a different error-correcting code help?" without changing the format or
//! re-photographing anything. Two passes:
//!
//! 1. **Record** (`llr-record`) runs the best colour pipeline over the whole `hiq` dataset and
//!    writes, for every data module, its colour posterior over the eight palette colours and
//!    its true colour, in the order the encoder places codeword bits. The real decode outcome
//!    of each layer is stored alongside, so the replay can be checked against the reader.
//!
//! 2. **Replay** (`llr-replay`) reads that back, calibrates the posteriors, reports how
//!    trustworthy they are, and pushes each layer's recorded channel through several decoders
//!    at the layer's own rate.
//!
//! # Replaying a channel into a different code
//!
//! A recorded module says how strongly the observation favoured the truth, not just whether
//! it was right. Per layer bit that is the *truth-relative* LLR `y`: positive when the
//! observation leaned towards the bit actually printed. A replayed code sends its own bit `b`
//! at the same position and receives `(1 - 2b) * y`, which keeps every error where the camera
//! put it — the same clusters, the same confident mistakes — while the bits being carried
//! change. What it gives up is any dependence of the noise on the printed colour: a code that
//! prints different bits would put different colours at each module, and those are not
//! equally hard to read. QR masking already makes the printed bits look random, so this is a
//! modest approximation, but it is one.
//!
//! Reed-Solomon is replayed on the real block layout with the reader's own decoder, and its
//! hard decisions are the classifier's winning colour exactly as the reader sees them, so its
//! row must agree with the recorded decode outcome. The report checks that first.
//!
//! Run with:
//!   cargo bench --features benchmark --bench color -- llr-record
//!   cargo bench --features benchmark --bench color -- llr-replay

mod ldpc;
mod rs;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use rand::rngs::StdRng;
use rand::SeedableRng;
use rayon::prelude::*;

use crate::analysis::{load_references, photos, sample_grid, BEST_BETA, FOLDERS};
use crate::deblur::Deblur;
use crate::hiq::{decode_layers, layer_ec_levels, Layer};
use crate::normalization::{Identity, Normalizer};
use crate::recovery::mahalanobis::Mahalanobis;
use qrism::bench_hooks::data_region;
use qrism::{ECLevel, Version};

use ldpc::Ldpc;
use rs::{Ranking, RsLayout, RsVariant};

const RECORD_PATH: &str = "target/llr/hiq.bin";
const MAGIC: &[u8; 4] = b"QLR2";

/// Log-posteriors are stored relative to the winner, quantized to this many steps per nat,
/// so the byte ceiling is ~64 nats. The raw Gaussian posteriors are several times
/// overconfident — the fitted scale comes out near 0.3 — so the ceiling has to sit well past
/// the ~10 nats that matter *after* calibration, or the top of the table collapses into one bin.
const QUANT: f64 = 4.0;

/// A module whose seven losing colours all sit at the ceiling is stored as its header byte
/// alone. Most modules are like that, which keeps the recording a few hundred MB.
const COMPACT: u8 = 0x40;

/// Records decoded and scored together. Bounds memory without starving the thread pool.
const BATCH: usize = 64;

/// Every n-th module feeds the calibration fit. The fit has one parameter.
const CALIB_STRIDE: usize = 16;

/// Bit-LLR magnitude bins for the calibration table, in nats.
const LLR_BINS: [f64; 10] = [0.0, 0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0];

/// Layer decode outcome codes, as stored.
const ACT_OK: u8 = 0;
const ACT_NOSYM: u8 = 1;
const ACT_BADFMT: u8 = 2;
const ACT_BADPLD: u8 = 3;
const ACT_WRONG: u8 = 4;

// Recording
//------------------------------------------------------------------------------

/// One symbol as recorded: its version, per-layer EC levels and real decode outcomes, and for
/// every data-region module the truth, the classifier's winner and the log-posteriors.
struct Record {
    ver: usize,
    ecl: [ECLevel; 3],
    actual: [u8; 3],
    truth: Vec<u8>,
    win: Vec<u8>,
    /// `ln p(c) - ln p(winner)` per colour, so the winner is 0 and the rest are negative.
    lp: Vec<[f32; 8]>,
}

pub fn record() {
    println!("\n\n########## LLR record ##########");
    let refs = load_references();

    // Layer EC levels and data regions depend only on the version.
    let meta: BTreeMap<usize, ([ECLevel; 3], Vec<(i32, i32)>)> = refs
        .iter()
        .map(|(&v, r)| {
            let ver = Version::Normal(v);
            (v, (layer_ec_levels(&r.truth, ver), data_region(ver)))
        })
        .collect();

    std::fs::create_dir_all(Path::new(RECORD_PATH).parent().unwrap()).unwrap();
    let mut out = BufWriter::new(File::create(RECORD_PATH).unwrap());
    out.write_all(MAGIC).unwrap();

    let (mut written, mut skipped, mut bytes) = (0usize, 0usize, 4usize);
    for folder in FOLDERS {
        let files = photos(folder);
        let recs: Vec<Option<Vec<u8>>> = files
            .par_iter()
            .map(|p| {
                let s = sample_grid(p, &refs).ok()?;
                let r = &refs[&s.version];
                let (ecl, region) = &meta[&s.version];
                Some(record_one(s.version, &s.rgb, &r.truth, &r.groups, &r.messages, *ecl, region))
            })
            .collect();
        for r in recs {
            match r {
                Some(buf) => {
                    out.write_all(&buf).unwrap();
                    bytes += buf.len();
                    written += 1;
                }
                None => skipped += 1,
            }
        }
        println!("  folder {folder}: {} photos, {written} recorded so far", files.len());
    }
    out.flush().unwrap();
    println!(
        "\n  wrote {written} symbols ({skipped} skipped: not localized or no reference), \
         {:.1} MB -> {RECORD_PATH}",
        bytes as f64 / 1e6
    );
}

/// Classifies one photo with the best pipeline, decodes its layers for the reference
/// outcome, and serializes the record.
fn record_one(
    version: usize,
    rgb: &[Vec<[f64; 3]>],
    truth: &[Vec<qrism::Color>],
    groups: &[Vec<(i32, i32)>; 8],
    messages: &[String; 3],
    ecl: [ECLevel; 3],
    region: &[(i32, i32)],
) -> Vec<u8> {
    let ver = Version::Normal(version);
    let grid = rgb.len();

    // Same fit as `analysis::best_with_conf`, so the recorded hard decisions are the ones the
    // erasure experiment in the accuracy pass decodes.
    let db = Deblur::fixed(BEST_BETA);
    let dg = db.apply(rgb);
    let norm = Identity;
    let ng = norm.normalize_groups(&db.sample_groups(rgb, groups));
    let rec = Mahalanobis::fit_rda(&ng);

    let ll: Vec<Vec<[f64; 8]>> = (0..grid)
        .map(|gy| (0..grid).map(|gx| rec.log_likelihoods(norm.apply(dg[gy][gx]))).collect())
        .collect();
    let argmax = |l: &[f64; 8]| (0..8).max_by(|&a, &b| l[a].total_cmp(&l[b])).unwrap();
    let pred: Vec<Vec<qrism::Color>> = ll
        .iter()
        .map(|row| row.iter().map(|l| qrism::Color::try_from(argmax(l) as u8).unwrap()).collect())
        .collect();

    let layers = decode_layers(&pred, ver);
    let actual: [u8; 3] = std::array::from_fn(|k| match &layers[k] {
        Layer::Decoded(m) if *m == messages[k] => ACT_OK,
        Layer::Decoded(_) => ACT_WRONG,
        Layer::NoSymbol => ACT_NOSYM,
        Layer::BadFormat => ACT_BADFMT,
        Layer::BadPayload => ACT_BADPLD,
    });

    let mut buf = Vec::with_capacity(16 + region.len() * 2);
    buf.push(version as u8);
    buf.extend(ecl.map(|e| e as u8));
    buf.extend(actual);
    buf.extend((region.len() as u32).to_le_bytes());
    for &(x, y) in region {
        let l = &ll[y as usize][x as usize];
        let w = argmax(l);
        let t = truth[y as usize][x as usize] as u8;
        let q: [u8; 8] =
            std::array::from_fn(|c| ((l[w] - l[c]) * QUANT).round().clamp(0.0, 255.0) as u8);
        let rest = (0..8).filter(|&c| c != w).map(|c| q[c]);
        let head = t | (w as u8) << 3;
        if rest.clone().all(|v| v == 255) {
            buf.push(head | COMPACT);
        } else {
            buf.push(head);
            buf.extend(rest);
        }
    }
    buf
}

/// Streams records back, `BATCH` at a time.
fn read_records(mut f: impl FnMut(Vec<Record>)) {
    let file = File::open(RECORD_PATH)
        .unwrap_or_else(|e| panic!("{RECORD_PATH}: {e} — run the llr-record pass first"));
    let mut r = BufReader::with_capacity(1 << 20, file);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).unwrap();
    assert_eq!(&magic, MAGIC, "{RECORD_PATH}: not an LLR recording, or an old format");

    let mut batch = Vec::with_capacity(BATCH);
    let mut head = [0u8; 11];
    while r.read_exact(&mut head).is_ok() {
        let ver = head[0] as usize;
        let ecl = [ECLevel::from(head[1]), ECLevel::from(head[2]), ECLevel::from(head[3])];
        let actual = [head[4], head[5], head[6]];
        let n = u32::from_le_bytes(head[7..11].try_into().unwrap()) as usize;

        let (mut truth, mut win, mut lp) =
            (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        let mut b = [0u8; 1];
        let mut rest = [0u8; 7];
        for _ in 0..n {
            r.read_exact(&mut b).unwrap();
            let (t, w) = (b[0] & 7, (b[0] >> 3) & 7);
            let mut l = [-(255.0 / QUANT) as f32; 8];
            l[w as usize] = 0.0;
            if b[0] & COMPACT == 0 {
                r.read_exact(&mut rest).unwrap();
                for (c, &q) in (0..8).filter(|&c| c != w as usize).zip(&rest) {
                    l[c] = -(q as f64 / QUANT) as f32;
                }
            }
            truth.push(t);
            win.push(w);
            lp.push(l);
        }
        batch.push(Record { ver, ecl, actual, truth, win, lp });
        if batch.len() == BATCH {
            f(std::mem::take(&mut batch));
        }
    }
    if !batch.is_empty() {
        f(batch);
    }
}

// Channel view
//------------------------------------------------------------------------------

/// One layer's channel, per stream bit in placement order.
pub(crate) struct Frame {
    /// Truth-relative LLR: positive when the observation favoured the printed bit.
    y: Vec<f64>,
    /// Whether the classifier's winning colour got this bit wrong.
    hard_err: Vec<bool>,
    /// LLR in favour of the hard-decided bit — what a decoder actually sees.
    a: Vec<f64>,
}

#[inline]
fn bit(c: u8, k: usize) -> u8 {
    (c >> (2 - k)) & 1
}

fn lse(xs: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = xs.collect();
    let m = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    m + v.iter().map(|&x| (x - m).exp()).sum::<f64>().ln()
}

/// Splits a record into its three layer channels under posterior scale `s`.
fn frames(rec: &Record, s: f64) -> [Frame; 3] {
    std::array::from_fn(|k| {
        let n = rec.truth.len();
        let (mut y, mut hard_err, mut a) =
            (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
        for i in 0..n {
            let tb = bit(rec.truth[i], k);
            let l = &rec.lp[i];
            let on = lse((0..8u8).filter(|&c| bit(c, k) == tb).map(|c| s * l[c as usize] as f64));
            let off = lse((0..8u8).filter(|&c| bit(c, k) != tb).map(|c| s * l[c as usize] as f64));
            let yi = on - off;
            let e = bit(rec.win[i], k) != tb;
            y.push(yi);
            hard_err.push(e);
            a.push(if e { -yi } else { yi });
        }
        Frame { y, hard_err, a }
    })
}

/// `-ln p(truth)` under scale `s`, the calibration objective.
fn symbol_nll(l: &[f32; 8], t: u8, s: f64) -> f64 {
    lse(l.iter().map(|&x| s * x as f64)) - s * l[t as usize] as f64
}

// Replay
//------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum BlockOutcome {
    Ok,
    /// The decoder gave up — a failure the reader can see.
    Failed,
    /// The decoder returned wrong data — a failure nobody sees.
    Wrong,
}

fn combine(bs: &[BlockOutcome]) -> BlockOutcome {
    if bs.contains(&BlockOutcome::Failed) {
        BlockOutcome::Failed
    } else if bs.contains(&BlockOutcome::Wrong) {
        BlockOutcome::Wrong
    } else {
        BlockOutcome::Ok
    }
}

enum Decoder {
    Rs(RsVariant),
    Ldpc,
}

const DECODERS: [(&str, Decoder); 6] = [
    ("rs hard", Decoder::Rs(RsVariant::Hard)),
    ("rs gmd, min-bit (reader)", Decoder::Rs(RsVariant::Gmd(Ranking::MinBit))),
    ("rs gmd, byte prob", Decoder::Rs(RsVariant::Gmd(Ranking::Prob))),
    ("rs gmd prob + chase", Decoder::Rs(RsVariant::Chase)),
    ("rs erasure genie (bound)", Decoder::Rs(RsVariant::Genie)),
    ("binary ldpc, bp", Decoder::Ldpc),
];
const ND: usize = DECODERS.len();

/// Failing-block anatomy bins: byte errors as a multiple of what an error-only decode fixes.
const OVER_BINS: [(f64, &str); 5] =
    [(1.25, "<=1.25x"), (1.5, "<=1.5x"), (2.0, "<=2x (erasure reach)"), (3.0, "<=3x"), (f64::INFINITY, ">3x")];

#[derive(Clone, Copy, Default)]
struct Tally {
    ok: usize,
    failed: usize,
    wrong: usize,
}

impl Tally {
    fn add(&mut self, o: BlockOutcome) {
        match o {
            BlockOutcome::Ok => self.ok += 1,
            BlockOutcome::Failed => self.failed += 1,
            BlockOutcome::Wrong => self.wrong += 1,
        }
    }
    fn merge(&mut self, o: &Tally) {
        self.ok += o.ok;
        self.failed += o.failed;
        self.wrong += o.wrong;
    }
    fn total(&self) -> usize {
        self.ok + self.failed + self.wrong
    }
    fn rate(&self) -> f64 {
        100.0 * self.ok as f64 / self.total().max(1) as f64
    }
}

/// Everything the report needs, mergeable across records.
#[derive(Clone)]
struct Stats {
    // Calibration, over every layer bit.
    bins: [(usize, usize, f64); LLR_BINS.len()],
    hard_errs: usize,
    confident: [usize; 2], // hard bit errors the decoder believed at p < 10% / p < 1%
    // Information content.
    bit_mi: [f64; 3],
    bits: usize,
    sym_nll_bits: f64,
    modules: usize,
    layers_mi_ok: usize,
    symbols_mi_ok: usize,
    // Decoding.
    layers: [BTreeMap<usize, Tally>; ND],
    symbols: [Tally; ND],
    over: [usize; OVER_BINS.len()],
    // Rs-hard replay against the recorded reader outcome: both ok, both failed, reader only,
    // replay only, and layers the reader lost before reaching Reed-Solomon.
    agree: [usize; 4],
    pre_rs: usize,
}

impl Stats {
    fn new() -> Self {
        Stats {
            bins: [(0, 0, 0.0); LLR_BINS.len()],
            hard_errs: 0,
            confident: [0; 2],
            bit_mi: [0.0; 3],
            bits: 0,
            sym_nll_bits: 0.0,
            modules: 0,
            layers_mi_ok: 0,
            symbols_mi_ok: 0,
            layers: std::array::from_fn(|_| BTreeMap::new()),
            symbols: [Tally::default(); ND],
            over: [0; OVER_BINS.len()],
            agree: [0; 4],
            pre_rs: 0,
        }
    }

    fn merge(mut self, o: Stats) -> Stats {
        for (a, b) in self.bins.iter_mut().zip(&o.bins) {
            a.0 += b.0;
            a.1 += b.1;
            a.2 += b.2;
        }
        self.hard_errs += o.hard_errs;
        for i in 0..2 {
            self.confident[i] += o.confident[i];
        }
        for k in 0..3 {
            self.bit_mi[k] += o.bit_mi[k];
        }
        self.bits += o.bits;
        self.sym_nll_bits += o.sym_nll_bits;
        self.modules += o.modules;
        self.layers_mi_ok += o.layers_mi_ok;
        self.symbols_mi_ok += o.symbols_mi_ok;
        for d in 0..ND {
            for (v, t) in &o.layers[d] {
                self.layers[d].entry(*v).or_default().merge(t);
            }
            self.symbols[d].merge(&o.symbols[d]);
        }
        for i in 0..OVER_BINS.len() {
            self.over[i] += o.over[i];
        }
        for i in 0..4 {
            self.agree[i] += o.agree[i];
        }
        self.pre_rs += o.pre_rs;
        self
    }
}

type Codes = BTreeMap<(usize, u8), (RsLayout, Ldpc)>;

fn replay_one(idx: usize, rec: &Record, s: f64, codes: &Codes) -> Stats {
    let mut st = Stats::new();
    let fr = frames(rec, s);
    let n = rec.truth.len();

    // Symbol-level information: what a code over whole modules (GF(8)) could draw on.
    let mut nll = 0.0;
    for i in 0..n {
        nll += symbol_nll(&rec.lp[i], rec.truth[i], s);
    }
    let sym_bits = nll / std::f64::consts::LN_2;
    st.sym_nll_bits = sym_bits;
    st.modules = n;

    let mut rate_sum = 0.0;
    let mut sym_ok = [true; ND];
    for (k, f) in fr.iter().enumerate() {
        let (layout, ldpc) = &codes[&(rec.ver, rec.ecl[k] as u8)];
        let rate = layout.data_bits as f64 / n as f64;
        rate_sum += rate;

        // Calibration and confident-error counts.
        let mut mi = 0.0;
        for i in 0..n {
            let y = f.y[i];
            let mag = y.abs();
            let b = LLR_BINS.iter().rposition(|&lo| mag >= lo).unwrap();
            st.bins[b].0 += 1;
            st.bins[b].1 += (y < 0.0) as usize;
            st.bins[b].2 += 1.0 / (1.0 + mag.exp());
            if f.hard_err[i] {
                st.hard_errs += 1;
                st.confident[0] += (f.a[i] > 2.2) as usize;
                st.confident[1] += (f.a[i] > 4.6) as usize;
            }
            mi += 1.0 - (-y).exp().ln_1p() / std::f64::consts::LN_2;
        }
        st.bit_mi[k] += mi;
        st.layers_mi_ok += (mi / n as f64 > rate) as usize;

        for (d, (_, dec)) in DECODERS.iter().enumerate() {
            let mut rng = StdRng::seed_from_u64((idx as u64) << 2 | k as u64);
            let out = match dec {
                Decoder::Rs(v) => {
                    let (blocks, anatomy) = rs::replay(f, layout, *v, &mut rng);
                    let out = combine(&blocks);
                    if matches!(v, RsVariant::Hard) {
                        if out != BlockOutcome::Ok {
                            for &(e, ec) in &anatomy {
                                let x = e as f64 / (ec / 2) as f64;
                                if x > 1.0 {
                                    let b = OVER_BINS.iter().position(|&(hi, _)| x <= hi).unwrap();
                                    st.over[b] += 1;
                                }
                            }
                        }
                        match rec.actual[k] {
                            ACT_NOSYM | ACT_BADFMT => st.pre_rs += 1,
                            a => {
                                let real = a == ACT_OK;
                                let rep = out == BlockOutcome::Ok;
                                st.agree[match (real, rep) {
                                    (true, true) => 0,
                                    (false, false) => 1,
                                    (true, false) => 2,
                                    (false, true) => 3,
                                }] += 1;
                            }
                        }
                    }
                    out
                }
                Decoder::Ldpc => ldpc.replay(&f.y, &mut rng),
            };
            st.layers[d].entry(rec.ver).or_default().add(out);
            sym_ok[d] &= out == BlockOutcome::Ok;
        }
    }
    st.bits = 3 * n;
    st.symbols_mi_ok += (3.0 - sym_bits / n as f64 > rate_sum) as usize;
    for d in 0..ND {
        st.symbols[d].add(if sym_ok[d] { BlockOutcome::Ok } else { BlockOutcome::Failed });
    }
    st
}

pub fn replay() {
    println!("\n\n########## LLR replay ##########");

    // Pass 1: fit the posterior scale, and find which codes have to be built.
    // Log-spaced grid, with the unscaled posterior appended last so the raw log-loss can be
    // reported against the fitted one.
    let mut scales: Vec<f64> =
        (0..=30).map(|i| 0.05 * (2.0f64 / 0.05).powf(i as f64 / 30.0)).collect();
    scales.push(1.0);
    let mut nll = vec![0.0f64; scales.len()];
    let mut samples = 0usize;
    let mut shapes: BTreeSet<(usize, u8)> = BTreeSet::new();
    let mut n_rec = 0usize;
    read_records(|batch| {
        let part: Vec<(Vec<f64>, usize)> = batch
            .par_iter()
            .map(|r| {
                let mut acc = vec![0.0; scales.len()];
                let mut cnt = 0;
                for i in (0..r.truth.len()).step_by(CALIB_STRIDE) {
                    for (j, &s) in scales.iter().enumerate() {
                        acc[j] += symbol_nll(&r.lp[i], r.truth[i], s);
                    }
                    cnt += 1;
                }
                (acc, cnt)
            })
            .collect();
        for (acc, cnt) in part {
            for j in 0..scales.len() {
                nll[j] += acc[j];
            }
            samples += cnt;
        }
        for r in &batch {
            for e in r.ecl {
                shapes.insert((r.ver, e as u8));
            }
        }
        n_rec += batch.len();
    });
    if n_rec == 0 {
        println!("  recording is empty");
        return;
    }
    let best = (0..scales.len()).min_by(|&a, &b| nll[a].total_cmp(&nll[b])).unwrap();
    let s = scales[best];
    let raw = nll[scales.len() - 1];

    let codes: Codes = shapes
        .iter()
        .map(|&(v, e)| {
            let ver = Version::Normal(v);
            let layout = RsLayout::new(ver, ECLevel::from(e));
            let n = ver.channel_codewords() * 8;
            let ldpc = Ldpc::new(n, layout.data_bits);
            ((v, e), (layout, ldpc))
        })
        .collect();

    // Pass 2: decode.
    let mut total = Stats::new();
    let mut idx = 0usize;
    read_records(|batch| {
        let base = idx;
        let st = batch
            .par_iter()
            .enumerate()
            .map(|(i, r)| replay_one(base + i, r, s, &codes))
            .reduce(Stats::new, Stats::merge);
        total = std::mem::replace(&mut total, Stats::new()).merge(st);
        idx += batch.len();
    });

    report(&total, n_rec, s, raw / samples as f64, nll[best] / samples as f64, &codes);
}

// Report
//------------------------------------------------------------------------------

fn report(st: &Stats, n_rec: usize, s: f64, nll_raw: f64, nll_cal: f64, codes: &Codes) {
    let ln2 = std::f64::consts::LN_2;
    println!("  {n_rec} symbols, {} layers", 3 * n_rec);

    println!("\n  code shapes (version, EC level: data bits / channel bits = rate)");
    for ((v, e), (l, _)) in codes {
        let n = Version::Normal(*v).channel_codewords() * 8;
        println!(
            "    v{v:<3} {:?}: {} / {} = {:.3}   ({} RS blocks)",
            ECLevel::from(*e),
            l.data_bits,
            n,
            l.data_bits as f64 / n as f64,
            l.blocks.len()
        );
    }

    // Consistency first: if this disagrees, nothing below is trustworthy.
    let [both_ok, both_fail, real_only, rep_only] = st.agree;
    println!("\n  replay check — rs hard vs the reader's own decode of the same layers");
    println!(
        "    agree: {both_ok} ok, {both_fail} failed   disagree: {real_only} reader-only ok, \
         {rep_only} replay-only ok   ({} layers lost before RS: no symbol / bad format)",
        st.pre_rs
    );

    println!("\n  calibration — posterior scale s = {s:.3} (fitted by log-loss on 1/{CALIB_STRIDE} of modules)");
    println!(
        "    symbol log-loss: raw {:.4} bits/module, calibrated {:.4}",
        nll_raw / ln2,
        nll_cal / ln2
    );
    println!("    {:>14}{:>12}{:>14}{:>14}", "|LLR| nats", "share%", "observed err%", "predicted%");
    for (i, &(cnt, err, pred)) in st.bins.iter().enumerate() {
        let hi = LLR_BINS.get(i + 1).map(|h| format!("{h}")).unwrap_or("inf".into());
        println!(
            "    {:>14}{:>12.3}{:>14.4}{:>14.4}",
            format!("{}-{}", LLR_BINS[i], hi),
            100.0 * cnt as f64 / st.bits.max(1) as f64,
            100.0 * err as f64 / cnt.max(1) as f64,
            100.0 * pred / cnt.max(1) as f64
        );
    }
    let he = st.hard_errs.max(1) as f64;
    println!(
        "    hard bit errors: {} ({:.3}% of bits); decoder saw p(wrong) < 10% on {:.1}%, < 1% on {:.1}%",
        st.hard_errs,
        100.0 * st.hard_errs as f64 / st.bits.max(1) as f64,
        100.0 * st.confident[0] as f64 / he,
        100.0 * st.confident[1] as f64 / he
    );
    println!("    (confident errors are out of reach of any soft decoder; the fix is upstream)");

    let per = st.bits as f64 / 3.0;
    println!("\n  information — what the channel could carry, before any code");
    println!(
        "    bit MI R/G/B {:.4} / {:.4} / {:.4} bits per bit",
        st.bit_mi[0] / per,
        st.bit_mi[1] / per,
        st.bit_mi[2] / per
    );
    let sym_mi = 3.0 - st.sym_nll_bits / st.modules.max(1) as f64;
    let bicm = (st.bit_mi.iter().sum::<f64>()) / per;
    println!(
        "    per module: joint {sym_mi:.4} bits vs sum of per-bit {bicm:.4} bits (gap = what \
         decoding bits separately throws away)"
    );
    println!(
        "    layers whose bit MI exceeds their rate: {:.2}%   symbols whose joint MI exceeds \
         their summed rate: {:.2}%",
        100.0 * st.layers_mi_ok as f64 / (3 * n_rec) as f64,
        100.0 * st.symbols_mi_ok as f64 / n_rec as f64
    );
    println!("    (an ideal long code of that rate succeeds roughly where MI > rate; a ceiling, not a promise)");

    let failing: usize = st.over.iter().sum();
    println!("\n  rs hard failure anatomy — {failing} over-capacity blocks, errors vs ec/2");
    for (i, &(_, name)) in OVER_BINS.iter().enumerate() {
        println!(
            "    {:<24}{:>8}{:>8.1}%",
            name,
            st.over[i],
            100.0 * st.over[i] as f64 / failing.max(1) as f64
        );
    }

    let versions: BTreeSet<usize> = st.layers[0].keys().copied().collect();
    println!("\n  decoders at each layer's own rate (layer% = layers delivered)");
    print!("    {:<28}{:>9}{:>9}{:>9}{:>9}", "decoder", "layer%", "code%", "gaveup", "wrong");
    for v in &versions {
        print!("{:>9}", format!("v{v}"));
    }
    println!();
    for (d, (name, _)) in DECODERS.iter().enumerate() {
        let mut all = Tally::default();
        for t in st.layers[d].values() {
            all.merge(t);
        }
        print!(
            "    {:<28}{:>9.2}{:>9.2}{:>9}{:>9}",
            name,
            all.rate(),
            st.symbols[d].rate(),
            all.failed,
            all.wrong
        );
        for v in &versions {
            print!("{:>9.2}", st.layers[d][v].rate());
        }
        println!();
    }
    println!(
        "    (`wrong` = decoded to the wrong data without noticing; the genie row is a bound on \
         erasure flagging, not a decoder)"
    );
}
