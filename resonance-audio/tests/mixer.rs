//! `mixer` test group: the audio-callback render path (`mixer/`): mix, monitor, recording push, stems, sidechain and sub-track routing.
//!
//! One binary per group instead of one per file (code review ARCH-03).
//! Each of these files used to be its own integration-test target; they
//! are still ordinary test files, only the target boundary moved. Add a
//! new test as a module here (or in another group), never as a new
//! top-level `tests/*.rs` file — `tools/arch-invariants` enforces that.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! the `tests/mixer/` subdirectory.

#[path = "mixer/multi_out_harness/mod.rs"]
mod multi_out_harness;
#[path = "mixer/note_recorder/mod.rs"]
mod note_recorder;

#[path = "mixer/audition_preview.rs"]
mod audition_preview;
#[path = "mixer/automation_comp_delay.rs"]
mod automation_comp_delay;
#[path = "mixer/automation_live_value.rs"]
mod automation_live_value;
#[path = "mixer/automation_render.rs"]
mod automation_render;
#[path = "mixer/aux_send_render.rs"]
mod aux_send_render;
#[path = "mixer/clip_fade_gain_render.rs"]
mod clip_fade_gain_render;
#[path = "mixer/cycle_load.rs"]
mod cycle_load;
#[path = "mixer/freeze_playback_substitution.rs"]
mod freeze_playback_substitution;
#[path = "mixer/graph_rate_assert.rs"]
mod graph_rate_assert;
#[path = "mixer/latency_comp.rs"]
mod latency_comp;
#[path = "mixer/measure_mix.rs"]
mod measure_mix;
#[path = "mixer/midi_event_cap.rs"]
mod midi_event_cap;
#[path = "mixer/midi_event_window.rs"]
mod midi_event_window;
#[path = "mixer/midi_stash.rs"]
mod midi_stash;
#[path = "mixer/mix_audio_parity.rs"]
mod mix_audio_parity;
#[path = "mixer/mixer_gain_ramp.rs"]
mod mixer_gain_ramp;
#[path = "mixer/monitor_fallback_resample.rs"]
mod monitor_fallback_resample;
#[path = "mixer/monitor_ring_alignment.rs"]
mod monitor_ring_alignment;
#[path = "mixer/playhead_discontinuity_flush.rs"]
mod playhead_discontinuity_flush;
#[path = "mixer/recorded_monitor_gating.rs"]
mod recorded_monitor_gating;
#[path = "mixer/recorded_outbound_gating.rs"]
mod recorded_outbound_gating;
#[path = "mixer/recording_drain.rs"]
mod recording_drain;
#[path = "mixer/recording_overflow_event.rs"]
mod recording_overflow_event;
#[path = "mixer/recording_start_latch.rs"]
mod recording_start_latch;
#[path = "mixer/recording_whole_frame_push.rs"]
mod recording_whole_frame_push;
#[path = "mixer/reference_monitor.rs"]
mod reference_monitor;
#[path = "mixer/render_block_parity.rs"]
mod render_block_parity;
#[path = "mixer/sidechain_key_delivery.rs"]
mod sidechain_key_delivery;
#[path = "mixer/silent_advance_loop_seam.rs"]
mod silent_advance_loop_seam;
#[path = "mixer/solo_predicate.rs"]
mod solo_predicate;
#[path = "mixer/stem_bus_sub_track_render.rs"]
mod stem_bus_sub_track_render;
#[path = "mixer/stem_render.rs"]
mod stem_render;
#[path = "mixer/stem_sub_track_render.rs"]
mod stem_sub_track_render;
#[path = "mixer/stopped_instrument_preview.rs"]
mod stopped_instrument_preview;
#[path = "mixer/sub_track_parent_fader.rs"]
mod sub_track_parent_fader;
#[path = "mixer/take_comp_render.rs"]
mod take_comp_render;
#[path = "mixer/underrun_rate_limiter.rs"]
mod underrun_rate_limiter;
