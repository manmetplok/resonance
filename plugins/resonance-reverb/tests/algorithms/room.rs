//! R4 (Room, Chamber) and Ambience (spec phase R6, built with R4):
//! reverb-algorithms.md §5.2 acceptance rows, stability, reset, size
//! sweeps and bit-exact goldens.
//!
//! Every render goes through `ReverbDsp::with_engines(SR, &[algorithm])`
//! with every engine setter called explicitly (`room/common.rs`).

#[path = "room/acceptance.rs"]
mod acceptance;
#[path = "room/common.rs"]
mod common;
#[path = "room/golden.rs"]
mod golden;
#[path = "room/stability.rs"]
mod stability;
