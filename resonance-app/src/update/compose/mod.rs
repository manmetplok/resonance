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

use crate::compose::ComposeMessage;

mod chord;
mod chord_inspector;
mod expand;
mod lane_inspector;
mod regenerate;
mod section;

pub fn handle(r: &mut crate::Resonance, msg: ComposeMessage) {
    let time_sig_num = r.transport.time_sig_num;

    match msg {
        ComposeMessage::Drumroll(m) => crate::update::drumroll::handle(r, m),

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
        }

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
        } => lane_inspector::handle(r, definition_id, track_id, msg),

        // Track role (arrangement metadata)
        ComposeMessage::SetTrackRole { track_id, role } => {
            if let Some(track) = r.registry.tracks.iter_mut().find(|t| t.id == track_id) {
                track.role = role;
            }
        }
    }
}
