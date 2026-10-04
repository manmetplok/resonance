//! The core audio engine. [`AudioEngine`] (`audio_engine`) wires up the
//! output stream and spawns the engine control thread; [`SharedState`]
//! (`shared_state`) is what the two threads share; the control thread's
//! command dispatch and per-concern handlers live in the submodules
//! (`thread`, `transport`, `tracks`, `clips`, `midi`, `plugins`, `chain`,
//! `busses`, plus `scan` and `bounce`). This file declares and re-exports
//! (ARCH2-12); it defines nothing itself.

mod audio_engine;
mod recording_ring;
mod shared_state;

pub use audio_engine::{AudioEngine, EngineOptions, EngineSendError};
#[doc(hidden)]
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use audio_engine::__reset_engine_disconnect_latch_for_test;
pub use recording_ring::recording_ring_len;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use recording_ring::RECORDING_RING_SECONDS;
pub(crate) use retire::rcu_tempo;
pub use shared_state::SharedState;

pub(crate) use crate::limits::MAX_BUSSES;

mod bounce;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use bounce::{
    chunk_span, encode_buffer_for_test, export_for_test, export_stems, freeze_terminal_event,
    measure_audio_file, measure_decoded, measure_mix, measure_mix_detailed,
    measure_rendered_buffer_detailed, measure_rendered_buffer, normalize_buffer_for_test, read_freeze_cache, render_stem,
    stem_filter, stem_project_range, to_audio_clip, to_freeze_cache, to_freeze_cache_spawn, to_wav,
    try_lock_with_backoff, write_stem_wav, FreezeError, OfflineRenderGuard,
    BOUNCE_CHUNK, FREEZE_CANCELLED_MSG, MEASURE_BUSY_MSG, MIN_CLAP_FRAMES,
    OFFLINE_RENDER_BUSY_MSG, StemFilter,
};
mod bounce_common;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use bounce_common::midi_render_range;

pub(crate) mod retire;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use retire::Retired;

mod loop_range;
pub use loop_range::LoopRange;

pub mod count_in_arm;

pub(crate) mod internal;

pub(crate) mod render_graph;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use render_graph::{RenderGraph, RenderGraphSlot};

pub(crate) mod audition;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use audition::{
    compute_sync_ratio, load_audition_source, set_audition_options_in_place,
    start_audition_in_place, stop_audition_in_place, AuditionSource,
};
mod automation;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use automation::{
    clear_automation_lane_in_place, set_automation_lane_in_place,
    set_automation_read_enabled_in_place, AutomationLanes, AutomationSnapshot, LiveValueEmitter,
    ResolvedParamLane, AUTOMATED_VALUE_EPSILON, AUTOMATED_VALUE_THROTTLE,
};
mod bounce_realtime;
mod busses;
mod chain;
pub(crate) mod clip_loads;
mod clips;
mod external_instrument;
mod external_instrument_ping;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use external_instrument::{
    check_external_instrument_devices_in_place, clear_external_instrument_in_place,
    mark_external_tracks, resend_external_instrument_patch_in_place,
    set_external_instrument_in_place,
    set_external_instrument_latency_in_place, set_external_instrument_patch_in_place,
    ExternalInstruments,
};
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use external_instrument_ping::{
    detect_impulse_onset, estimate_noise_floor, onset_to_engine_samples, onset_to_ms,
    ping_deadline_reached, OnsetOutcome,
};
pub use clips::transcode_to_wav;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use clips::{partition_deferred_clip_commands, DeferredClipCommand};
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use clips::{
    detect_clip_tempo_in_place, set_clip_fade_in_place, set_clip_gain_in_place,
    set_clip_warp_in_place, set_clip_warp_markers_in_place, MAX_CLIP_GAIN_DB, MIN_CLIP_GAIN_DB,
};
pub(crate) mod id_grant;
mod import_pool;
/// Input-device enumeration on a worker: it can block on the macOS
/// microphone-permission prompt.
pub(crate) mod input_devices;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use import_pool::{
    import_one_to_pool, run_pool_import, run_pool_import_with, PoolImportOutcome,
    POOL_IMPORT_CANCELLED,
};
mod import_queue;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use import_queue::{ImportQueue, MAX_CONCURRENT_IMPORTS};
pub(crate) mod probe;
pub(crate) mod midi;
mod midi_map;
pub(crate) mod plugins;
pub(crate) mod reference;
pub(crate) mod sidechain;
pub(crate) mod scan;
pub(crate) mod take_park;
pub(crate) mod takes;
mod thread;
#[cfg(feature = "test-internals")]
pub use thread::test_support::EngineHandlerHarness;
mod tracks;
mod transport;
pub(crate) mod vocal_analysis;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use plugins::affects_latency;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use tracks::set_track_playback_source_in_place;
#[cfg_attr(not(feature = "test-internals"), allow(unused_imports))]
pub use vocal_analysis::analyze_pitch;
pub mod vocal_render;

