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
pub(crate) mod latency;
#[cfg(target_os = "linux")]
mod input_pipewire;
mod limits;
pub(crate) mod midi_clock;
mod midi_hardware;
pub mod midi_io;
mod mixer;
mod platform;
pub(crate) mod prefault;
pub mod quantize;
mod recording;
pub(crate) mod stream_errors;
pub mod types;

pub use decode::{linear_resample, StreamingLinearResampler};
pub use engine::{transcode_to_wav, AudioEngine, EngineSendError};
// `AudioEvent::AssetImported` carries an `AudioFormat`; re-export it so
// app consumers of the event surface don't need a direct dependency on
// `resonance_common` just to match on it.
pub use resonance_common::AudioFormat;
pub use limits::DEFAULT_HISTORY_CAPACITY;
pub use midi_hardware::MidiDeviceInfo;
pub use types::*;

/// Test surfaces for engine internals. Re-exported under a
/// `__test_support` module so integration tests can probe internals
/// without forcing the parent module public.
#[doc(hidden)]
pub mod __test_support {
    pub use crate::clap_host::{ClapBundle, SyncClapInstance};
    pub use crate::engine::{
        midi_render_range, to_audio_clip, to_wav, try_lock_with_backoff, SharedState,
    };
    pub use crate::engine::{
        export_stems, render_stem, stem_filter, stem_project_range, write_stem_wav, StemFilter,
    };
    pub use crate::types::{StemBitDepth, StemSource, StemTarget};
    pub use crate::latency::{chain_latencies, compensation_delays, LatencyComp};
    pub use crate::limits::MAX_COMP_LATENCY;
    pub use crate::engine::__reset_engine_disconnect_latch_for_test;
    pub use crate::midi_clock::{parse_clock_message, ClockTempoTracker, MidiClockEvent};
    pub use crate::midi_hardware::{
        parse_control_event_for_test, parse_live_event_for_test, LiveControlEvent, LiveMidiEvent,
    };
    pub use crate::mixer::{
        mix_audition_overlay, mix_track_clips, monitor_catchup_skip, monitor_read_len,
        ramped_gain, render_aux_for_test, sum_to_output, sum_to_stereo, transport_pos_beats,
        whole_frame_push_len,
    };
    pub use crate::stream_errors::{
        format_underrun_line, UnderrunRateLimiter, UnderrunReport, UNDERRUN_REPORT_INTERVAL,
    };
    /// Re-exported so app-side handler tests can name the command receiver
    /// returned by [`AudioEngine::for_test_capture`](crate::AudioEngine::for_test_capture).
    pub use crossbeam_channel::Receiver;
}

/// Test surface for the audition preview handlers (doc #175). Exposed so the
/// integration test in `tests/audition_preview.rs` can drive the
/// command/state boundary — decode + start, stop, options/ratio recompute,
/// and the realtime overlay mix — against a plain `SharedState` without
/// spinning up the engine thread or a real audio device.
#[doc(hidden)]
pub use engine::{
    compute_sync_ratio, load_audition_source, set_audition_options_in_place,
    start_audition_in_place, stop_audition_in_place, AuditionSource,
};

/// Test surface for the hardware-MIDI loop-wrap rewind logic. Exposed
/// so integration tests can verify the discontinuity classification
/// without bringing up the engine thread.
#[doc(hidden)]
pub use engine::midi::{outbound_step_start, OutboundStep};

/// Test surface for the MIDI clip move/trim handlers. Exposed so the
/// regression test in `tests/midi_clip_handlers.rs` can drive the
/// missing-clip no-op branch without spinning up the engine thread.
#[doc(hidden)]
pub use engine::midi::{move_midi_clip_in_place, trim_midi_clip_in_place};

/// Test surface for the audio clip fade/gain/warp handlers. Exposed so
/// the integration tests in `tests/clip_fade_gain_handlers.rs` and
/// `tests/clip_warp_handlers.rs` can drive the command boundary (mutation
/// + event emission, including the missing-clip no-op branch and the
/// marker-sort invariant) without spinning up the engine thread.
#[doc(hidden)]
pub use engine::{
    detect_clip_tempo_in_place, set_clip_fade_in_place, set_clip_gain_in_place,
    set_clip_warp_in_place, set_clip_warp_markers_in_place, MAX_CLIP_GAIN_DB, MIN_CLIP_GAIN_DB,
};

/// Test surface for the reference-track (A/B) command handlers. Exposed
/// so `tests/reference_handlers.rs` can drive each command's mutation +
/// event emission against a bare `ReferencePlayer` without spinning up
/// the engine thread.
#[doc(hidden)]
pub use engine::reference::{
    handle_add_ref_marker, handle_load_reference_track, handle_poll_ab_meters,
    handle_reference_analyzed, handle_remove_ref_marker, handle_remove_reference_track,
    handle_set_ab_source, handle_set_active_reference, handle_set_ref_loop_to_mix,
    handle_set_ref_loudness_match, handle_set_ref_position, handle_set_ref_trim, register_reference,
    run_reference_analysis, ABMeterTap, ABMeters, ReferenceMonitor, ReferencePlayer,
    REFERENCE_OVERVIEW_PEAKS,
};

/// Test surface for the audio import-to-pool path. Exposed so the
/// integration test in `tests/import_audio_to_pool.rs` can drive the
/// pure per-file import (`import_one_to_pool`) and the full ordered
/// event lifecycle (`run_pool_import`) without bringing up the engine
/// thread or a real audio device.
#[doc(hidden)]
pub use engine::{import_one_to_pool, run_pool_import, PoolImportOutcome};

/// Test surface for the vocal pitch-analysis path. Exposed so the
/// integration test in `tests/clip_pitch_analysis.rs` can drive the
/// command boundary (cache store + `ClipPitchDetected` emission, plus the
/// pure DSP mapping) without spinning up the engine thread.
#[doc(hidden)]
pub use engine::{analyze_clip_pitch_in_place, analyze_pitch};

/// Test surface for the bounce path's MIDI event collection. Exposed so
/// integration tests can drive the chunk-by-chunk note-event walk
/// without spinning up a CLAP plugin or the engine thread.
#[doc(hidden)]
pub use mixer::collect_midi_events_bounce;

/// Test surface for the plugin-lock-contention MIDI stash. Exposed so
/// the regression test in `tests/midi_stash.rs` can drive stash /
/// overflow / panic / delivery without a live CLAP plugin (the test
/// supplies its own `NoteSink`).
#[doc(hidden)]
pub use mixer::{MidiStash, NoteSink};
#[doc(hidden)]
pub use limits::{MAX_STASHED_EVENTS, MAX_STASHED_INSTRUMENTS};

/// Test surface for the live-input contention path. Exposed so the
/// regression test in `tests/live_note_retry_order.rs` can verify that
/// a NoteOn parked on a contended plugin lock is always delivered
/// before a later NoteOff for the same key (the test supplies its own
/// `NoteSink` behind a `parking_lot::Mutex`).
#[doc(hidden)]
pub use engine::midi::deliver_or_stash;

/// Test surface for the live-input arrival → intra-block sample offset
/// conversion. Exposed so the test in `tests/live_arrival_offset.rs`
/// can drive the pure function without bringing up the engine thread.
#[doc(hidden)]
pub use engine::midi::live_arrival_sample_offset;

/// Test surface for the streaming recording drain path. Exposed so
/// integration tests can verify that `TrackRecordingBuf` never
/// accumulates audio in RAM as a take grows. Not part of the public
/// API — the engine owns `RecordingState` internally.
#[doc(hidden)]
pub use recording::{PrecountState, RecordingState, RolledAudioTake, TrackRecordingBuf};
