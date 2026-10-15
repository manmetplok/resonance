//! Building a [`VocalRollCanvas`] from app state: which section definition
//! and vocal lane the opened clip belongs to, and everything the canvas
//! borrows from them.

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use crate::compose::SectionDefinitionState;
use crate::state::MidiClipState;

use super::VocalRollCanvas;

/// Build a `VocalRollCanvas` from the `Compose` view's already-available
/// state. Returns `None` when the editing clip's track isn't a vocal
/// track or its section/definition can't be located.
pub fn build_canvas<'a>(
    app: &'a crate::Resonance,
    clip: &'a MidiClipState,
) -> Option<VocalRollCanvas<'a>> {
    let editor_state = app.ui.interaction.editing_midi_clip.as_ref()?;
    let definition = find_definition_for_clip(app, clip)?;
    let params = find_vocal_params(definition, clip.track_id)?;
    let voice_label = params.voice.as_str();
    // The section's meter, not the one under the playhead (VIEW-13).
    let meter = crate::update::compose::section_meter(app, definition.id);
    // Side-table lookup — falls back to an empty slice when the clip
    // has never had lyrics installed (e.g. a vocal clip created
    // through the engine without going through the generator). Callers
    // already pad to `clip.notes.len()` when they install, so when
    // present the slice length always matches the note count.
    static EMPTY_LYRICS: Vec<String> = Vec::new();
    let lyrics = app
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip.id)
        .map(|v| v.as_slice())
        .unwrap_or(EMPTY_LYRICS.as_slice());
    Some(VocalRollCanvas {
        keys_blocked: app.canvas_keys_blocked(),
        keymap: &app.ui.keymap,
        clip,
        track_id: editor_state.track_id,
        params,
        bank: app.voicebanks.profile(&params.voicebank),
        chords: &definition.chords,
        chord_spelling: definition.chord_spelling(),
        section_beats: definition.length_bars * meter.numerator as u32,
        scroll_y: editor_state.scroll_y,
        zoom_x: editor_state.zoom_x,
        zoom_y: editor_state.zoom_y,
        snap_ticks: editor_state.snap_ticks,
        // The vocal roll still works one note at a time; take the single
        // representative from the piano-roll multi-selection set.
        selected_note: editor_state.primary_selected(),
        time_sig_num: meter.numerator,
        bpm: meter.bpm,
        voice_label,
        lyrics,
    })
}

/// The vocal lane `(definition, track)` the opened clip belongs to — the
/// key of its render state (the toolbar's stale / error badge).
pub fn lane_for_clip(
    app: &crate::Resonance,
    clip: &MidiClipState,
) -> Option<(u64, TrackId)> {
    let definition = find_definition_for_clip(app, clip)?;
    find_vocal_params(definition, clip.track_id)?;
    Some((definition.id, clip.track_id))
}

/// Walk the compose state to find which section definition owns the
/// derived MIDI clip the user just opened. Falls back to scanning every
/// definition's lane generators for one keyed to the clip's track id.
fn find_definition_for_clip<'a>(
    app: &'a crate::Resonance,
    clip: &MidiClipState,
) -> Option<&'a SectionDefinitionState> {
    for ((def_id, _placement_id, track_id), cid) in app.compose.derived_clips.iter() {
        if *cid == clip.id && *track_id == clip.track_id {
            return app.compose.definitions.iter().find(|d| d.id == *def_id);
        }
    }
    // Fall back: any definition that has a vocal generator for this
    // track — this covers projects where the derived-clip map hasn't
    // been rebuilt yet (e.g. immediately after load).
    app.compose
        .definitions
        .iter()
        .find(|d| find_vocal_params(d, clip.track_id).is_some())
}

fn find_vocal_params(
    def: &SectionDefinitionState,
    track_id: TrackId,
) -> Option<&VocalParams> {
    use crate::compose::LaneGeneratorKind;
    def.lane_generators.get(&track_id).and_then(|cfg| match &cfg.kind {
        LaneGeneratorKind::Vocal(p) => Some(p),
        _ => None,
    })
}
