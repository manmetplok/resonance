//! DSP-side of the drum plugin: sampler engine, voice picking, and
//! voice/heap cleanup helpers. Split out of the original monolithic
//! `sampler.rs` so the per-block render path, the pure picking helpers,
//! and the voice/heap janitor each live in a focused file.

pub mod janitor;
pub mod sampler;
pub mod voice_pick;

pub use sampler::{DrumSampler, PortBuffers};
pub use voice_pick::{pick_rr, pick_velocity_layer, MAX_LAYERS};
