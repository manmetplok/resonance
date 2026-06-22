//! Top-level dispatcher for `ComposeMessage`. Each arm hands the work off
//! to the appropriate submodule:
//!
//! - [`section`] — section + placement CRUD and the new/edit dialog forms.
//! - [`chord`] — chord-state ops (add / edit / move / resize / delete).
//! - [`chord_inspector`] — chord-lane inspector messages (Markov knobs +
//!   shared motif knobs).
//! - [`lane_inspector`] — per-track lane inspector messages (generator
//!   choice, Bass/Melody/Pad/Drum params, lane Regenerate).
//! - [`regenerate`] — derive notes for one lane and the cascade helpers
//!   that fan chord changes / motif-seed bumps to every dependent lane.
//! - [`expand`] — expanded piano-roll viewport (open track, scroll, zoom).

use iced::Task;

use crate::compose::ComposeMessage;
use crate::message::Message;

mod chord;
mod chord_inspector;
pub(crate) mod drum_groups;
mod expand;
mod lane_inspector;
pub(crate) mod regenerate;
mod section;
mod vocal_lyrics;
mod vocal_render;

pub fn handle(r: &mut crate::Resonance, msg: ComposeMessage) -> Task<Message> {
    let time_sig_num = r.transport.time_sig_num;

    match msg {
        ComposeMessage::DrumGroups(m) => return drum_groups::handle(r, m),

        ComposeMessage::CreateMidiClipInSection {
            track_id,
            start_sample,
            length_bars,
        } => section::handle_create_midi_clip(r, track_id, start_sample, length_bars, time_sig_num),

        // Create-section dialog
        ComposeMessage::OpenCreateSectionDialog => section::handle_open_create_dialog(r),
        ComposeMessage::CancelCreateSectionDialog => section::handle_cancel_create_dialog(r),
        ComposeMessage::SetNewSectionName(name) => section::handle_set_new_name(r, name),
        ComposeMessage::SetNewSectionLength(input) => section::handle_set_new_length(r, input),
        ComposeMessage::ConfirmCreateSection => section::handle_confirm_create(r),

        // Edit-section dialog
        ComposeMessage::OpenEditSectionDialog { definition_id } => {
            section::handle_open_edit_dialog(r, definition_id)
        }
        ComposeMessage::CancelEditSectionDialog => section::handle_cancel_edit_dialog(r),
        ComposeMessage::SetEditSectionName(name) => section::handle_set_edit_name(r, name),
        ComposeMessage::SetEditSectionLength(input) => section::handle_set_edit_length(r, input),
        ComposeMessage::ConfirmEditSection => section::handle_confirm_edit(r),
        ComposeMessage::CycleSectionColor { definition_id } => {
            section::handle_cycle_color(r, definition_id)
        }

        // Section CRUD
        ComposeMessage::CreateSection {
            name,
            length_bars,
            color,
        } => section::handle_create(r, name, length_bars, color),
        ComposeMessage::RenameSection {
            definition_id,
            name,
        } => section::handle_rename(r, definition_id, name),
        ComposeMessage::ResizeSection {
            definition_id,
            length_bars,
        } => section::handle_resize(r, definition_id, length_bars, time_sig_num),
        ComposeMessage::SetSectionScale {
            definition_id,
            scale,
        } => section::handle_set_scale(r, definition_id, scale),
        ComposeMessage::DeleteSectionDefinition { definition_id } => {
            section::handle_delete_definition(r, definition_id)
        }

        // Placement CRUD
        ComposeMessage::PlaceSection {
            definition_id,
            start_bar,
        } => section::handle_place(r, definition_id, start_bar),
        ComposeMessage::DeleteSectionPlacement { placement_id } => {
            section::handle_delete_placement(r, placement_id)
        }
        ComposeMessage::SelectSectionPlacement { placement_id } => {
            section::handle_select_placement(r, placement_id)
        }

        // Chord lane selection
        ComposeMessage::SelectChord { chord_id } => {
            r.compose.selected_chord_id = Some(chord_id);
            r.compose.selected_lane = crate::compose::SelectedLane::Chords;
        }
        ComposeMessage::ClearChordSelection => {
            r.compose.selected_chord_id = None;
        }
        ComposeMessage::SelectLane(lane) => {
            r.compose.selected_lane = lane;
            ensure_vocal_bulk_lyrics_for_selection(r);
        }

        // Collapse toggles — runtime UI state, never persisted.
        ComposeMessage::ToggleRailPanel(key) => {
            let set = &mut r.compose.collapsed_rail_panels;
            if !set.remove(&key) {
                set.insert(key);
            }
        }
        ComposeMessage::ToggleWorkspaceGroup(group) => match group {
            crate::compose::WorkspaceGroup::Section => {
                r.compose.section_lanes_collapsed = !r.compose.section_lanes_collapsed;
            }
            crate::compose::WorkspaceGroup::Tracks => {
                r.compose.track_lanes_collapsed = !r.compose.track_lanes_collapsed;
            }
        },

        // Expanded piano-roll viewport
        ComposeMessage::ExpandTrack { track_id } => expand::handle_expand(r, track_id),
        ComposeMessage::CollapseTrack => expand::handle_collapse(r),
        ComposeMessage::ExpandedScrollX(delta) => expand::handle_scroll_x(r, delta),
        ComposeMessage::ExpandedScrollY(delta) => expand::handle_scroll_y(r, delta),
        ComposeMessage::ExpandedZoomY(delta) => expand::handle_zoom_y(r, delta),

        // Chord ops
        ComposeMessage::AddChord {
            definition_id,
            start_beat,
            duration_beats,
            root,
            quality,
        } => chord::handle_add(
            r,
            definition_id,
            start_beat,
            duration_beats,
            root,
            quality,
            time_sig_num,
        ),
        ComposeMessage::EditChord {
            definition_id,
            chord_id,
            chord,
        } => chord::handle_edit(r, definition_id, chord_id, chord),
        ComposeMessage::MoveChord {
            definition_id,
            chord_id,
            start_beat,
        } => chord::handle_move(r, definition_id, chord_id, start_beat, time_sig_num),
        ComposeMessage::ResizeChord {
            definition_id,
            chord_id,
            duration_beats,
        } => chord::handle_resize(r, definition_id, chord_id, duration_beats, time_sig_num),
        ComposeMessage::DeleteChord {
            definition_id,
            chord_id,
        } => chord::handle_delete(r, definition_id, chord_id),

        // Inspectors
        ComposeMessage::ChordInspector { definition_id, msg } => {
            chord_inspector::handle(r, definition_id, msg)
        }
        ComposeMessage::LaneInspector {
            definition_id,
            track_id,
            msg,
        } => return lane_inspector::handle(r, definition_id, track_id, msg),

        // Vocal audio render completion (dispatched from the background
        // SVS task that `lane_inspector::handle` queued).
        ComposeMessage::VocalAudioReady(data) => {
            vocal_render::handle_vocal_audio_ready(r, *data);
        }
        ComposeMessage::VocalAudioFailed { error } => {
            r.compose.last_error = Some(error);
        }
    }
    Task::none()
}

/// Ensure the bulk-lyrics text-editor buffer exists for the currently
/// selected vocal lane. The view layer hands a `&Content` to the iced
/// widget, so the entry must be present before the first paint to avoid
/// rendering a dead fallback. Seeded from the lane's current `params.draft`
/// so the user sees their existing lyrics in the editor.
pub(crate) fn ensure_vocal_bulk_lyrics_for_selection(r: &mut crate::Resonance) {
    use crate::compose::{LaneGeneratorKind, SelectedLane};
    let track_id = match r.compose.selected_lane {
        SelectedLane::Instrument(id) => id,
        _ => return,
    };
    let Some(placement_id) = r.compose.selected_placement_id else {
        return;
    };
    let Some(placement) = r
        .compose
        .placements
        .iter()
        .find(|p| p.id == placement_id)
        .cloned()
    else {
        return;
    };
    let definition_id = placement.definition_id;
    let Some(def) = r.compose.find_definition(definition_id) else {
        return;
    };
    let Some(cfg) = def.lane_generators.get(&track_id) else {
        return;
    };
    let LaneGeneratorKind::Vocal(params) = &cfg.kind else {
        return;
    };
    let key = (definition_id, track_id);
    if r.compose.vocal_bulk_lyrics.contains_key(&key) {
        return;
    }
    let body = params
        .draft
        .iter()
        .map(|l| l.text.replace('\u{00B7}', "").replace("  ", " "))
        .collect::<Vec<_>>()
        .join("\n");
    r.compose
        .vocal_bulk_lyrics
        .insert(key, iced::widget::text_editor::Content::with_text(&body));
}
