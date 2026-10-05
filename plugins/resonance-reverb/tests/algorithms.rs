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
