//! `types` test group: pure value types and math (`types/`, tempo map, quantize, latency math).
//!
//! One binary per group instead of one per file (code review ARCH-03).
//! Each of these files used to be its own integration-test target; they
//! are still ordinary test files, only the target boundary moved. Add a
//! new test as a module here (or in another group), never as a new
//! top-level `tests/*.rs` file — `tools/arch-invariants` enforces that.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! the `tests/types/` subdirectory.

#[path = "types/aux_send_model.rs"]
mod aux_send_model;
#[path = "types/bar_length_shared.rs"]
mod bar_length_shared;
#[path = "types/clip_warp.rs"]
mod clip_warp;
#[path = "types/fade_curve.rs"]
mod fade_curve;
#[path = "types/input_channel_candidates.rs"]
mod input_channel_candidates;
#[path = "types/pw_latency_math.rs"]
mod pw_latency_math;
#[path = "types/quantize_engine.rs"]
mod quantize_engine;
#[path = "types/sample_to_abs_tick.rs"]
mod sample_to_abs_tick;
#[path = "types/tempo_bar_at_sample_exact.rs"]
mod tempo_bar_at_sample_exact;
#[path = "types/tempo_map.rs"]
mod tempo_map;
#[path = "types/tempo_position_to_bars.rs"]
mod tempo_position_to_bars;
#[path = "types/tempo_reanchor_math.rs"]
mod tempo_reanchor_math;
#[path = "types/transport_pos_beats.rs"]
mod transport_pos_beats;
#[path = "types/types_track.rs"]
mod types_track;
#[path = "types/vocal_tuning_model.rs"]
mod vocal_tuning_model;
