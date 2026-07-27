//! Mixer **Reference & A/B** right-rail (design doc #184/#198). A 360px
//! panel that auditions external mastered tracks against the mix.
//! Split into focused submodules; the public entry point is [`container::view`].

mod ab_controls;
mod bodies;
mod container;
mod loudness_canvas;
mod waveform;
mod widgets;

pub(super) use container::view;
