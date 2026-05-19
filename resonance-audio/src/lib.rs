pub mod clap_host;
pub mod decode;
mod engine;
mod input_handle;
#[cfg(target_os = "linux")]
mod input_pipewire;
pub mod limits;
pub mod midi_clock;
pub mod midi_hardware;
pub mod midi_io;
mod mixer;
mod platform;
mod recording;
pub mod types;

pub use engine::{transcode_to_wav, AudioEngine};
pub use types::*;

/// Test surface for the hardware-MIDI loop-wrap rewind logic. Exposed
/// so integration tests can verify the discontinuity classification
/// without bringing up the engine thread.
#[doc(hidden)]
pub use engine::midi::{outbound_step_start, OutboundStep};

/// Test surface for the bounce path's MIDI event collection. Exposed so
/// integration tests can drive the chunk-by-chunk note-event walk
/// without spinning up a CLAP plugin or the engine thread.
#[doc(hidden)]
pub use mixer::collect_midi_events_bounce;

/// Test surface for the streaming recording drain path. Exposed so
/// integration tests can verify that `TrackRecordingBuf` never
/// accumulates audio in RAM as a take grows. Not part of the public
/// API — the engine owns `RecordingState` internally.
#[doc(hidden)]
pub use recording::{PrecountState, RecordingState, TrackRecordingBuf};
