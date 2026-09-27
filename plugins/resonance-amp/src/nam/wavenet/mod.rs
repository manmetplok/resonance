//! WaveNet inference engine for NAM models.
//!
//! Submodules:
//! - `conv_layer` — `WaveNetLayer` and the 1x1 conv primitives it composes from.
//! - `film` — FiLM (feature-wise linear modulation) block.
//! - `head` — dense MLP layer used by the output head.
//! - `history` — dilated-conv input history (linear, block-windowed).
//! - `model` — the `WaveNetModel` orchestrator (load + block inference).
//! - `params` — typed full A2 config surface (parse-only for now).
//! - `slimmable` — packed-weight slice extraction for slimmable models.
//!
//! Only `WaveNetModel`, the typed `params` config structs, and the
//! `slimmable` slice helpers are exported; the layer primitives stay
//! crate-private.

mod conv_layer;
mod film;
mod head;
mod model;
pub mod params;
mod history;
pub mod slimmable;

pub use model::WaveNetModel;
