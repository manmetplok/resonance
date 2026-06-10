//! Custom egui widgets used across the wavetable editor: horizontal
//! slider with fill/thumb, segmented tab strip, chip button. The rotary
//! knob is the shared one from `wayland_plugin_gui::widgets`,
//! re-exported here so call sites keep their `widgets::knob_*` paths.
//!
//! These are deliberately lightweight — they paint a few shapes into the
//! current ui's painter and forward drag interactions back to the param.

pub mod chip;
pub mod segmented;
pub mod slider;

pub use chip::chip_button;
pub use segmented::segmented;
pub use slider::slider_bipolar;
pub use wayland_plugin_gui::widgets::{knob_bipolar, knob_unipolar};
