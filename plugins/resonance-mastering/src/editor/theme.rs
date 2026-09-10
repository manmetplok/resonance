//! Shared palette — re-exports the canonical lavender palette from
//! `wayland_plugin_gui::theme` so all Resonance plugins feel like one
//! product (ba todo #1338).
//!
//! The local `apply()` is gone with it. It set a *subset* of the visuals
//! the shared one does — it left `hovered`/`active`/`open` fills and the
//! inactive border at egui's stock dark values — which is precisely the
//! kind of per-editor drift this migration exists to end; the shared
//! `apply()` from the glob above is what the other ten editors install.
//! The visible difference is that a hovered or open combo box now fills
//! `BG_3` like everywhere else instead of egui grey.
//!
//! Its local green `GOOD` is gone the same way: the shared palette
//! carries one.

pub use plugin_gui_core::theme::lavender::*;
