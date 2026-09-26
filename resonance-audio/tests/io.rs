//! `io` test group: file I/O and decode (`io/`, `decode`): WAV parsing, import to pool, clip loading, analysis.
//!
//! One binary per group instead of one per file (code review ARCH-03).
//! Each of these files used to be its own integration-test target; they
//! are still ordinary test files, only the target boundary moved. Add a
//! new test as a module here (or in another group), never as a new
//! top-level `tests/*.rs` file — `tools/arch-invariants` enforces that.
//!
//! The `#[path]` attributes are load-bearing: `mod foo;` in a crate-root
//! file resolves against that file's own directory (`tests/`), not against
//! the `tests/io/` subdirectory.

#[path = "io/clip_pitch_analysis.rs"]
mod clip_pitch_analysis;
#[path = "io/clip_tempo_detect.rs"]
mod clip_tempo_detect;
#[path = "io/import_audio_to_pool.rs"]
mod import_audio_to_pool;
#[path = "io/load_clip_offthread.rs"]
mod load_clip_offthread;
#[path = "io/load_wav_rate_mismatch.rs"]
mod load_wav_rate_mismatch;
#[path = "io/pw_output_smoke.rs"]
mod pw_output_smoke;
#[path = "io/reference_analysis.rs"]
mod reference_analysis;
#[path = "io/wav_chunk_parse.rs"]
mod wav_chunk_parse;
