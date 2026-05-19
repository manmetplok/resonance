//! Delay plugin editor: an egui UI hosted in `wayland-plugin-gui`.
//!
//! Layout: a top header with title + preset picker + readouts + freeze
//! indicator, a bottom control strip, and a centre echo-view visualisation.

mod app;
mod controls;
mod echo_view;
mod factory;
mod theme;
mod widgets;

pub use factory::DelayEditorFactory;
