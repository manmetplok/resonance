//! Chord-state ops: AddChord, EditChord, MoveChord, ResizeChord,
//! DeleteChord. Each one validates the new state against the section's
//! length + neighbouring chords and, on success, fans the change out to
//! every dependent instrument lane via `propagate_chord_change`.

use resonance_music_theory::{Chord, ChordQuality, PitchClass};

use super::regenerate::propagate_chord_change;
use crate::compose::invariants::{chord_fits_in_section, chord_overlaps};
use crate::compose::ChordState;

pub(super) fn handle_add(
    r: &mut crate::Resonance,
    definition_id: u64,
    start_beat: u32,
    duration_beats: u32,
    root: PitchClass,
    quality: ChordQuality,
    time_sig_num: u8,
) {
    let (length_bars, overlap) = match r.compose.find_definition(definition_id) {
        Some(d) => (
            d.length_bars,
            chord_overlaps(&d.chords, start_beat, duration_beats, None),
        ),
        None => return,
    };
    if !chord_fits_in_section(start_beat, duration_beats, length_bars, time_sig_num) {
        r.compose.last_error = Some("Chord does not fit inside the section".into());
        return;
    }
    if overlap {
        r.compose.last_error = Some("Chord overlaps another chord".into());
        return;
    }
    let id = r.compose.fresh_id();
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.chords.push(ChordState {
            id,
            start_beat,
            duration_beats,
            chord: Chord::new(root, quality),
        });
        def.chords.sort_by_key(|c| c.start_beat);
    }
    r.compose.last_error = None;
    propagate_chord_change(r, definition_id);
}

pub(super) fn handle_edit(
    r: &mut crate::Resonance,
    definition_id: u64,
    chord_id: u64,
    chord: Chord,
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(c) = def.chords.iter_mut().find(|c| c.id == chord_id) {
            c.chord = chord;
        }
    }
    r.compose.last_error = None;
    propagate_chord_change(r, definition_id);
}

pub(super) fn handle_move(
    r: &mut crate::Resonance,
    definition_id: u64,
    chord_id: u64,
    start_beat: u32,
    time_sig_num: u8,
) {
    let (length_bars, duration_beats, overlap) = match r.compose.find_definition(definition_id) {
        Some(d) => {
            let c = match d.chords.iter().find(|c| c.id == chord_id) {
                Some(c) => c,
                None => return,
            };
            (
                d.length_bars,
                c.duration_beats,
                chord_overlaps(&d.chords, start_beat, c.duration_beats, Some(chord_id)),
            )
        }
        None => return,
    };
    if !chord_fits_in_section(start_beat, duration_beats, length_bars, time_sig_num) {
        r.compose.last_error = Some("Chord would move outside the section".into());
        return;
    }
    if overlap {
        r.compose.last_error = Some("Chord would overlap another chord".into());
        return;
    }
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(c) = def.chords.iter_mut().find(|c| c.id == chord_id) {
            c.start_beat = start_beat;
        }
        def.chords.sort_by_key(|c| c.start_beat);
    }
    r.compose.last_error = None;
    propagate_chord_change(r, definition_id);
}

pub(super) fn handle_resize(
    r: &mut crate::Resonance,
    definition_id: u64,
    chord_id: u64,
    duration_beats: u32,
    time_sig_num: u8,
) {
    if duration_beats == 0 {
        r.compose.last_error = Some("Chord duration must be at least 1 beat".into());
        return;
    }
    let (length_bars, start_beat, overlap) = match r.compose.find_definition(definition_id) {
        Some(d) => {
            let c = match d.chords.iter().find(|c| c.id == chord_id) {
                Some(c) => c,
                None => return,
            };
            (
                d.length_bars,
                c.start_beat,
                chord_overlaps(&d.chords, c.start_beat, duration_beats, Some(chord_id)),
            )
        }
        None => return,
    };
    if !chord_fits_in_section(start_beat, duration_beats, length_bars, time_sig_num) {
        r.compose.last_error = Some("Chord would extend past the section".into());
        return;
    }
    if overlap {
        r.compose.last_error = Some("Chord would overlap another chord".into());
        return;
    }
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(c) = def.chords.iter_mut().find(|c| c.id == chord_id) {
            c.duration_beats = duration_beats;
        }
    }
    r.compose.last_error = None;
    propagate_chord_change(r, definition_id);
}

pub(super) fn handle_delete(r: &mut crate::Resonance, definition_id: u64, chord_id: u64) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.chords.retain(|c| c.id != chord_id);
    }
    if r.compose.selected_chord_id == Some(chord_id) {
        r.compose.selected_chord_id = None;
    }
    r.compose.last_error = None;
    propagate_chord_change(r, definition_id);
}
