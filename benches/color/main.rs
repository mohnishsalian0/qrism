//! Colour-decode benchmark for high-capacity (colour) QR symbols.
//!
//! The decode itself is a three-phase pipeline:
//!
//! 1. [`normalization`] — map raw sampled RGB into a working space (or pass through).
//! 2. [`recovery`] — recover the encoded signal: per-channel indicators (per-colorant) or
//!    a direct colour estimate (euclid-measured).
//! 3. [`thresholding`] — turn per-channel indicators into a `Color` (channel recoveries
//!    only; direct recoveries self-threshold).
//!
//! [`hiq`] then decodes the resulting colour grid into the three messages a HiQ symbol
//! carries, so a pipeline is scored on delivered messages as well as on module accuracy.
//!
//! Each phase fits its parameters per image from the same grouped calibration samples (see
//! [`calibration`]), fitting in the space produced by the previous phase. The [`analysis`]
//! module composes concrete combinations and measures them two ways: decode accuracy against
//! reference renders, and per-module cost.
//!
//! This lives in `benches/` rather than the library because nothing here is wired into the
//! decode path yet. It reaches into the library through one benchmark-gated hook,
//! `Symbol::exact_map`, which is why the target needs `--features benchmark`.
//!
//! Run with:
//!   cargo bench --features benchmark --bench color
//!   cargo bench --features benchmark --bench color -- accuracy
//!   cargo bench --features benchmark --bench color -- timing

pub(crate) mod calibration;
pub(crate) mod hiq;
pub(crate) mod normalization;
pub(crate) mod recovery;
pub(crate) mod thresholding;

mod analysis;

fn main() {
    // `cargo bench` passes its own flags (e.g. --bench); treat only bare words as pass names.
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with('-')).collect();
    let run = |name: &str| args.is_empty() || args.iter().any(|a| a == name);

    if run("accuracy") {
        analysis::benchmark_accuracy();
    }
    if run("timing") {
        analysis::benchmark_timing();
    }
}
