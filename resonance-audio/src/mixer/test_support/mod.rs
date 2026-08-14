//! Test and benchmark entry points into the mixer, kept out of the
//! realtime modules themselves (ba todo #1255).
//!
//! Everything here exists so an integration test or a bench can drive the
//! real audio code without an audio device, a CLAP plugin or the engine
//! thread — and so `indexmap`, `ringbuf`, `parking_lot` and the
//! `SharedState` plumbing stay out of the `tests/` crate:
//!
//! - [`render_block`]: one offline block through the render core
//!   ([`render_aux_for_test`], [`render_aux_with_comp_for_test`]) and the
//!   live-strategy bench loop ([`RenderBenchHarness`]).
//! - [`callback`]: the whole audio callback over owned state
//!   ([`MixAudioHarness`]).
//!
//! None of it is part of the crate's public API; it is re-exported under
//! `crate::__test_support`.

mod callback;
mod render_block;

pub use callback::MixAudioHarness;
pub use render_block::{render_aux_for_test, render_aux_with_comp_for_test, RenderBenchHarness};
