//! Custom egui widgets used across the drums editor: rotary knob,
//! horizontal slider with fill/thumb, segmented tab strip, chip button.
//!
//! These are deliberately lightweight — they paint a few shapes into the
//! current ui's painter and forward drag interactions back to the param.
//!
//! Duplicated from `resonance-wavetable` so the two editors can evolve
//! independently. May factor into a shared crate later.

pub mod chip;
pub mod knob;
pub mod segmented;
pub mod slider;

pub use chip::chip_button;
pub use knob::{knob_bipolar, knob_unipolar};
pub use segmented::segmented;
pub use slider::{slider_bipolar, slider_bipolar_warm, slider_unipolar};
