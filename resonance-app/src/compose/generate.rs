//! Chord progression + derived arrangement generation for compose sections.
//!
//! This module is the orchestration layer between the pure generators in
//! `resonance-music-theory` and the audio engine. The pure functions take
//! `TimedChord`s and produce `GeneratedNote`s; the helpers here convert
//! to the engine's `MidiNote` / `MidiClip` types and know how to find the
//! current target track for each role.

use resonance_audio::types::MidiNote;
use resonance_music_theory::{
    derive_bass, derive_bass_motif, derive_melody, derive_motif_melody_with_section, derive_pad,
    BassParams, BassStyle, GeneratedNote, MelodyParams, MelodyStyle, MotifSource, PadParams,
    TimedChord,
};
use serde::{Deserialize, Serialize};

use super::ChordState;

/// Per-section persisted knobs for the generators. Held on each
/// `SectionDefinitionState` so the UI can remember the user's choices
/// across sections (e.g. the bass track style sticks to "walking"
/// for a specific verse even if another section uses "root hold").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateParams {
    /// Chord count for the progression walker.
    pub chord_count: u32,
    /// Beats each chord occupies on the section grid (e.g. 4 = one
    /// chord per bar in 4/4, 2 = one per half-bar).
    pub beats_per_chord: u32,
    /// Build diatonic seventh chords instead of triads.
    pub seventh_chords: bool,
    pub pad: PadParams,
    pub bass: BassParams,
    pub melody: MelodyParams,
}

impl Default for GenerateParams {
    fn default() -> Self {
        Self {
            chord_count: 4,
            beats_per_chord: 4,
            seventh_chords: false,
            pad: PadParams::default(),
            bass: BassParams::default(),
            melody: MelodyParams::default(),
        }
    }
}

/// Overlay the global chord track's user-pinned regions onto a section's
/// chord progression (doc #168, todo #445). The section keeps its chord
/// *rhythm* — each [`ChordState`]'s `start_beat` / `duration_beats` is
/// untouched — but any chord whose grid position falls inside a pinned
/// region is re-symboled to that region's chord, so regenerated parts
/// follow the user's pinned harmony. Chords with no covering pinned
/// region keep their generated symbol, and non-pinned regions never
/// override (only explicit user pins constrain regeneration).
///
/// `section_start_sample` is where the section sits on the timeline and
/// `samples_per_beat` is the compose grid's fixed samples-per-beat (the
/// same constant `compose_samples_per_bar` is built from). Together they
/// map each section beat onto the absolute sample axis the chord track is
/// positioned on, so a pinned region is matched against the very beat the
/// generator will voice.
pub fn overlay_pinned_chords(
    section_chords: &[ChordState],
    chord_track: &crate::chord_track::ChordTrack,
    section_start_sample: u64,
    samples_per_beat: f64,
) -> Vec<ChordState> {
    section_chords
        .iter()
        .map(|c| {
            let beat_sample =
                section_start_sample + (c.start_beat as f64 * samples_per_beat) as u64;
            match chord_track.regions.iter().find(|r| {
                r.pinned && r.start_sample <= beat_sample && beat_sample < r.end_sample
            }) {
                Some(region) => ChordState {
                    chord: region.chord,
                    ..c.clone()
                },
                None => c.clone(),
            }
        })
        .collect()
}

/// Convert the app's `ChordState`s into the music-theory crate's input type.
pub fn to_timed_chords(chords: &[ChordState]) -> Vec<TimedChord> {
    chords
        .iter()
        .map(|c| TimedChord {
            chord: c.chord,
            start_beat: c.start_beat,
            duration_beats: c.duration_beats,
        })
        .collect()
}

/// Convert music-theory crate output into engine-side `MidiNote`s.
pub fn to_midi_notes(notes: &[GeneratedNote]) -> Vec<MidiNote> {
    notes
        .iter()
        .map(|n| MidiNote {
            note: n.note,
            velocity: n.velocity,
            start_tick: n.start_tick,
            duration_ticks: n.duration_ticks,
        })
        .collect()
}

/// What kind of part to derive. Distinct from `TrackRole` because we
/// might one day want to derive a second pad onto a Lead-role track,
/// or allow stacked Pad generators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeriveKind {
    Pad,
    Bass,
    Lead,
}

/// Run the generator for a derive kind against a chord list and the
/// section's scale, returning the engine-ready MIDI notes. Motif-style
/// bass and melody lanes route through the section-shared `motif_source`
/// so they share the same underlying motif identity within a section
/// (whether it's procedurally generated or hand-drawn).
pub fn derive_notes(
    kind: DeriveKind,
    chords: &[ChordState],
    scale: Option<resonance_music_theory::Scale>,
    params: &GenerateParams,
    motif_source: &MotifSource,
    ticks_per_beat: u32,
    seed: u64,
) -> Vec<MidiNote> {
    let timed = to_timed_chords(chords);
    let generated = match kind {
        DeriveKind::Pad => derive_pad(&timed, scale, &params.pad, ticks_per_beat),
        DeriveKind::Bass => match params.bass.style {
            BassStyle::Motif => derive_bass_motif(
                &timed,
                scale,
                &params.bass,
                motif_source,
                seed,
                ticks_per_beat,
            ),
            _ => derive_bass(&timed, scale, &params.bass, ticks_per_beat),
        },
        DeriveKind::Lead => match params.melody.style {
            MelodyStyle::Motif => derive_motif_melody_with_section(
                &timed,
                scale,
                &params.melody,
                motif_source,
                seed,
                ticks_per_beat,
            ),
            _ => derive_melody(&timed, scale, &params.melody, ticks_per_beat, seed),
        },
    };
    to_midi_notes(&generated)
}
