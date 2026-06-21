//! Deterministic humanize: add seeded random timing and velocity jitter.

use crate::types::MidiNote;

use super::rng::Rng;

/// Apply deterministic timing + velocity jitter to the selected notes.
///
/// * `timing_ticks` — maximum absolute timing offset, in ticks (the
///   per-note offset is uniform in `-timing_ticks..=timing_ticks`).
/// * `vel_amt` — velocity jitter fraction in `0.0..=1.0`; velocity is
///   scaled by `1 + U(-vel_amt, vel_amt)` and clamped to `0.0..=1.0`.
/// * `seed` — RNG seed; identical inputs always produce identical output.
///
/// Notes are processed by index and never reordered, merged, or dropped.
/// Out-of-range indices in `selection` are ignored.
pub fn humanize_notes(
    notes: &[MidiNote],
    selection: &[usize],
    timing_ticks: u32,
    vel_amt: f32,
    seed: u64,
) -> Vec<MidiNote> {
    let mut out = notes.to_vec();
    let vel_amt = vel_amt.clamp(0.0, 1.0) as f64;
    for &i in selection {
        let Some(n) = out.get_mut(i) else { continue };
        // Salt by index so each note has an independent stream; the
        // stream is fixed by (seed, index), hence reproducible.
        let mut rng = Rng::new(seed, i as u64);

        if timing_ticks > 0 {
            let off = (rng.next_bipolar() * timing_ticks as f64).round() as i64;
            n.start_tick = (n.start_tick as i64 + off).max(0) as u64;
        }
        if vel_amt > 0.0 {
            let scale = 1.0 + rng.next_bipolar() * vel_amt;
            n.velocity = (n.velocity as f64 * scale).clamp(0.0, 1.0) as f32;
        }
    }
    out
}
