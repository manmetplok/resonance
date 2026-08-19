//! Editor theme — the shared lavender palette from
//! `wayland_plugin_gui::theme` (canonical Resonance tokens, applied once
//! per frame from the top-level `ui()` method).
//!
//! Nothing local is left: the `ACCENT_GLOW` this module hand-mixed and
//! the `WARN` alias the viz modules use are both carried by the shared
//! palette now — ba todo #1338 promoted them when the seven effect
//! editors migrated onto it and needed the same two.

pub use wayland_plugin_gui::theme::lavender::*;
