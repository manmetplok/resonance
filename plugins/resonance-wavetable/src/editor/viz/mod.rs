//! Canvas-based visualisations for the editor.
//!
//! Each submodule defines a `draw_*` function that takes a `&mut egui::Ui`
//! and an allocated rect, reads whatever state it needs from the
//! `WavetableEditorApp`, and paints using `ui.painter()`.

pub mod envelope;
pub mod filter_response;
pub mod frame_strip;
pub mod lfo_shape;
pub mod scope;
pub mod waveform;
