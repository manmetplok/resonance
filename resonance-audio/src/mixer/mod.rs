//! Audio mixing for the realtime callback thread. Must be
//! allocation-free (everything writes into pre-allocated buffers) and
//! non-blocking.
//!
//! `mod.rs` owns no mixing code of its own — it declares the submodules
//! and re-exports the handful of items the engine, the offline bounce and
//! the test/bench surfaces reach for. The work is split by concern:
//!
//! - [`callback`]: [`mix_audio`], the top-level callback, and the branches
//!   it delegates to (reference A/B, count-in, stopped-monitor, playing,
//!   loop seam) plus the borrowed parameter structs they share.
//! - [`render_core`]: the order of the phases in one render block, shared
//!   with the offline bounce path and parameterized by `RenderStrategy`;
//!   the phases themselves live in [`render`].
//! - [`track_block`]: live wrapper over `render_core`.
//! - [`take_comp`]: the comped cover of a take group's loop slot — which
//!   recorded take is audible where, with an equal-power crossfade at each
//!   segment seam. Read by `render_core` on both the live and bounce path.
//! - [`midi_events`]: per-block MIDI tick→sample collection.
//! - [`midi_stash`]: notes parked while a plugin's mutex is contended.
//! - [`live_midi`]: hardware-MIDI pickup on the audio thread.
//! - [`monitor`]: monitor-ring pacing and live-input pass-through.
//! - [`master`]: master FX insert chain + master volume / peaks.
//! - [`click`]: count-in and timeline metronome click synthesis.
//! - [`audition`]: the transport-independent preview overlay.
//! - [`automation_apply`]: automation-lane evaluation for the mix params.
//! - [`common`]: tiny helpers (pan-law gains, transport latching, the
//!   silent fallback playhead, the loop-seam panic routine).
//! - [`recording_push`]: whole-frame pushes into the recording ring (the
//!   capture side's twin of the monitor ring's pacing).
//! - [`test_support`]: harnesses that drive the callback and the render
//!   core from tests and benches; no realtime code depends on it.

mod audition;
mod automation_apply;
mod callback;
mod click;
mod common;
mod live_midi;
mod master;
mod midi_events;
mod midi_stash;
mod monitor;
mod recording_push;
pub(crate) mod render;
mod render_core;
mod take_comp;
mod test_support;
mod track_block;

pub(crate) use callback::{mix_audio, CallbackInputs, CallbackScratch, MixFn};

pub use audition::mix_audition_overlay;
pub use automation_apply::{auto_gain_ramp, auto_master_volume, auto_muted};
pub use common::{commit_playhead, ramped_gain, sum_to_output, sum_to_stereo, transport_pos_beats};
pub(crate) use common::TransportContinuity;
pub use live_midi::live_instrument_for;
pub use midi_events::collect_midi_events_bounce;
pub(crate) use midi_events::MAX_MIDI_EVENTS_PER_BUFFER;
pub use midi_stash::{MidiStash, NoteSink, StashEntry};
pub use monitor::{monitor_catchup_skip, monitor_read_len, MonitorDrain, MONITOR_DRAIN_STREAK};
pub use recording_push::{push_recording_frames, whole_frame_push_len};
pub(crate) use render_core::{render_block, BlockInputs, BlockScratch, RenderStrategy};
pub use render_core::{mix_track_clips, recorded_monitor_gate, CLIP_DECLICK_FRAMES};
pub use take_comp::{
    build_comp_table, mix_track_comp, CompRenderTable, CompSpan, TrackComp, COMP_XFADE_FRAMES,
};
pub use test_support::{
    render_aux_for_test, render_aux_with_comp_for_test, render_take_comp_borrowed_for_test,
    render_take_comp_for_test, MixAudioHarness, RenderBenchHarness,
};

pub(crate) use crate::limits::MAX_PLUGIN_OUTPUT_PORTS;
