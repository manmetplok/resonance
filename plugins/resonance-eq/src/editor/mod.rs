//! Custom EQ editor: a frequency-response curve with draggable band nodes,
//! a per-band control strip underneath, and a factory-preset dropdown in
//! the header. Runs on an egui UI hosted by `wayland-plugin-gui`.

mod app;
mod control_strip;
mod factory;
mod nodes;
mod response;
mod theme;

pub use factory::EqEditorFactory;

pub(crate) use app::{AnalyzerMode, EqEditorApp};
