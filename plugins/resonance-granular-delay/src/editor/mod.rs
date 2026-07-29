//! Granular-delay editor: an egui UI hosted in `wayland-plugin-gui`
//! (ba todo #1079), rendered in the canonical lavender design-system
//! tokens (design doc #264 req-7, ba todo #1136).
//!
//! Layout (three bands, design doc #264): a 46 px header with title +
//! live readouts (effective delay, tempo/division, voice-lock, grain
//! count, freeze), the central hero buffer-view band, and the 246 px
//! control strip with the full §9 parameter set grouped per doc #252
//! §9: Time, Grains, Pitch, Feedback, Space, Output.
//!
//! `controls` is public for the group/label unit tests
//! (tests/editor_groups.rs): the grouping table and label lists are
//! pure static data.

mod app;
pub mod controls;
mod factory;
mod theme;
// Public: the lavender widget kit (ba todo #1138) is the editor's
// building-block API for the signal-flow strip and hero todos.
pub mod widgets;

pub use factory::GranularEditorFactory;
