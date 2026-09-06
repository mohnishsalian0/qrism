//! Colour decoding for high-capacity (colour) QR symbols, as a three-phase pipeline:
//!
//! 1. [`normalization`] — map raw sampled RGB into a working space (or pass through).
//! 2. [`recovery`] — recover the encoded signal: per-channel indicators (per-colorant) or
//!    a direct colour estimate (euclid-measured).
//! 3. [`thresholding`] — turn per-channel indicators into a `Color` (channel recoveries
//!    only; direct recoveries self-threshold).
//!
//! Each phase fits its parameters per image from the same grouped calibration samples (see
//! [`calibration`]), fitting in the space produced by the previous phase. The [`analysis`]
//! harness composes concrete combinations and benchmarks them. Everything here is
//! test-only for now (the module is declared under `#[cfg(test)]`); promote it when wiring
//! colour decode into the pipeline.

pub(crate) mod calibration;
pub(crate) mod normalization;
pub(crate) mod recovery;
pub(crate) mod thresholding;

mod analysis;
