//! Unit tests for the piano-roll quantize overlay geometry (todo #396):
//! the live grid-line offsets and the non-destructive ghost-target preview.
//!
//! The overlay's drawing is a `canvas::Frame` side effect that can't be
//! asserted directly, so both the grid renderer and the ghost preview are
//! built on pure helpers (`quantize_grid_steps`, `ghost_targets`) that this
//! file exercises. The ghost helper is a thin wrapper over the same
//! `resonance_audio::quantize::quantize_notes` the engine applies on
//! Apply, so a passing ghost test means the preview matches the eventual
//! committed result.

use resonance_app::view::midi_editor::{ghost_targets, quantize_grid_steps, QuantizePreview};

use resonance_audio::quantize::{Division, GridValue, QuantizeMode};
use resonance_audio::types::{MidiNote, TempoMap, TICKS_PER_QUARTER_NOTE};

/// One 4/4 bar in ticks (4 quarter notes).
const BAR_4_4: u64 = 4 * TICKS_PER_QUARTER_NOTE; // 1920

fn note(start_tick: u64, duration_ticks: u64) -> MidiNote {
    MidiNote {
        note: 60,
        velocity: 0.8,
        start_tick,
        duration_ticks,
    }
}

fn preview(division: Division, strength: f32, swing: f32) -> QuantizePreview {
    QuantizePreview {
        division,
        strength,
        swing,
        mode: QuantizeMode::StartOnly,
        quantize_ends: false,
        iterative: false,
    }
}

// ---- quantize_grid_steps ----

#[test]
fn straight_sixteenth_grid_spans_the_bar() {
    let g = Division::straight(GridValue::Sixteenth).ticks(); // 120
    let steps = quantize_grid_steps(g, BAR_4_4, 0.0);
    // 16 sixteenths in a 4/4 bar, starting on the downbeat.
    assert_eq!(steps.len(), 16);
    assert_eq!(steps[0], 0);
    assert_eq!(steps[1], 120);
    assert_eq!(*steps.last().unwrap(), 1800);
    // Strictly increasing, never past the bar end.
    assert!(steps.windows(2).all(|w| w[0] < w[1]));
    assert!(steps.iter().all(|&s| s < BAR_4_4));
}

#[test]
fn triplet_eighth_grid_has_twelve_even_steps() {
    let g = Division::triplet(GridValue::Eighth).ticks(); // 240 * 2/3 = 160
    assert_eq!(g, 160);
    let steps = quantize_grid_steps(g, BAR_4_4, 0.0);
    // 1920 / 160 = 12 triplet-eighths per bar.
    assert_eq!(steps.len(), 12);
    assert_eq!(steps[1], 160);
    assert_eq!(steps[2], 320);
}

#[test]
fn swing_delays_only_the_odd_steps() {
    let g = Division::straight(GridValue::Eighth).ticks(); // 240
    // A 2/3 swing pushes the off-beat by g/3 (== 80), the classic feel.
    let steps = quantize_grid_steps(g, BAR_4_4, 2.0 / 3.0);
    // Even steps stay on the straight grid...
    assert_eq!(steps[0], 0);
    assert_eq!(steps[2], 480);
    assert_eq!(steps[4], 960);
    // ...odd steps slide later by the swing delay.
    assert_eq!(steps[1], 240 + 80);
    assert_eq!(steps[3], 720 + 80);
    // Straight vs swung must differ, proving swing is visible in the grid.
    let straight = quantize_grid_steps(g, BAR_4_4, 0.0);
    assert_ne!(steps, straight);
}

#[test]
fn degenerate_inputs_yield_no_grid() {
    assert!(quantize_grid_steps(0, BAR_4_4, 0.0).is_empty());
    assert!(quantize_grid_steps(120, 0, 0.0).is_empty());
}

// ---- ghost_targets ----

#[test]
fn ghost_snaps_selected_note_to_nearest_grid_line() {
    let notes = vec![note(130, 240)]; // 10 ticks past the 120 grid line
    let q = preview(Division::straight(GridValue::Sixteenth), 1.0, 0.0);
    let ghosts = ghost_targets(&notes, &[0], &q, &TempoMap::default());
    assert_eq!(ghosts.len(), 1);
    assert_eq!(ghosts[0].start_tick, 120);
    // StartOnly leaves the duration alone.
    assert_eq!(ghosts[0].duration_ticks, 240);
}

#[test]
fn ghost_leaves_unselected_notes_untouched() {
    let notes = vec![note(130, 240), note(370, 240)];
    let q = preview(Division::straight(GridValue::Sixteenth), 1.0, 0.0);
    // Only index 1 is selected.
    let ghosts = ghost_targets(&notes, &[1], &q, &TempoMap::default());
    // Note 0 is copied through verbatim; note 1 snaps (360 nearest 120 grid).
    assert_eq!(ghosts[0].start_tick, 130);
    assert_eq!(ghosts[1].start_tick, 360);
}

#[test]
fn ghost_partial_strength_lands_between_origin_and_grid() {
    let notes = vec![note(140, 240)]; // 20 past the 120 grid line
    let q = preview(Division::straight(GridValue::Sixteenth), 0.5, 0.0);
    let ghosts = ghost_targets(&notes, &[0], &q, &TempoMap::default());
    // Half-strength pulls 50% of the way back toward 120 → 130.
    assert_eq!(ghosts[0].start_tick, 130);
}

#[test]
fn ghost_preview_matches_an_actual_quantize_pass() {
    // The ghost must predict the engine result: compare the wrapper against
    // a direct quantize_notes call with the same params + clip-tick anchor.
    let notes = vec![note(130, 250), note(605, 130)];
    let q = preview(Division::straight(GridValue::Eighth), 0.8, 0.0);
    let tempo = TempoMap::default();
    let ghost = ghost_targets(&notes, &[0, 1], &q, &tempo);
    let direct = resonance_audio::quantize::quantize_notes(
        &notes,
        &[0, 1],
        q.division,
        q.strength,
        q.swing,
        q.mode,
        q.quantize_ends,
        q.iterative,
        &tempo,
        0,
    );
    assert_eq!(ghost.len(), direct.len());
    for (g, d) in ghost.iter().zip(direct.iter()) {
        assert_eq!(g.start_tick, d.start_tick);
        assert_eq!(g.duration_ticks, d.duration_ticks);
        assert_eq!(g.note, d.note);
        assert_eq!(g.velocity.to_bits(), d.velocity.to_bits());
    }
}
