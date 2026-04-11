//! Mastering chain stages. Each sub-module owns one DSP block that fits
//! into the mastering signal path. Later phases will add `multiband`,
//! `imager`, `limiter`, and `dither`.

pub mod dither;
pub mod glue_compressor;
pub mod imager;
pub mod limiter;
pub mod linear_phase_eq;
pub mod multiband;
pub mod saturator;
