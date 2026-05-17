//! Compressor editor — egui UI hosted by wayland-plugin-gui.
//!
//! Layout (top-down):
//! - Header: plugin name, preset dropdown.
//! - Middle: transfer curve + GR history + 3 meters (In / GR / Out) in a row.
//! - Bottom: control strip with the 11 parameters.

mod app;
mod control_strip;
mod curve;
mod factory;
mod history;
mod meters;
mod theme;

pub use factory::CompressorEditorFactory;
