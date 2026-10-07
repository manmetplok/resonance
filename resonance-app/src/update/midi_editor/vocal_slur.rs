//! The vocal roll's `s` key (vocal-mcp-control-and-quality.md §6): a
//! hold belongs in the lyric draft, so `s` writes or removes a `_` there
//! instead of flipping a per-note marker. The clip's per-note slur
//! entries (`clip_lyrics` `"+"`) are left to the melody generator's own
//! holds, which a regenerate replaces; a user hold rides in the lyrics
//! and survives `vocal.generate` and chord-driven regenerates.
//!
//! On a note of a vocal lane's clip:
//!
//! * a **sung** note becomes a user hold of the syllable before it — a
//!   `_` after that syllable in the draft (a no-op on a line's first
//!   note, which has nothing in its line to hold);
//! * a **generator hold** becomes a user hold: its per-note marker goes
//!   and the draft gains the `_`;
//! * a **user hold** loses one `_` of its syllable.
//!
//! A hold run always reads draft holds first, then generator holds (the
//! note walk gives a syllable's `_` holds to the first slur notes after
//! it), so making note `i` a user hold makes every hold between the
//! syllable and `i` a user hold too.

use resonance_audio::types::{ClipId, TrackId};
use resonance_music_theory::g2p::{self, HoldOrigin};

use crate::compose::LaneGeneratorKind;
use crate::Resonance;

/// The `(section definition, track)` vocal lane whose derived clip
/// `clip_id` is, the way the vocal roll finds it: the derived-clip map,
/// then a clip on the track starting at one of the lane's placements.
pub(super) fn lane_for_clip(r: &Resonance, clip_id: ClipId) -> Option<(u64, TrackId)> {
    let is_vocal_lane = |definition_id: u64, track_id: TrackId| {
        r.compose
            .find_definition(definition_id)
            .and_then(|d| d.lane_generators.get(&track_id))
            .is_some_and(|c| matches!(c.kind, LaneGeneratorKind::Vocal(_)))
    };
    let mapped = r
        .compose
        .derived_clips
        .iter()
        .find(|(_, id)| **id == clip_id)
        .map(|((def, _, track), _)| (*def, *track))
        .filter(|&(def, track)| is_vocal_lane(def, track));
    if mapped.is_some() {
        return mapped;
    }
    let clip = r.midi_clips.iter().find(|c| c.id == clip_id)?;
    r.compose
        .placements
        .iter()
        .find(|p| {
            r.tempo_map.bar_to_sample(p.start_bar) == clip.start_sample
                && is_vocal_lane(p.definition_id, clip.track_id)
        })
        .map(|p| (p.definition_id, clip.track_id))
}

/// Apply the `s` key to note `note_index` of `clip_id`, the clip of
/// vocal lane `(definition_id, track_id)`. Leaves everything untouched
/// when the press means nothing (see the module docs).
pub(super) fn toggle_draft_hold(
    r: &mut Resonance,
    (definition_id, track_id): (u64, TrackId),
    clip_id: ClipId,
    note_index: usize,
) {
    let Some(note_count) = r
        .midi_clips
        .iter()
        .find(|c| c.id == clip_id)
        .map(|c| c.notes.len())
    else {
        return;
    };
    let Some(mut draft) = lane_draft(r, definition_id, track_id) else {
        return;
    };
    let mut annotations = r
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip_id)
        .cloned()
        .unwrap_or_default();
    annotations.resize(note_count, String::new());

    let Some(edit) = plan(&draft, &annotations, note_index) else {
        return;
    };
    if !g2p::set_holds_after(&mut draft, edit.syllable, edit.holds) {
        return;
    }
    // The line now carries a hand edit: lock it so a lyric re-roll keeps
    // it, as `vocal.set_line` does.
    if let Some(line) = line_of_syllable(&draft, edit.syllable).and_then(|l| draft.get_mut(l)) {
        line.locked = true;
    }
    install_draft(r, definition_id, track_id, draft);
    clear_markers(r, (definition_id, track_id), clip_id, note_count, &edit.clear);
}

/// What one `s` press does: syllable `syllable` gets `holds` draft
/// holds, and the per-note markers at `clear` go.
struct HoldEdit {
    syllable: usize,
    holds: usize,
    clear: Vec<usize>,
}

fn plan(
    draft: &[resonance_music_theory::LyricLine],
    annotations: &[String],
    i: usize,
) -> Option<HoldEdit> {
    let note_count = annotations.len();
    if i >= note_count {
        return None;
    }
    let syllables = g2p::resolve_draft(draft);
    let assigned = g2p::assign_syllables_to_notes(&syllables, annotations, note_count);
    let origins = g2p::hold_origins(&syllables, annotations, note_count);
    // The sung note this hold run belongs to.
    let head = (0..i).rev().find(|&j| !assigned[j].is_slur)?;
    let held = assigned[head].syllable_index;
    let current = syllables.get(held)?.slurs_after;
    match origins[i] {
        Some(HoldOrigin::Draft) => {
            // The run's last draft hold is the note that stops holding;
            // a marker there would keep it held as a generator hold.
            Some(HoldEdit {
                syllable: held,
                holds: current.checked_sub(1)?,
                clear: vec![head + current],
            })
        }
        Some(HoldOrigin::Annotation) => Some(HoldEdit {
            syllable: held,
            holds: i - head,
            clear: (head + 1..=i).collect(),
        }),
        None => {
            // A line's first note has nothing in its line to hold.
            let own = assigned[i].syllable_index;
            if g2p::line_first_syllables(draft).contains(&own) {
                return None;
            }
            Some(HoldEdit {
                syllable: held,
                holds: i - head,
                clear: (head + 1..=i).collect(),
            })
        }
    }
}

/// The draft line that sings resolved syllable `syllable`.
fn line_of_syllable(draft: &[resonance_music_theory::LyricLine], syllable: usize) -> Option<usize> {
    let mut first = 0usize;
    for (idx, line) in draft.iter().enumerate() {
        first += g2p::resolve_draft(std::slice::from_ref(line)).len();
        if syllable < first {
            return Some(idx);
        }
    }
    None
}

fn lane_draft(
    r: &Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Option<Vec<resonance_music_theory::LyricLine>> {
    match &r.compose.find_definition(definition_id)?.lane_generators.get(&track_id)?.kind {
        LaneGeneratorKind::Vocal(p) => Some(p.draft.clone()),
        _ => None,
    }
}

/// Write the edited draft back into the lane and mirror it into the
/// lane's bulk lyric editor, when one is open.
fn install_draft(
    r: &mut Resonance,
    definition_id: u64,
    track_id: TrackId,
    draft: Vec<resonance_music_theory::LyricLine>,
) {
    let body = draft
        .iter()
        .map(|l| g2p::strip_syllable_marks(&l.text))
        .collect::<Vec<_>>()
        .join("\n");
    let Some(cfg) = r
        .compose
        .find_definition_mut(definition_id)
        .and_then(|d| d.lane_generators.get_mut(&track_id))
    else {
        return;
    };
    if let LaneGeneratorKind::Vocal(p) = &mut cfg.kind {
        p.draft = draft;
    }
    if let Some(content) = r.compose.vocal_bulk_lyrics.get_mut(&(definition_id, track_id)) {
        *content = iced::widget::text_editor::Content::with_text(&body);
    }
}

/// Clear the per-note markers at `indices` on the edited clip and on
/// every other placement's clip of the same lane (they carry the same
/// material).
fn clear_markers(
    r: &mut Resonance,
    (definition_id, track_id): (u64, TrackId),
    clip_id: ClipId,
    note_count: usize,
    indices: &[usize],
) {
    let mut clips: Vec<ClipId> = r
        .compose
        .derived_clips
        .iter()
        .filter(|((def, _, track), _)| *def == definition_id && *track == track_id)
        .map(|(_, id)| *id)
        .collect();
    if !clips.contains(&clip_id) {
        clips.push(clip_id);
    }
    for id in clips {
        let same_shape =
            r.midi_clips.iter().any(|c| c.id == id && c.notes.len() == note_count);
        if !same_shape {
            continue;
        }
        let Some(entry) = r.compose.vocal_audio.clip_lyrics.get_mut(&id) else {
            continue;
        };
        for &i in indices {
            if let Some(marker) = entry.get_mut(i) {
                marker.clear();
            }
        }
    }
}
