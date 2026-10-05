//! Group binary for the reverb-algorithms work (reverb-algorithms.md):
//! the IR-metric baseline, the algorithm switch, tempo sync, and one
//! module per algorithm as each phase lands. Add a module here rather than
//! a new top-level test file (the test-binary ratchet).

#[path = "algorithms/baseline.rs"]
mod baseline;
#[path = "algorithms/switching.rs"]
mod switching;
#[path = "algorithms/sync.rs"]
mod sync;
#[path = "algorithms/plate.rs"]
mod plate;
#[path = "algorithms/room.rs"]
mod room;
#[path = "algorithms/hall.rs"]
mod hall;
#[path = "algorithms/common.rs"]
mod common;
#[path = "algorithms/spring.rs"]
mod spring;
#[path = "algorithms/nonlinear.rs"]
mod nonlinear;
#[path = "algorithms/shimmer.rs"]
mod shimmer;
