//! Granular-delay editor: an egui UI hosted in `wayland-plugin-gui`
//! (ba todo #1079), following `plugins/resonance-delay/src/editor`'s
//! layout and the shared classic theme (ux-guidelines.md).
//!
//! Layout: a top header with title + live readouts (effective delay,
//! tempo/division, voice-lock, grain count, freeze), and a central
//! control surface with the full §9 parameter set grouped per doc #252
//! §9 / ba todo #1079: Time, Grains, Pitch, Feedback, Space, Output.
//!
//! `controls` is public for the group/label unit tests
//! (tests/editor_groups.rs): the grouping table and label lists are
//! pure static data.

mod app;
pub mod controls;
mod factory;
mod theme;
mod widgets;

pub use factory::GranularEditorFactory;
