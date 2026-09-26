//! Section + placement CRUD plus the new/edit-section dialog form
//! handlers. The dialog confirmations re-enter the parent dispatcher so
//! they share validation with the direct CreateSection / RenameSection /
//! ResizeSection paths.

use std::collections::HashMap;

use resonance_audio::types::{AudioCommand, TICKS_PER_QUARTER_NOTE};

use iced::Task;

use super::handle as dispatch;
use crate::message::Message;
use crate::compose::invariants::{
    chord_fits_in_section, placement_overlaps, section_span_in_bounds, MAX_SECTION_BARS,
};
use crate::util::seed_from_id;
use crate::compose::{
    ComposeMessage, ComposeState, EditSectionForm, NewSectionForm, SectionDefinitionState,
    SectionPlacementState,
};

/// Stock section names offered in the order Intro → Verse → Chorus → Bridge
/// → Outro. Whichever is not yet present in the project is used as the
/// initial value of the new-section form.
const STOCK_SECTION_NAMES: &[&str] = &["Intro", "Verse", "Chorus", "Bridge", "Outro"];

/// Section colors cycled through by the auto-rotating palette. Indexed by
/// `definitions.len()` modulo the palette size.
const SECTION_PALETTE: &[[u8; 3]] = &[
    [0x5b, 0x8d, 0xef], // blue
    [0xef, 0x8d, 0x5b], // orange
    [0x8d, 0xef, 0x5b], // green
    [0xef, 0x5b, 0x8d], // pink
    [0xbd, 0x8d, 0xef], // purple
    [0xef, 0xef, 0x5b], // yellow
];

fn default_section_name(state: &ComposeState) -> String {
    let existing: std::collections::HashSet<&str> =
        state.definitions.iter().map(|d| d.name.as_str()).collect();
    for name in STOCK_SECTION_NAMES {
        if !existing.contains(*name) {
            return (*name).to_string();
        }
    }
    format!("Section {}", state.definitions.len() + 1)
}

/// Next color from the auto-rotating palette; also used by the control
/// endpoint's `section.create` (which has no color in its params).
pub(crate) fn next_default_color(state: &ComposeState) -> [u8; 3] {
    SECTION_PALETTE[state.definitions.len() % SECTION_PALETTE.len()]
}

/// Lowest start_bar at which a section of `length_bars` would not overlap
/// any existing placement, or `None` when no such bar keeps the section
/// inside [`MAX_SECTION_BARS`]. Jumps past each overlapping placement, so
/// the search is bounded by the placement count (code review VIEW-17: it
/// used to give up at bar 10_001 and return that bar even if it overlapped).
pub(super) fn first_free_bar(state: &ComposeState, length_bars: u32) -> Option<u32> {
    let mut candidate = 0u32;
    for _ in 0..=state.placements.len() {
        if !section_span_in_bounds(candidate, length_bars) {
            return None;
        }
        let blocker = state.placements.iter().find_map(|p| {
            let len = state.find_definition(p.definition_id)?.length_bars;
            let end = u64::from(p.start_bar) + u64::from(len);
            let overlaps = u64::from(candidate) < end
                && u64::from(p.start_bar) < u64::from(candidate) + u64::from(length_bars);
            overlaps.then_some(end)
        });
        match blocker {
            None => return Some(candidate),
            Some(end) => candidate = u32::try_from(end).ok()?,
        }
    }
    None
}

/// The "too long" error shared by the dialogs and the CRUD handlers.
fn too_long_error() -> String {
    format!("Section length must be at most {MAX_SECTION_BARS} bars")
}

pub(super) fn handle_create_midi_clip(
    r: &mut crate::Resonance,
    track_id: resonance_audio::types::TrackId,
    start_sample: u64,
    length_bars: u32,
) {
    let (bar, _) = r.tempo_map.sample_to_bar(start_sample, r.sample_rate);
    let time_sig_num = super::meter_at_bar(r, bar).numerator;
    let duration_ticks = length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;
    let _ = r.engine.send(AudioCommand::CreateMidiClip {
        track_id,
        start_sample,
        duration_ticks,
        name: "MIDI Clip".to_string(),
    });
}

// ---------------------------------------------------------------------------
// Create-section dialog
// ---------------------------------------------------------------------------

pub(super) fn handle_open_create_dialog(r: &mut crate::Resonance) {
    r.compose.edit_section_form = None;
    r.compose.new_section_form = Some(NewSectionForm {
        name: default_section_name(&r.compose),
        length_input: "8".to_string(),
        color: next_default_color(&r.compose),
    });
    r.compose.last_error = None;
}

pub(super) fn handle_cancel_create_dialog(r: &mut crate::Resonance) {
    r.compose.new_section_form = None;
    r.compose.last_error = None;
}

pub(super) fn handle_set_new_name(r: &mut crate::Resonance, name: String) {
    if let Some(form) = r.compose.new_section_form.as_mut() {
        form.name = name;
    }
}

pub(super) fn handle_set_new_length(r: &mut crate::Resonance, input: String) {
    if let Some(form) = r.compose.new_section_form.as_mut() {
        form.length_input = input.chars().filter(|c| c.is_ascii_digit()).collect();
    }
}

pub(super) fn handle_confirm_create(r: &mut crate::Resonance) {
    let Some(form) = r.compose.new_section_form.clone() else {
        return;
    };
    let name = form.name.trim().to_string();
    if name.is_empty() {
        r.compose.last_error = Some("Section name cannot be empty".into());
        return;
    }
    let length_bars: u32 = match form.length_input.parse() {
        Ok(n) if n > 0 => n,
        _ => {
            r.compose.last_error =
                Some("Section length must be a positive whole number of bars".into());
            return;
        }
    };
    if length_bars > MAX_SECTION_BARS {
        r.compose.last_error = Some(too_long_error());
        return;
    }
    r.compose.new_section_form = None;
    // These re-entrant dispatches never produce a real task — section
    // CRUD is fully synchronous — so dropping the returned Task is safe.
    let _ = dispatch(
        r,
        ComposeMessage::CreateSection {
            name,
            length_bars,
            color: form.color,
            place: true,
        },
    );
}

// ---------------------------------------------------------------------------
// Edit-section dialog
// ---------------------------------------------------------------------------

pub(super) fn handle_open_edit_dialog(r: &mut crate::Resonance, definition_id: u64) {
    let snapshot = match r.compose.find_definition(definition_id) {
        Some(def) => (def.name.clone(), def.length_bars),
        None => return,
    };
    r.compose.new_section_form = None;
    r.compose.edit_section_form = Some(EditSectionForm {
        definition_id,
        name: snapshot.0,
        length_input: snapshot.1.to_string(),
    });
    r.compose.last_error = None;
}

pub(super) fn handle_cancel_edit_dialog(r: &mut crate::Resonance) {
    r.compose.edit_section_form = None;
    r.compose.last_error = None;
}

pub(super) fn handle_set_edit_name(r: &mut crate::Resonance, name: String) {
    if let Some(form) = r.compose.edit_section_form.as_mut() {
        form.name = name;
    }
}

pub(super) fn handle_set_edit_length(r: &mut crate::Resonance, input: String) {
    if let Some(form) = r.compose.edit_section_form.as_mut() {
        form.length_input = input.chars().filter(|c| c.is_ascii_digit()).collect();
    }
}

pub(super) fn handle_confirm_edit(r: &mut crate::Resonance) -> Task<Message> {
    let Some(form) = r.compose.edit_section_form.clone() else {
        return Task::none();
    };
    let name = form.name.trim().to_string();
    if name.is_empty() {
        r.compose.last_error = Some("Section name cannot be empty".into());
        return Task::none();
    }
    let length_bars: u32 = match form.length_input.parse() {
        Ok(n) if n > 0 => n,
        _ => {
            r.compose.last_error =
                Some("Section length must be a positive whole number of bars".into());
            return Task::none();
        }
    };
    let _ = dispatch(
        r,
        ComposeMessage::RenameSection {
            definition_id: form.definition_id,
            name,
        },
    );
    // Resizing re-derives the section's lanes; a vocal lane's re-render
    // comes back as a task.
    let task = dispatch(
        r,
        ComposeMessage::ResizeSection {
            definition_id: form.definition_id,
            length_bars,
        },
    );
    if r.compose.last_error.is_none() {
        r.compose.edit_section_form = None;
    }
    task
}

pub(super) fn handle_cycle_color(r: &mut crate::Resonance, definition_id: u64) {
    let current = r
        .compose
        .find_definition(definition_id)
        .map(|d| d.color)
        .unwrap_or([0, 0, 0]);
    let next_index = SECTION_PALETTE
        .iter()
        .position(|c| *c == current)
        .map(|i| (i + 1) % SECTION_PALETTE.len())
        .unwrap_or(0);
    let next_color = SECTION_PALETTE[next_index];
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.color = next_color;
        r.compose.last_error = None;
    }
}

// ---------------------------------------------------------------------------
// Section CRUD
// ---------------------------------------------------------------------------

pub(super) fn handle_create(
    r: &mut crate::Resonance,
    name: String,
    length_bars: u32,
    color: [u8; 3],
    place: bool,
) {
    if length_bars == 0 {
        r.compose.last_error = Some("Section length must be at least 1 bar".into());
        return;
    }
    if length_bars > MAX_SECTION_BARS {
        r.compose.last_error = Some(too_long_error());
        return;
    }
    let start_bar = if place {
        match first_free_bar(&r.compose, length_bars) {
            Some(bar) => Some(bar),
            None => {
                r.compose.last_error =
                    Some(format!("No room to place the section within {MAX_SECTION_BARS} bars"));
                return;
            }
        }
    } else {
        None
    };
    let id = r.compose.fresh_id();
    r.compose.definitions.push(SectionDefinitionState {
        id,
        name,
        color,
        length_bars,
        chords: Vec::new(),
        scale: None,
        progression_seed: seed_from_id(id),
        generate_params: crate::compose::GenerateParams::default(),
        generator_spec: None,
        generator_seed: id.wrapping_mul(0x517CC1B727220A95),
        generated_material: None,
        lane_generators: HashMap::new(),
        beats_per_chord: 4,
        seventh_chords: false,
        motif_source: resonance_music_theory::MotifSource::Generated(
            resonance_music_theory::MotifParams {
                seed: id.wrapping_mul(0x6C62272E07BB0142),
                ..resonance_music_theory::MotifParams::default()
            },
        ),
        arrangement: Vec::new(),
    });
    if let Some(start_bar) = start_bar {
        let placement_id = r.compose.fresh_id();
        r.compose.placements.push(SectionPlacementState {
            id: placement_id,
            definition_id: id,
            start_bar,
        });
        r.compose.placements.sort_by_key(|p| p.start_bar);
        r.compose.selected_placement_id = Some(placement_id);
    }
    r.compose.last_error = None;
}

pub(super) fn handle_rename(r: &mut crate::Resonance, definition_id: u64, name: String) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.name = name;
        r.compose.last_error = None;
    }
}

pub(super) fn handle_resize(
    r: &mut crate::Resonance,
    definition_id: u64,
    length_bars: u32,
    time_sig_num: u8,
) -> Task<Message> {
    if length_bars == 0 {
        r.compose.last_error = Some("Section length must be at least 1 bar".into());
        return Task::none();
    }
    let out_of_bounds = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .any(|p| !section_span_in_bounds(p.start_bar, length_bars));
    if length_bars > MAX_SECTION_BARS || out_of_bounds {
        r.compose.last_error = Some(too_long_error());
        return Task::none();
    }
    let old_length = match r.compose.find_definition(definition_id) {
        Some(d) => d.length_bars,
        None => return Task::none(),
    };
    if length_bars == old_length {
        // Nothing to re-derive (the edit dialog resizes on every confirm,
        // including a pure rename).
        r.compose.last_error = None;
        return Task::none();
    }
    if length_bars > old_length {
        let snapshot = r.compose.placements.clone();
        let definitions = r.compose.definitions.clone();
        for p in snapshot.iter().filter(|p| p.definition_id == definition_id) {
            let others: Vec<SectionPlacementState> =
                snapshot.iter().filter(|q| q.id != p.id).cloned().collect();
            if placement_overlaps(&others, &definitions, p.start_bar, length_bars, None) {
                r.compose.last_error =
                    Some("Cannot grow section: a placement would overlap a neighbour".into());
                return Task::none();
            }
        }
    }
    let chords_fit = r
        .compose
        .find_definition(definition_id)
        .map(|d| {
            d.chords.iter().all(|c| {
                chord_fits_in_section(c.start_beat, c.duration_beats, length_bars, time_sig_num)
            })
        })
        .unwrap_or(true);
    if !chords_fit {
        r.compose.last_error =
            Some("Cannot shrink section: chords would fall outside the new length".into());
        return Task::none();
    }
    // A drum arrangement that filled the old length (e.g. the single
    // `Bars(old_len)` entry `set_primary_pattern` writes) keeps filling
    // the section; one that deliberately left a trailing gap keeps it.
    let arrangement_filled = r.compose.find_definition(definition_id).is_some_and(|def| {
        !def.arrangement.is_empty()
            && r
                .compose
                .resolve_arrangement_for(def)
                .spans
                .last()
                .is_some_and(|s| s.bar_end >= old_length)
    });
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.length_bars = length_bars;
    }
    if length_bars > old_length && arrangement_filled {
        super::drum_groups::fill_to_end(r, definition_id);
    }
    let task = rederive_section_clips(r, definition_id);
    r.compose.last_error = None;
    task
}

/// Revalidate every section's chords after the global signature changed
/// (code review FU-V2b). A section is `length_bars` bars in the meter at
/// its start, so a shorter meter can leave chords past its end — which
/// every chord edit refuses (`chord_fits_in_section`). A chord that
/// straddles the new end is trimmed to it, one that starts at or past it
/// is dropped, and the section's derived clips are rebuilt so the lanes
/// stop playing what is gone. A longer meter touches nothing. Runs inside
/// the signature edit's own dispatch, so it rides that undo entry.
pub(crate) fn revalidate_chords_after_meter_change(r: &mut crate::Resonance) -> Task<Message> {
    let changed = trim_chords_to_sections(r);
    let tasks: Vec<_> = changed
        .into_iter()
        .map(|id| rederive_section_clips(r, id))
        .collect();
    Task::batch(tasks)
}

/// Trim every section's chords to its end in the meter at its start — a
/// straddling chord is shortened, one starting at or past the end
/// dropped — and return the sections that changed. The load path calls
/// this alone (code review FU-V4b): a file's derived clips are what was
/// saved, so nothing is re-derived there.
pub(crate) fn trim_chords_to_sections(r: &mut crate::Resonance) -> Vec<u64> {
    let ids: Vec<u64> = r.compose.definitions.iter().map(|d| d.id).collect();
    let mut changed = Vec::new();
    for id in ids {
        let numerator = super::section_meter(r, id).numerator;
        let Some(def) = r.compose.find_definition_mut(id) else {
            continue;
        };
        let end = u64::from(def.length_bars) * u64::from(numerator);
        let before = def.chords.len();
        let mut trimmed = false;
        def.chords.retain(|c| u64::from(c.start_beat) < end);
        for c in def.chords.iter_mut() {
            let chord_end = u64::from(c.start_beat) + u64::from(c.duration_beats);
            if chord_end > end {
                c.duration_beats = (end - u64::from(c.start_beat)) as u32;
                trimmed = true;
            }
        }
        if trimmed || def.chords.len() != before {
            changed.push(id);
        }
    }
    changed
}

/// Re-derive every generated clip of a section after its length changed
/// (code review VIEW-05): clip durations are fixed when a clip is built,
/// so without this a shrink left full-length clips overlapping the next
/// section and a grow left the added bars silent. Only lanes that already
/// have clips are rebuilt — a resize never generates a lane the user did
/// not ask for. Returns the vocal lanes' re-render tasks.
fn rederive_section_clips(r: &mut crate::Resonance, definition_id: u64) -> Task<Message> {
    let mut tracks: Vec<resonance_audio::types::TrackId> = r
        .compose
        .derived_clips
        .keys()
        .filter(|(d, _, _)| *d == definition_id)
        .map(|(_, _, t)| *t)
        .collect();
    tracks.sort_unstable();
    tracks.dedup();
    let has_drums = tracks.iter().any(|t| {
        r.registry
            .tracks
            .iter()
            .any(|tr| tr.id == *t && tr.instrument_type == crate::state::InstrumentType::Drum)
    });

    if has_drums {
        super::drum_groups::materialize_drum_clips_for(
            r,
            Some(definition_id),
            super::ClipVisibility::Immediate,
        );
    }
    // Drum tracks have no chord-lane generator, so `regenerate_lane` is a
    // no-op for them; every other lane is rebuilt at the new length.
    let tasks: Vec<Task<Message>> = tracks
        .into_iter()
        .map(|t| super::regenerate::regenerate_lane(r, definition_id, t))
        .collect();
    if has_drums {
        // Materializing replaced any Motif-mode drum voices; lay them back.
        super::regenerate::propagate_motif_change(r, definition_id);
    }
    Task::batch(tasks)
}

pub(super) fn handle_set_scale(
    r: &mut crate::Resonance,
    definition_id: u64,
    scale: Option<resonance_music_theory::Scale>,
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.scale = scale;
        r.compose.last_error = None;
    }
}

/// Delete a definition together with every placement referencing it, in
/// one undoable step (control endpoint `section.delete`, ba todo #1153).
/// The GUI path ([`handle_delete_definition`]) instead refuses while
/// placements exist.
pub(super) fn handle_delete_with_placements(r: &mut crate::Resonance, definition_id: u64) {
    if r.compose.find_definition(definition_id).is_none() {
        return;
    }
    let doomed: Vec<u64> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| p.id)
        .collect();
    for placement_id in doomed {
        purge_placement_outputs(r, placement_id);
    }
    r.compose.placements.retain(|p| p.definition_id != definition_id);
    if r
        .compose
        .selected_placement_id
        .is_some_and(|id| r.compose.find_placement(id).is_none())
    {
        r.compose.selected_placement_id = r.compose.placements.first().map(|p| p.id);
    }
    if let Some(chord_id) = r.compose.selected_chord_id {
        let selected_here = r
            .compose
            .find_definition(definition_id)
            .is_some_and(|d| d.chords.iter().any(|c| c.id == chord_id));
        if selected_here {
            r.compose.selected_chord_id = None;
        }
    }
    r.compose.definitions.retain(|d| d.id != definition_id);
    purge_definition_side_tables(r, definition_id);
    r.compose.last_error = None;
}

/// Tear down everything generated for one placement: its derived MIDI
/// clips and its installed vocal audio clips, from the engine, the
/// project's clip lists and the compose maps (code review VIEW-04).
/// Called by every path that deletes a placement so the lane stops
/// playing a section that no longer exists. The vocal WAV is left on
/// disk: it is shared by the definition's other placements, and an undo
/// of this delete restores a clip that still points at it.
pub(crate) fn purge_placement_outputs(r: &mut crate::Resonance, placement_id: u64) {
    let midi: Vec<_> = r
        .compose
        .derived_clips
        .iter()
        .filter(|((_, p, _), _)| *p == placement_id)
        .map(|(_, id)| *id)
        .collect();
    r.compose
        .derived_clips
        .retain(|(_, p, _), _| *p != placement_id);
    for clip_id in midi {
        let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id });
        r.midi_clips.retain(|c| c.id != clip_id);
        r.compose.vocal_audio.clip_lyrics.remove(&clip_id);
    }

    let audio: Vec<_> = r
        .compose
        .vocal_audio
        .clips
        .iter()
        .filter(|((_, p, _), _)| *p == placement_id)
        .map(|(_, (id, _))| *id)
        .collect();
    r.compose
        .vocal_audio
        .clips
        .retain(|(_, p, _), _| *p != placement_id);
    let removed_audio = !audio.is_empty();
    for clip_id in audio {
        let _ = r.engine.send(AudioCommand::DeleteClip { clip_id });
        r.clips.retain(|c| c.id != clip_id);
    }
    if removed_audio {
        r.recompute_pool_usage(); // FU-V3c, as VIEW-30 did for other deletes
    }
}

/// Drop the per-`(definition, track)` runtime tables of a deleted
/// definition. The render epoch is bumped rather than removed so a vocal
/// render still in flight for it is discarded on completion instead of
/// installing audio for a section that is gone.
fn purge_definition_side_tables(r: &mut crate::Resonance, definition_id: u64) {
    let compose = &mut r.compose;
    for ((d, _), epoch) in compose.vocal_audio.render_epoch.iter_mut() {
        if *d == definition_id {
            *epoch += 1;
        }
    }
    compose.vocal_audio.render_cache.retain(|(d, _), _| *d != definition_id);
    compose.vocal_bulk_lyrics.retain(|(d, _), _| *d != definition_id);
    compose.expression_curves.retain(|(d, _), _| *d != definition_id);
}

pub(super) fn handle_delete_definition(r: &mut crate::Resonance, definition_id: u64) {
    let in_use = r
        .compose
        .placements
        .iter()
        .any(|p| p.definition_id == definition_id);
    if in_use {
        r.compose.last_error =
            Some("Cannot delete a section while placements still reference it".into());
        return;
    }
    r.compose.definitions.retain(|d| d.id != definition_id);
    purge_definition_side_tables(r, definition_id);
    r.compose.last_error = None;
}

// ---------------------------------------------------------------------------
// Placement CRUD + selection
// ---------------------------------------------------------------------------

pub(super) fn handle_place(r: &mut crate::Resonance, definition_id: u64, start_bar: u32) {
    let length_bars = match r.compose.find_definition(definition_id) {
        Some(d) => d.length_bars,
        None => return,
    };
    if !section_span_in_bounds(start_bar, length_bars) {
        r.compose.last_error =
            Some(format!("A placement must end by bar {MAX_SECTION_BARS}"));
        return;
    }
    if placement_overlaps(
        &r.compose.placements,
        &r.compose.definitions,
        start_bar,
        length_bars,
        None,
    ) {
        r.compose.last_error = Some("Placement would overlap an existing section".into());
        return;
    }
    let id = r.compose.fresh_id();
    r.compose.placements.push(SectionPlacementState {
        id,
        definition_id,
        start_bar,
    });
    r.compose.placements.sort_by_key(|p| p.start_bar);
    r.compose.selected_placement_id = Some(id);
    r.compose.last_error = None;
}

pub(super) fn handle_delete_placement(r: &mut crate::Resonance, placement_id: u64) {
    if r.compose.find_placement(placement_id).is_none() {
        return;
    }
    purge_placement_outputs(r, placement_id);
    r.compose.placements.retain(|p| p.id != placement_id);
    if r.compose.selected_placement_id == Some(placement_id) {
        r.compose.selected_placement_id = r.compose.placements.first().map(|p| p.id);
    }
    r.compose.last_error = None;
}

pub(super) fn handle_select_placement(r: &mut crate::Resonance, placement_id: u64) {
    if r.compose.find_placement(placement_id).is_some() {
        r.compose.selected_placement_id = Some(placement_id);
        r.compose.selected_chord_id = None;
    }
}
