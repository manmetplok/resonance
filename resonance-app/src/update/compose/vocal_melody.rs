//! What a chord or section-length edit does to a vocal lane's melody
//! (vocal-melody-on-chord-change, BUG-70). The edit no longer re-derives
//! a melody it would change: the lane is kept, re-fitted or regenerated
//! as the edit's caller asked (a control call's `vocal_melody`), as the
//! project remembers, or — from the GUI, with nothing remembered — kept
//! for now and put to the user in a question
//! ([`vocal_melody_prompt`](super::vocal_melody_prompt)).
//!
//! - **Keep** leaves the notes alone and flags the lane
//!   `melody_predates_chords`. A length change re-installs the kept notes
//!   at the new clip length: notes past a shrunk end are cut, a grown end
//!   is silent.
//! - **Refit** re-pitches the notes in place
//!   ([`resonance_music_theory::refit_melody`]); rhythm, note count and
//!   the per-note lyric table stay.
//! - **Regenerate** is what every chord edit did before: re-derive from
//!   the generator with the lane's seed
//!   ([`super::vocal_render::rederive_vocal_melody`]).
//!
//! A lane whose re-derived melody would equal what it holds (notes and
//! lyric side table), or that holds no notes yet, has nothing to lose: it
//! is re-derived as before, unasked and unreported.
//!
//! **Hand edits.** A lane is `hand_edited` once anything but the
//! generator changes its notes or lyric side table: every note-editing
//! `MidiEditorMessage` on its clip (the vocal roll, `notes.*`,
//! `notes.import_midi`) reports through [`note_hand_edit`]. A (re)generate
//! clears it. Both flags live on the lane config, so they ride undo and
//! save.

use resonance_audio::types::{AudioCommand, ClipId, MidiNote, TrackId, TICKS_PER_QUARTER_NOTE};

use crate::compose::vocal_melody::{AffectedVocalLane, LaneMelodyFlags, VocalMelodyChoice};
use crate::compose::{LaneGeneratorConfig, LaneGeneratorKind, SectionDefinitionState};

/// The lane's flags when it is a vocal lane.
pub(crate) fn lane_flags(
    r: &crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> LaneMelodyFlags {
    r.compose
        .find_definition(definition_id)
        .and_then(|d| d.lane_generators.get(&track_id))
        .filter(|c| matches!(c.kind, LaneGeneratorKind::Vocal(_)))
        .map(|c| c.melody)
        .unwrap_or_default()
}

fn update_flags(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl FnOnce(&mut LaneMelodyFlags),
) {
    if let Some(cfg) = r
        .compose
        .find_definition_mut(definition_id)
        .and_then(|d| d.lane_generators.get_mut(&track_id))
        .filter(|c| matches!(c.kind, LaneGeneratorKind::Vocal(_)))
    {
        f(&mut cfg.melody);
    }
}

/// The lane was (re)generated: neither hand-edited nor out of date.
pub(super) fn clear_lane_flags(r: &mut crate::Resonance, definition_id: u64, track_id: TrackId) {
    update_flags(r, definition_id, track_id, |f| *f = LaneMelodyFlags::default());
}

/// Install `config` as the lane's generator. Replacing a vocal lane's
/// settings with a vocal config that carries no flags of its own
/// (`vocal.set_params`, `section.set_lane_generator`) keeps the lane's
/// flags: new settings do not make hand-written notes generated.
pub(super) fn install_lane_config(
    def: &mut SectionDefinitionState,
    track_id: TrackId,
    mut config: LaneGeneratorConfig,
) {
    if let Some(old) = def.lane_generators.get(&track_id) {
        let both_vocal = matches!(old.kind, LaneGeneratorKind::Vocal(_))
            && matches!(config.kind, LaneGeneratorKind::Vocal(_));
        if both_vocal && config.melody.is_clear() {
            config.melody = old.melody;
        }
    }
    def.lane_generators.insert(track_id, config);
}

/// A note or lyric edit landed on `clip_id`: if it is a vocal lane's
/// clip, the lane's melody is now hand-edited, and no longer out of date
/// (its notes were edited again since the chords changed).
pub(crate) fn note_hand_edit(r: &mut crate::Resonance, clip_id: ClipId) {
    let mut lanes: Vec<(u64, TrackId)> = r
        .compose
        .derived_clips
        .iter()
        .filter(|(_, id)| **id == clip_id)
        .map(|((def, _, track), _)| (*def, *track))
        .collect();
    if lanes.is_empty() {
        // A lane loaded without its clip mapping resolves its clip by
        // position, the way a render does.
        lanes = super::vocal_control::all_vocal_lanes(r)
            .into_iter()
            .filter(|&(def, track)| lane_clip_ids(r, def, track).contains(&clip_id))
            .collect();
    }
    for (definition_id, track_id) in lanes {
        update_flags(r, definition_id, track_id, |f| {
            f.hand_edited = true;
            f.melody_predates_chords = false;
        });
    }
}

/// The lane's clips, one per placement that has one, in placement order.
fn lane_clip_ids(r: &crate::Resonance, definition_id: u64, track_id: TrackId) -> Vec<ClipId> {
    let placements: Vec<(u64, u32)> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.id, p.start_bar))
        .collect();
    let mapped: Vec<ClipId> = placements
        .iter()
        .filter_map(|(pid, _)| {
            r.compose
                .derived_clips
                .get(&(definition_id, *pid, track_id))
                .copied()
        })
        .collect();
    if !mapped.is_empty() {
        return mapped;
    }
    super::vocal_render::lane_midi_clip(r, definition_id, track_id, &placements)
        .into_iter()
        .collect()
}

/// The pitches of the lane's first clip, in note order.
pub(crate) fn lane_pitches(r: &crate::Resonance, definition_id: u64, track_id: TrackId) -> Vec<u8> {
    lane_content(r, definition_id, track_id)
        .map(|(notes, _)| notes.iter().map(|n| n.note).collect())
        .unwrap_or_default()
}

fn notes_equal(a: &[MidiNote], b: &[MidiNote]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| crate::state::note_matches(x, y))
}

/// The lane's notes and padded lyric side table, from its first clip.
fn lane_content(
    r: &crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Option<(Vec<MidiNote>, Vec<String>)> {
    let clip_id = *lane_clip_ids(r, definition_id, track_id).first()?;
    let clip = r.midi_clips.iter().find(|c| c.id == clip_id)?;
    let notes = clip.notes.as_ref().clone();
    let mut lyrics = r
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip_id)
        .cloned()
        .unwrap_or_default();
    lyrics.resize(notes.len(), String::new());
    Some((notes, lyrics))
}

/// A chord or section-length edit reached this vocal lane (from
/// `regenerate_lane_as_side_effect`). See the module docs.
pub(super) fn on_chord_change(r: &mut crate::Resonance, definition_id: u64, track_id: TrackId) {
    let derived = super::vocal_render::compute_vocal_melody(r, definition_id, track_id);
    let current =
        lane_content(r, definition_id, track_id).filter(|(notes, _)| !notes.is_empty());
    let would_change = match (&derived, &current) {
        (Some(d), Some((notes, lyrics))) => {
            let mut derived_lyrics = d.lyrics.clone();
            derived_lyrics.resize(d.midi_notes.len(), String::new());
            !notes_equal(&d.midi_notes, notes) || derived_lyrics != *lyrics
        }
        // Nothing derives (no draft, no chords): the edit leaves the
        // lane as it is, as it always did.
        (None, _) => false,
        // Nothing to lose yet.
        (Some(_), None) => false,
    };
    if !would_change {
        if let Some(derived) = derived {
            let _ = super::vocal_render::install_vocal_melody(r, derived);
        }
        return;
    }
    let state = &r.compose.vocal_melody;
    let chosen = state.requested.or(state.remembered);
    let outcome = chosen.unwrap_or(VocalMelodyChoice::Keep);
    apply_outcome(r, definition_id, track_id, outcome);
    let state = &mut r.compose.vocal_melody;
    state.affected.push(AffectedVocalLane {
        definition_id,
        track_id,
        outcome,
    });
    if chosen.is_none() && !state.undecided.contains(&(definition_id, track_id)) {
        state.undecided.push((definition_id, track_id));
    }
}

/// Carry out one lane's choice against the section as it is now.
pub(super) fn apply_outcome(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    outcome: VocalMelodyChoice,
) {
    match outcome {
        VocalMelodyChoice::Keep => keep_lane(r, definition_id, track_id),
        VocalMelodyChoice::Refit => {
            let _ = refit_lane(r, definition_id, track_id);
        }
        VocalMelodyChoice::Regenerate => {
            super::vocal_render::rederive_vocal_melody(r, definition_id, track_id)
        }
    }
}

/// The section's clip length now, in ticks.
fn section_ticks(r: &crate::Resonance, definition_id: u64) -> Option<u64> {
    let def = r.compose.find_definition(definition_id)?;
    let numerator = super::section_meter(r, definition_id).numerator;
    Some(u64::from(def.length_bars) * u64::from(numerator) * TICKS_PER_QUARTER_NOTE)
}

/// Keep the lane's notes and flag them as written against other chords.
fn keep_lane(r: &mut crate::Resonance, definition_id: u64, track_id: TrackId) {
    update_flags(r, definition_id, track_id, |f| f.melody_predates_chords = true);
    rewrite_lane_notes(r, definition_id, track_id, |notes| notes.to_vec());
}

/// Re-fit the lane's pitches to its section's chords (the chord track's
/// pinned regions and key included, as a derive reads them). Returns how
/// many of the first clip's notes changed pitch, or `None` when the lane
/// has no notes to re-fit.
pub(crate) fn refit_lane(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Option<usize> {
    let mut def = r.compose.find_definition(definition_id).cloned()?;
    super::regenerate::apply_chord_track_harmony(r, definition_id, &mut def);
    let chords = crate::compose::generate::to_timed_chords(&def.chords);
    let beats_per_bar = u32::from(super::section_meter(r, definition_id).numerator);
    let first = lane_content(r, definition_id, track_id)?.0;
    if first.is_empty() {
        return None;
    }
    let refit = |notes: &[MidiNote]| -> Vec<MidiNote> {
        let generated: Vec<resonance_music_theory::GeneratedNote> = notes
            .iter()
            .map(|n| resonance_music_theory::GeneratedNote {
                note: n.note,
                velocity: n.velocity,
                start_tick: n.start_tick,
                duration_ticks: n.duration_ticks,
            })
            .collect();
        resonance_music_theory::refit_melody(
            &generated,
            &chords,
            def.scale,
            TICKS_PER_QUARTER_NOTE as u32,
            beats_per_bar,
        )
        .into_iter()
        .zip(notes)
        .map(|(g, n)| MidiNote { note: g.note, ..n.clone() })
        .collect()
    };
    let changed = refit(&first)
        .iter()
        .zip(&first)
        .filter(|(a, b)| a.note != b.note)
        .count();
    rewrite_lane_notes(r, definition_id, track_id, refit);
    update_flags(r, definition_id, track_id, |f| f.melody_predates_chords = false);
    Some(changed)
}

/// Rewrite every clip of the lane through `f`, at the section's current
/// length: a clip whose length still matches is edited in place (its id
/// and lyric table stay), one whose section changed length is
/// re-installed at the new length with the notes past its end cut.
fn rewrite_lane_notes(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl Fn(&[MidiNote]) -> Vec<MidiNote>,
) {
    let Some(duration) = section_ticks(r, definition_id) else {
        return;
    };
    let placements: Vec<u64> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| p.id)
        .collect();
    for placement_id in placements {
        let key = (definition_id, placement_id, track_id);
        let Some(clip_id) = r.compose.derived_clips.get(&key).copied() else {
            continue;
        };
        let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
            continue;
        };
        let old: Vec<MidiNote> = clip.notes.as_ref().clone();
        let notes = f(&old);
        if clip.duration_ticks == duration {
            if !notes_equal(&notes, &old) {
                let _ = r.engine.send(AudioCommand::SetMidiClipNotes {
                    clip_id,
                    notes: notes.clone(),
                });
                crate::engine_events::midi::optimistic_set_notes(r, clip_id, notes);
            }
            continue;
        }
        let (start_sample, name) = (clip.start_sample, clip.name.clone());
        let lyrics = r
            .compose
            .vocal_audio
            .clip_lyrics
            .get(&clip_id)
            .cloned()
            .unwrap_or_default();
        let (notes, lyrics) = cut_to(notes, lyrics, duration);
        super::vocal_midi_install::VocalMidiInstall {
            definition_id,
            track_id,
            placements: &[(placement_id, start_sample)],
            duration_ticks: duration,
            midi_notes: &notes,
            lyrics: &lyrics,
            name: &name,
        }
        .install(r);
    }
}

/// Drop the notes that start at or past `duration`, shorten the ones that
/// cross it, and keep the lyric table aligned.
fn cut_to(
    notes: Vec<MidiNote>,
    mut lyrics: Vec<String>,
    duration: u64,
) -> (Vec<MidiNote>, Vec<String>) {
    lyrics.resize(notes.len(), String::new());
    notes
        .into_iter()
        .zip(lyrics)
        .filter(|(n, _)| n.start_tick < duration)
        .map(|(mut n, l)| {
            n.duration_ticks = n.duration_ticks.min(duration - n.start_tick).max(1);
            (n, l)
        })
        .unzip()
}

/// Run `f` — a control call's dispatch — with `choice` as the outcome
/// every chord or length edit inside it applies to an affected vocal
/// lane (`None`: the remembered choice, else keep). Returns `f`'s result
/// and what happened to each lane, for the reply's
/// `vocal_lanes_affected`.
pub(crate) fn with_requested_choice<T>(
    r: &mut crate::Resonance,
    choice: Option<VocalMelodyChoice>,
    f: impl FnOnce(&mut crate::Resonance) -> T,
) -> (T, Vec<AffectedVocalLane>) {
    r.compose.vocal_melody.affected.clear();
    let previous = std::mem::replace(&mut r.compose.vocal_melody.requested, choice);
    let out = f(r);
    r.compose.vocal_melody.requested = previous;
    let affected = std::mem::take(&mut r.compose.vocal_melody.affected);
    (out, affected)
}
