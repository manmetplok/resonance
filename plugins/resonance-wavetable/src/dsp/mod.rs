//! Pure DSP for the wavetable synth: oscillators, voices, envelopes,
//! filters, LFOs, modulation routing, effects, and the per-block render
//! path. No plugin trait, no UI, no allocations in the process path.
//!
//! `wavetable_gen` is build-time only — `build.rs` `#[path]`-includes it
//! to bake the bundled wavetables into `$OUT_DIR/wavetables.bin`; it is
//! deliberately not declared as a module here.

pub mod effects;
pub mod engine;
pub mod envelope;
pub mod filter;
pub mod lfo;
pub mod modulation;
pub mod oscillator;
pub mod render;
pub mod voice;
pub mod wavetable;
