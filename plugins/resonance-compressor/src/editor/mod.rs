//! Compressor editor — egui UI hosted by wayland-plugin-gui.
//!
//! Layout (top-down):
//! - Header: plugin name, preset dropdown, detector-source status.
//! - Middle: transfer curve + GR history + the meter row (In / GR / Out,
//!   plus Key while a sidechain key is connected).
//! - Bottom: control strip with the 11 parameters.

mod app;
mod control_strip;
// Public so `tests/curve_axis.rs` can hold the plot's dB window to the
// threshold parameter's declared range (ba todo #1346); the drawing
// itself still needs a live egui painter and is not exercised there.
pub mod curve;
mod factory;
mod history;
mod meters;
mod theme;

pub use factory::CompressorEditorFactory;
