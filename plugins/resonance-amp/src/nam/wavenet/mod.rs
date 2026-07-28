//! WaveNet inference engine for NAM models.
//!
//! Submodules:
//! - `conv_layer` — `WaveNetLayer` and the 1x1 conv primitives it composes from.
//! - `film` — FiLM (feature-wise linear modulation) block.
//! - `head` — dense MLP layer used by the output head.
//! - `ring` — dilated-conv state ring buffer.
//! - `model` — the `WaveNetModel` orchestrator (load + per-sample inference).
//! - `params` — typed full A2 config surface (parse-only for now).
//!
//! Only `WaveNetModel` and the typed `params` config structs are exported;
//! the layer primitives stay crate-private.

mod conv_layer;
mod film;
mod head;
mod model;
pub mod params;
mod ring;

pub use model::WaveNetModel;
