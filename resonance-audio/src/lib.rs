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
pub(crate) mod bypass;
pub(crate) mod clap_host;
pub(crate) mod cycle_load;
pub(crate) mod decode;
mod engine;
mod input_handle;
pub(crate) mod io;
pub(crate) mod latency;
#[cfg(target_os = "linux")]
mod input_pipewire;
#[cfg(target_os = "linux")]
mod output_pipewire;
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
pub(crate) mod supervise;
pub mod types;

pub use decode::{linear_resample, StreamingLinearResampler};
pub use engine::{transcode_to_wav, AudioEngine, EngineSendError};
/// Decode a freeze-cache WAV back into a [`FrozenSource`] for project-load
/// rehydration (ba todo #577). Lives in the audio crate alongside the
/// writer ([`engine::to_freeze_cache`]) so the cache format stays owned in
/// one place; the app calls it to build the `SetTrackFrozenSource` payload.
pub use engine::read_freeze_cache;
// `AudioEvent::AssetImported` carries an `AudioFormat`; re-export it so
// app consumers of the event surface don't need a direct dependency on
// `resonance_common` just to match on it.
pub use resonance_common::AudioFormat;
/// The unit behind a plugin's formatted parameter value — `"dB"` out of
/// `"-6.0 dB"` (ba todo #1290). CLAP has no unit field, so the only
/// place one exists is the plugin's own `value_to_text` output; the app
/// re-derives it when the engine echoes fresh text for a value it just
/// wrote (`AudioEvent::PluginParamText`).
pub use clap_host::unit_from_text;
pub use limits::DEFAULT_HISTORY_CAPACITY;
pub use midi_hardware::MidiDeviceInfo;
pub use types::*;

/// Test surfaces for engine internals. Re-exported under a
/// `__test_support` module so integration tests can probe internals
/// without forcing the parent module public.
#[doc(hidden)]
pub mod __test_support {
    pub use crate::clap_host::{ClapBundle, ClapInstance, PluginMap, PluginSlot, SyncClapInstance};
    /// The live, additive plugin rescan (ba todo #1307) — exposed so
    /// `tests/plugin_rescan.rs` can assert it never loads a bundle it
    /// already holds, which is what keeps running instances safe. The
    /// `_in` variant takes the directories to scan, so the test drives a
    /// temp dir rather than the machine's real plugin folders.
    pub use crate::engine::scan::{rescan_plugins, rescan_plugins_in};
    /// Build a `ClapInstance` around a hand-rolled raw `clap_plugin` —
    /// see `tests/clap_latency_tracking.rs` (doc #260 finding #10).
    pub use crate::clap_host::__instance_from_raw_for_test;
    /// The stepped-parameter choice-label walk behind `ParamInfo.choices`
    /// (ba todo #1290) — pure over a formatter closure, so
    /// `tests/clap_param_meta.rs` can drive it without a plugin.
    pub use crate::clap_host::{choice_labels, MAX_CHOICE_STEPS};
    /// The `.clap` path → dlopen-target resolution (macOS bundle dirs
    /// descend to `Contents/MacOS/<name>`) — pure over the filesystem,
    /// so `tests/clap_bundle_path.rs` can drive it with temp dirs.
    pub use crate::clap_host::bundle_binary_path;
    pub use crate::engine::{
        chunk_span, encode_buffer_for_test, freeze_terminal_event, midi_render_range,
        normalize_buffer_for_test, to_audio_clip, to_freeze_cache, to_freeze_cache_spawn, to_wav,
        try_lock_with_backoff, AutomationSnapshot, ResolvedParamLane, BOUNCE_CHUNK,
        FREEZE_CANCELLED_MSG, MIN_CLAP_FRAMES, SharedState,
    };
    pub use crate::engine::{
        export_stems, measure_mix, measure_rendered_buffer, render_stem, stem_filter,
        stem_project_range, write_stem_wav, StemFilter, MEASURE_BUSY_MSG,
    };
    /// The one "offline render in progress" gate (code review MIX-02 /
    /// ENG-05): every offline renderer holds one of these, the audio
    /// callback outputs silence instead of touching a plugin while any is
    /// held, and Play / Record refuse. Exposed so
    /// `tests/offline_render_gate.rs` can hold the real guard over the
    /// mixer and engine harnesses.
    pub use crate::engine::{OfflineRenderGuard, OFFLINE_RENDER_BUSY_MSG};
    pub use crate::types::{MeasureSource, MixMeasurement, StemBitDepth, StemSource, StemTarget};
    pub use crate::engine::affects_latency;
    /// The take-id allocator behind every captured cycle-record pass
    /// (epic #15, ba doc #292). Pure over a `TakeGroup`, so
    /// `tests/loop_record_takes.rs` can pin "ids are unique within their
    /// group" against the real code — including the case where one pass
    /// emits twice for the same track.
    pub use crate::engine::takes::push_take;
    /// The project-load rehydration of the engine's take-group store, and
    /// the take-group id high-water bump that goes with it (epic #15, ba
    /// todo #1394). Pure over the map + counter, so
    /// `tests/loop_record_takes.rs` can pin "a restored group is never
    /// re-issued to a later cycle-record run" against the real code.
    pub use crate::engine::takes::restore_take_groups_in_place;
    /// Headless harness over the engine control thread's real
    /// `HandlerCtx` + `HandlerState` (ba todo #1399), so a test can run a
    /// whole command handler — not an extracted pure half of one — with
    /// no audio device and no engine thread. Used by
    /// `tests/loop_record_takes.rs` to pin `ClearAll`'s take-lane reset.
    pub use crate::engine::EngineHandlerHarness;
    /// The "one lane per slot" lookup a cycle-record run resolves its take
    /// group through, and the "same slot" predicate behind it (epic #15,
    /// ba todo #1392). Pure over the store, so
    /// `tests/loop_record_takes.rs` can pin the reuse ruling — including
    /// reuse of a group restored from a saved project — against the real
    /// code.
    pub use crate::engine::takes::{
        capture_take_event, resolve_take_group, slots_match, store_take_in, take_group_for_slot,
        TakeGroupStore, SAME_SLOT_TOLERANCE_FRAMES,
    };
    /// The "crossfade or land immediately" rule every bypass handler
    /// shares — see `tests/plugin_bypass.rs`.
    pub use crate::engine::plugins::apply_bypass_request;
    /// The engine's plugin instance-id allocation rule, shared by the
    /// track / bus / master add paths — see `tests/plugin_id_ranges.rs`.
    pub use crate::engine::plugins::allocate_plugin_instance_id;
    /// The add-failure report every chain's add path goes through, and
    /// the two lookups that can produce a reason for it — see
    /// `tests/plugin_load_failure.rs` (ba doc #275 P5, todo #1309).
    pub use crate::engine::plugins::{ensure_bundle, plugin_load_failed_event};
    /// State reload + the error it reports on failure (code review
    /// ENG-02) — see `tests/clap_latency_tracking.rs`.
    pub use crate::engine::plugins::reload_plugin_state;
    /// The event pair the engine emits when a plugin editor refuses to
    /// open (ba todo #1347) — see `tests/plugin_editor_state.rs`.
    pub use crate::engine::plugins::plugin_editor_failure_events;
    /// The RIFF/WAVE chunk walk behind `ClipSource::open_wav` — a pure
    /// function over bytes, so `tests/wav_chunk_parse.rs` can drive every
    /// malformed-header case without touching the filesystem.
    pub use crate::io::wav::{locate_wav_float_data, WavDataChunk};
    pub use crate::latency::{
        add_external_offsets, bus_chain_latencies, chain_latencies, comp_latency_clamped,
        compensation_delays, master_chain_latency, slot_latency, LatencyComp,
    };
    /// The click-free bypass crossfade (ba doc #275 finding X3): the fade
    /// state machine every chain / slot bypass runs through, and the
    /// crossfade the render paths apply. Exposed so
    /// `tests/plugin_bypass.rs` can drive the exact production code path
    /// with a synthetic "plugin" closure — no CLAP instance needed.
    pub use crate::bypass::{
        can_fade, crossfade_to_dry, fade_frames, fade_weight, run_faded, save_dry, BypassFade,
        FadeStage, FxDryScratch, BYPASS_FADE_MS,
    };
    pub use crate::engine::vocal_render::{ensure_tuning_caches, pitch_ratio_curve, retune_clip};
    pub use crate::limits::MAX_COMP_LATENCY;
    pub use crate::platform::{pw_delay_to_engine_samples, MonitorResampler};
    pub use crate::recording::apply_take_shift;
    pub use crate::engine::__reset_engine_disconnect_latch_for_test;
    pub use crate::midi_clock::{parse_clock_message, ClockTempoTracker, MidiClockEvent};
    pub use crate::midi_hardware::{
        encode_control_change, encode_nrpn, parse_control_event_for_test,
        parse_live_event_for_test, LiveControlEvent, LiveMidiEvent,
    };
    pub use crate::mixer::{
        auto_gain_ramp, auto_master_volume, auto_muted, commit_playhead, mix_audition_overlay,
        mix_track_clips,
        live_instrument_for, monitor_catchup_skip, monitor_read_len, ramped_gain,
        recorded_monitor_gate, MixAudioHarness, MonitorDrain, CLIP_DECLICK_FRAMES,
        MONITOR_DRAIN_STREAK,
        push_recording_frames, render_aux_for_test, render_aux_with_comp_for_test,
        RenderBenchHarness, sum_to_output,
        sum_to_stereo, transport_pos_beats,
        whole_frame_push_len,
    };
    /// Take-comp playback (epic #15, doc #165): the control-thread flatten
    /// of the authoritative take groups into the audio-thread table, the
    /// per-segment render with its equal-power seam crossfades, and the
    /// block entry point that drives both through the real `render_block`
    /// — see `tests/take_comp_render.rs`.
    pub use crate::mixer::{
        build_comp_table, mix_track_comp, render_take_comp_for_test, CompRenderTable, CompSpan,
        TrackComp, COMP_XFADE_FRAMES,
    };
    pub use crate::platform::{
        choose_assert_rate, force_is_redundant, force_release_target, needs_reassert,
        parse_allowed_rates, parse_pw_metadata_value, reassert_source_key, CANONICAL_RATE,
    };
    pub use crate::stream_errors::{
        format_underrun_line, UnderrunRateLimiter, UnderrunReport, UNDERRUN_REPORT_INTERVAL,
    };
    /// The panic containment every detached worker (offline render
    /// spawn sites, `ImportQueue` jobs) runs its body through — exposed
    /// so tests can pin "a panicking worker still emits its terminal
    /// error event" without needing a way to make the real render core
    /// panic.
    pub use crate::supervise::{panic_message, run_supervised};
    pub use crate::cycle_load::{
        format_cycle_load_line, CycleLoadMeter, CycleLoadReport, LOAD_EMA_ALPHA,
        QUIET_PEAK_THRESHOLD, QUIET_REPORT_INTERVAL, VERBOSE_REPORT_INTERVAL,
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

/// Test surface for the timeline → hardware note emission core and its
/// Recorded-playback span gating (doc #257, todo #1099). Exposed so
/// `tests/recorded_outbound_gating.rs` can drive the pure emitter with a
/// capturing fake [`OutboundNoteSink`] — covered-span NoteOn suppression,
/// held-note release at span entry, live fallback in gaps — without
/// opening a hardware port or spinning up the engine thread.
#[doc(hidden)]
pub use engine::midi::{
    emit_outbound_notes, outbound_track_snapshot, OutboundNoteSink, OutboundTrack,
};

/// Test surface for the `SetTrackPlaybackSource` command boundary
/// (doc #257, todo #1099): engine-side track-field update +
/// `TrackPlaybackSourceChanged` echo (and the missing-track no-op
/// branch), testable without spinning up the engine thread.
#[doc(hidden)]
pub use engine::set_track_playback_source_in_place;

/// Test surface for the device-parameter automation → CC/NRPN emission
/// core (doc #201 §4, todo #723). Exposed so the integration test in
/// `tests/device_param_automation.rs` can drive the pure emitter with a
/// capturing fake [`DeviceParamMidiSink`] — asserting the ordered
/// CC/NRPN sequence and live↔bounce parity — without opening a port.
#[doc(hidden)]
pub use engine::midi::{emit_device_param_automation, DeviceParamMidiSink};

/// Test surface for the MIDI clip move/trim handlers. Exposed so the
/// regression test in `tests/midi_clip_handlers.rs` can drive the
/// missing-clip no-op branch without spinning up the engine thread.
#[doc(hidden)]
pub use engine::midi::{move_midi_clip_in_place, trim_midi_clip_in_place};

/// Test surface for the `SetTrackDeviceParams` command boundary (epic #40,
/// doc #201 §4). Exposed so the integration test in
/// `tests/device_params_handler.rs` can drive the engine-side map update +
/// `TrackDeviceParamsApplied` emission (and the missing-track no-op branch)
/// without spinning up the engine thread.
#[doc(hidden)]
pub use engine::midi::set_track_device_params_in_place;

/// Test surface for the bulk MIDI-edit handlers (quantize / humanize /
/// groove). Exposed so the engine tests in `tests/midi_bulk_edits.rs` can
/// drive each bulk command's mutation + event emission (including the
/// missing-clip no-op branch) without spinning up the engine thread.
#[doc(hidden)]
pub use engine::midi::{
    apply_groove_to_clip_in_place, extract_groove_from_clip_in_place, humanize_midi_notes_in_place,
    quantize_midi_notes_in_place,
};

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

/// Test surface for the deferred-clip-edit queue (ba doc #276 BUG 1):
/// the pure half of "a clip edit that arrives before its clip finished
/// loading is parked and replayed, not dropped".
#[doc(hidden)]
pub use engine::{partition_deferred_clip_commands, DeferredClipCommand};

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

/// Test surface for the automation-lane handlers. Exposed so the
/// integration test in `tests/automation_handlers.rs` can drive the
/// command boundary (store/replace, clear, read-flag toggle, and the
/// missing-target no-op branches) against a plain lane map without
/// spinning up the engine thread.
#[doc(hidden)]
pub use engine::{
    clear_automation_lane_in_place, set_automation_lane_in_place,
    set_automation_read_enabled_in_place, AutomationLanes, LiveValueEmitter,
    AUTOMATED_VALUE_EPSILON, AUTOMATED_VALUE_THROTTLE,
};

/// Test surface for the external-instrument config handlers. Exposed so the
/// integration test in `tests/external_instrument_handlers.rs` can drive the
/// command boundary (store/replace, clear, latency/patch updates, the
/// device-offline reporting, and the not-an-external-instrument no-op
/// branches) against a plain config map without spinning up the engine thread.
#[doc(hidden)]
pub use engine::{
    check_external_instrument_devices_in_place, clear_external_instrument_in_place,
    mark_external_tracks, resend_external_instrument_patch_in_place,
    set_external_instrument_in_place,
    set_external_instrument_latency_in_place, set_external_instrument_patch_in_place,
    ExternalInstruments,
};

/// Test surface for the external-instrument round-trip latency ("ping")
/// detector. Exposed so the integration test in
/// `tests/external_instrument_ping.rs` can drive the pure onset-detection and
/// sample-rate conversion math — the heart of the auto-detect — without
/// opening a real audio device or MIDI port.
#[doc(hidden)]
pub use engine::{
    detect_impulse_onset, estimate_noise_floor, onset_to_engine_samples, onset_to_ms,
    ping_deadline_reached, OnsetOutcome,
};
/// Exposed for `tests/external_instrument_handlers.rs` so it can construct an
/// empty output registry and exercise the patch-send offline branch without
/// opening a real MIDI port.
#[doc(hidden)]
pub use midi_hardware::MidiOutputRegistry;

/// Test surface for the audio import-to-pool path. Exposed so the
/// integration test in `tests/import_audio_to_pool.rs` can drive the
/// pure per-file import (`import_one_to_pool`) and the full ordered
/// event lifecycle (`run_pool_import`) without bringing up the engine
/// thread or a real audio device.
#[doc(hidden)]
pub use engine::{import_one_to_pool, run_pool_import, PoolImportOutcome};

/// Test surface for the bounded clip import / project-load worker pool.
/// Exposed so `tests/load_clip_offthread.rs` can drive the exact queue
/// the engine handlers submit to (nothing dropped past the concurrency
/// cap) without bringing up the engine thread or an audio device.
#[doc(hidden)]
pub use engine::{ImportQueue, MAX_CONCURRENT_IMPORTS};

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
