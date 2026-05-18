//! WaveNet inference engine for NAM models.
//!
//! Submodules:
//! - `conv_layer` — `WaveNetLayer` and the 1x1 conv primitives it composes from.
//! - `head` — dense MLP layer used by the output head.
//! - `ring` — dilated-conv state ring buffer.
//! - `model` — the `WaveNetModel` orchestrator (load + per-sample inference).
//!
//! Only `WaveNetModel` is re-exported; the layer primitives stay crate-private.

mod conv_layer;
mod head;
mod model;
mod ring;

pub use model::WaveNetModel;
