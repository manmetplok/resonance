//! Pure helpers for picking velocity layers and round-robin takes.
//!
//! Kept out of `DrumSampler` so they're unit-testable without
//! constructing a full sampler (which spawns a janitor thread).

/// Maximum velocity layer count we track per pad in the round-robin counter
/// array. Drummica's deepest pad has 28 layers, so 32 is a comfortable cap.
pub const MAX_LAYERS: usize = 32;

/// Map a MIDI velocity in [0, 1] onto a layer index in [0, n_layers).
///
/// Uses equal-width buckets. Callers must guarantee `n_layers >= 1`; with
/// `n_layers == 1` the result is always 0.
pub fn pick_velocity_layer(velocity: f32, n_layers: usize) -> usize {
    debug_assert!(n_layers >= 1, "n_layers must be at least 1");
    if n_layers <= 1 {
        return 0;
    }
    ((velocity.clamp(0.0, 1.0) * n_layers as f32) as usize).min(n_layers - 1)
}

/// Advance a round-robin counter and return the RR index for this trigger.
/// Wraps the counter at `u32::MAX` so it can run indefinitely.
pub fn pick_rr(counter: &mut u32, n_rrs: usize) -> usize {
    debug_assert!(n_rrs >= 1, "n_rrs must be at least 1");
    let idx = (*counter as usize) % n_rrs;
    *counter = counter.wrapping_add(1);
    idx
}
