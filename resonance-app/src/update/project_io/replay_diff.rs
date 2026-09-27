//! Helpers of the undo/redo restore's diff (ARCH-01 A-13).
//!
//! The restore itself is `reconcile::reconcile_all` with `old` set, called
//! from `Resonance::restore_from_snapshot`: one engine command per changed
//! scalar, one add or remove per entity that differs, and every plugin
//! instance that is kept stays alive. Since A-13j it is the only undo
//! path — the `ClearAll` fallback (and `structurally_compatible`, which
//! chose it) is gone.

use resonance_audio::types::*;

/// `MidiNote` is a plain bag of `u8/f32/u64` fields but does not derive
/// `PartialEq` (the engine has no need for it). Comparing field-wise
/// here keeps the diff replay self-contained without touching the
/// engine crate's public API.
pub fn midi_notes_equal(a: &[MidiNote], b: &[MidiNote]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| {
        x.note == y.note
            && x.velocity.to_bits() == y.velocity.to_bits()
            && x.start_tick == y.start_tick
            && x.duration_ticks == y.duration_ticks
    })
}
