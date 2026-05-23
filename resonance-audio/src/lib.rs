// Per ARCHITECTURE.md the only public surface is `AudioEngine` +
// `AudioCommand` / `AudioEvent` (and the value types those carry).
// Modules that were previously `pub` are now `pub(crate)`. The handful
// of items the app legitimately needs are re-exported below:
// - `MidiDeviceInfo`            (replaces `pub use midi_hardware::*`)
// - `DEFAULT_HISTORY_CAPACITY`  (replaces `pub use limits::*`)
// - `linear_resample` / `StreamingLinearResampler`  (decode tools used
//   by the app's vocal-SVS post-processing path)
// - `midi_io` stays public — it's a small, stable utility surface for
//   reading/writing .mid files used by project save/load.
pub(crate) mod clap_host;
pub(crate) mod decode;
mod engine;
mod input_handle;
#[cfg(target_os = "linux")]
mod input_pipewire;
mod limits;
pub(crate) mod midi_clock;
mod midi_hardware;
pub mod midi_io;
mod mixer;
mod platform;
mod recording;
pub(crate) mod stream_errors;
pub mod types;

pub use decode::{linear_resample, StreamingLinearResampler};
pub use engine::{transcode_to_wav, AudioEngine, EngineSendError};
pub use limits::DEFAULT_HISTORY_CAPACITY;
pub use midi_hardware::MidiDeviceInfo;
pub use types::*;

/// Test surfaces for engine internals. Re-exported under a
/// `__test_support` module so integration tests can probe internals
/// without forcing the parent module public.
#[doc(hidden)]
pub mod __test_support {
    pub use crate::clap_host::{ClapBundle, SyncClapInstance};
    pub use crate::engine::try_lock_with_backoff;
    pub use crate::engine::__reset_engine_disconnect_latch_for_test;
    pub use crate::midi_clock::{parse_clock_message, ClockTempoTracker, MidiClockEvent};
    pub use crate::midi_hardware::{parse_live_event_for_test, LiveMidiEvent};
    pub use crate::stream_errors::{
        format_underrun_line, UnderrunRateLimiter, UnderrunReport, UNDERRUN_REPORT_INTERVAL,
    };
}

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
