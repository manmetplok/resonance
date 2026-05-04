pub mod clap_host;
pub mod decode;
mod engine;
pub mod limits;
pub mod midi_clock;
pub mod midi_hardware;
pub mod midi_io;
mod mixer;
mod platform;
mod recording;
pub mod types;

pub use engine::AudioEngine;
pub use types::*;

/// Test surface for the hardware-MIDI loop-wrap rewind logic. Exposed
/// so integration tests can verify the discontinuity classification
/// without bringing up the engine thread.
#[doc(hidden)]
pub use engine::midi::{outbound_step_start, OutboundStep};
