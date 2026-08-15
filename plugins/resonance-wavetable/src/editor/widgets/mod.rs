//! Custom egui widgets used across the wavetable editor: horizontal
//! slider with fill/thumb, segmented tab strip, chip button. The rotary
//! knob is the shared themed one from `wayland_plugin_gui::widgets`,
//! re-exported here so call sites keep their `widgets::` paths.
//!
//! Everything here works in **unit (`0..1`) travel** and knows nothing
//! about parameters — the param bindings live in
//! [`crate::editor::tabs`], which is the only place allowed to turn a
//! parameter into a control (ba todo #1285).

pub mod chip;
pub mod segmented;
pub mod slider;

pub use chip::chip_button;
pub use segmented::segmented;
pub use slider::slider_unit;
pub use wayland_plugin_gui::widgets::{knob_themed, ThemedKnob};
