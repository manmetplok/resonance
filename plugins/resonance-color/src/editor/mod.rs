//! Color editor — egui UI hosted by the platform plugin-GUI runtime.
//!
//! Layout (top-down):
//! - Header: plugin name and the preset bar.
//! - Middle: the transfer curve (with the live input level on it), the
//!   harmonic bars H1–H7 with the THD readout, and the In / Out meters
//!   with the auto-gain readout.
//! - Bottom: the control strip — one control per parameter, every one
//!   bound to its parameter (`tests/editor_param_binding.rs`).
//!
//! # Cost
//!
//! The two plots are functions of the settings, not of the audio, so
//! both are cached and recomputed only when a setting they depend on
//! moves ([`curve::CurveCache`], [`harmonics::ProbeCache`]) — the
//! plugin-editor form of the view-performance rules (`ui-work.md` §11).
//! Only the meters and the operating-point marker read live values each
//! frame, and those are three atomics.

mod app;
mod controls;
pub mod curve;
mod factory;
pub mod harmonics;
mod theme;

pub use factory::ColorEditorFactory;
