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

use resonance_plugin::Smoother;

/// Point a [`Smoother`] at `target` only when the target actually
/// changed. Calling `set_target` unconditionally every block would
/// restart a linear ramp from the current value each time, turning the
/// fixed-length ramp into an asymptotic crawl — and would keep a
/// converged smoother from ever snapping exactly onto its target.
/// `last` is the caller's record of the previously requested target.
pub(crate) fn retarget(smoother: &mut Smoother, last: &mut f32, target: f32) {
    if *last != target {
        smoother.set_target(target);
        *last = target;
    }
}
