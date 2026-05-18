//! Clip drag + trim handlers, for both audio clips and MIDI clips.
//! The two flavours share most of their logic — edge-threshold trims,
//! pixel-to-sample conversions, target-track hit-tests — so they live
//! side by side here.
use iced::Task;
use resonance_audio::types::*;

use crate::message::{ClipMessage, Message};
use crate::state::*;
use crate::Resonance;

/// Route a `ClipMessage` to the appropriate handler.
pub fn handle(r: &mut Resonance, m: ClipMessage) -> Task<Message> {
    match m {
        ClipMessage::DeleteClip(id) => {
            r.engine.send(AudioCommand::DeleteClip { clip_id: id });
            if r.interaction.selected_clip == Some(id) {
                r.interaction.selected_clip = None;
            }
        }
        ClipMessage::StartClipDrag {
            clip_id,
            grab_offset_x,
            start_x,
            start_y,
        } => {
            start_clip_drag(r, clip_id, grab_offset_x, start_x, start_y);
        }
        ClipMessage::UpdateClipDrag(x, y) => {
            update_clip_drag(r, x, y);
        }
        ClipMessage::EndClipDrag => {
            end_clip_drag(r);
        }
        ClipMessage::StartClipTrim {
            clip_id,
            edge,
            anchor_x,
        } => {
            start_clip_trim(r, clip_id, edge, anchor_x);
        }
        ClipMessage::UpdateClipTrim(x) => {
            update_clip_trim(r, x);
        }
        ClipMessage::EndClipTrim => {
            end_clip_trim(r);
        }
    }
    Task::none()
}

// -- Audio clip drag/trim ----------------------------------------------

pub fn start_clip_drag(
    r: &mut Resonance,
    clip_id: ClipId,
    grab_offset_x: f32,
    start_x: f32,
    start_y: f32,
) {
    if let Some(clip) = r.clips.iter().find(|c| c.id == clip_id) {
        r.interaction.selected_clip = Some(clip_id);
        r.interaction.selected_track = Some(clip.track_id);
        r.interaction.clip_drag = Some(ClipDragState {
            clip_id,
            grab_offset_x,
            original_track_id: clip.track_id,
            current_x: start_x,
            current_y: start_y,
        });
    }
}

pub fn update_clip_drag(r: &mut Resonance, x: f32, y: f32) {
    let original_track_id = r
        .interaction
        .clip_drag
        .as_ref()
        .map(|d| d.original_track_id);
    let Some(orig) = original_track_id else {
        return;
    };
    let target_track_id = r.track_id_at_arrange_y(y).unwrap_or(orig);
    let bpm = r.transport.bpm;
    let num = r.transport.time_sig_num;
    let sample_rate = r.sample_rate;
    let zoom = r.viewport.zoom;
    let scroll = r.viewport.scroll_offset;
    if let Some(ref mut drag) = r.interaction.clip_drag {
        drag.current_x = x;
        drag.current_y = y;
        let seconds = ((x - drag.grab_offset_x) + scroll) / zoom;
        let raw = if seconds < 0.0 {
            0u64
        } else {
            (seconds as f64 * sample_rate as f64) as u64
        };
        let new_start = crate::timeline::snap_sample_to_grid(raw, bpm, num, sample_rate, zoom);
        let clip_id = drag.clip_id;
        if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
            clip.start_sample = new_start;
            clip.track_id = target_track_id;
        }
    }
}

pub fn end_clip_drag(r: &mut Resonance) {
    if let Some(drag) = r.interaction.clip_drag.take() {
        if let Some(clip) = r.clips.iter().find(|c| c.id == drag.clip_id) {
            r.engine.send(AudioCommand::MoveClip {
                clip_id: drag.clip_id,
                new_start_sample: clip.start_sample,
                new_track_id: clip.track_id,
            });
        }
    }
}

pub fn start_clip_trim(r: &mut Resonance, clip_id: ClipId, edge: ClipEdge, anchor_x: f32) {
    if let Some(clip) = r.clips.iter().find(|c| c.id == clip_id) {
        r.interaction.selected_clip = Some(clip_id);
        r.interaction.selected_track = Some(clip.track_id);
        r.interaction.clip_trim = Some(ClipTrimState {
            clip_id,
            edge,
            original_start_sample: clip.start_sample,
            original_trim_start: clip.trim_start_frames,
            original_trim_end: clip.trim_end_frames,
            original_total_frames: clip.total_frames,
            anchor_x,
        });
    }
}

pub fn update_clip_trim(r: &mut Resonance, x: f32) {
    let Some(trim) = r.interaction.clip_trim.clone() else {
        return;
    };
    let delta_px = x - trim.anchor_x;
    let delta_seconds = delta_px as f64 / r.viewport.zoom as f64;
    let delta_samples_signed = (delta_seconds * r.sample_rate as f64) as i64;
    let min_duration_frames = (0.01 * r.sample_rate as f64) as u64;
    // Snap the edge's final sample position to the grid so trimmed
    // clip edges land on bar/beat lines instead of wherever the mouse
    // happens to be.
    let snap = |sample: u64| -> u64 {
        crate::timeline::snap_sample_to_grid(
            sample,
            r.transport.bpm,
            r.transport.time_sig_num,
            r.sample_rate,
            r.viewport.zoom,
        )
    };

    match trim.edge {
        ClipEdge::Left => {
            let original_edge = trim.original_start_sample;
            let raw_target = (original_edge as i64 + delta_samples_signed).max(0) as u64;
            let snapped_delta = snap(raw_target) as i64 - original_edge as i64;
            let max_trim = trim
                .original_total_frames
                .saturating_sub(trim.original_trim_end)
                .saturating_sub(min_duration_frames);
            let new_trim_start = if snapped_delta >= 0 {
                (trim.original_trim_start + snapped_delta as u64).min(max_trim)
            } else {
                trim.original_trim_start
                    .saturating_sub((-snapped_delta) as u64)
            };
            let actual_delta = new_trim_start as i64 - trim.original_trim_start as i64;
            let new_start = (trim.original_start_sample as i64 + actual_delta).max(0) as u64;
            let new_duration = trim
                .original_total_frames
                .saturating_sub(new_trim_start)
                .saturating_sub(trim.original_trim_end);
            if let Some(clip) = r.clips.iter_mut().find(|c| c.id == trim.clip_id) {
                clip.start_sample = new_start;
                clip.trim_start_frames = new_trim_start;
                clip.duration_samples = new_duration;
            }
        }
        ClipEdge::Right => {
            let original_duration = trim
                .original_total_frames
                .saturating_sub(trim.original_trim_start)
                .saturating_sub(trim.original_trim_end);
            let original_edge = trim.original_start_sample + original_duration;
            let raw_target = (original_edge as i64 + delta_samples_signed).max(0) as u64;
            let snapped_delta = snap(raw_target) as i64 - original_edge as i64;
            let max_trim = trim
                .original_total_frames
                .saturating_sub(trim.original_trim_start)
                .saturating_sub(min_duration_frames);
            let new_trim_end = if snapped_delta <= 0 {
                (trim.original_trim_end + (-snapped_delta) as u64).min(max_trim)
            } else {
                trim.original_trim_end.saturating_sub(snapped_delta as u64)
            };
            let new_duration = trim
                .original_total_frames
                .saturating_sub(trim.original_trim_start)
                .saturating_sub(new_trim_end);
            if let Some(clip) = r.clips.iter_mut().find(|c| c.id == trim.clip_id) {
                clip.trim_end_frames = new_trim_end;
                clip.duration_samples = new_duration;
            }
        }
    }
}

pub fn end_clip_trim(r: &mut Resonance) {
    if let Some(trim) = r.interaction.clip_trim.take() {
        if let Some(clip) = r.clips.iter().find(|c| c.id == trim.clip_id) {
            r.engine.send(AudioCommand::TrimClip {
                clip_id: trim.clip_id,
                new_start_sample: clip.start_sample,
                trim_start_frames: clip.trim_start_frames,
                trim_end_frames: clip.trim_end_frames,
            });
        }
    }
}

// -- MIDI clip drag/trim -----------------------------------------------

pub fn start_midi_clip_drag(
    r: &mut Resonance,
    clip_id: ClipId,
    grab_offset_x: f32,
    start_x: f32,
    start_y: f32,
) {
    if let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) {
        r.interaction.selected_midi_clip = Some(clip_id);
        r.interaction.selected_clip = None;
        r.interaction.selected_track = Some(clip.track_id);
        r.interaction.midi_clip_drag = Some(MidiClipDragState {
            clip_id,
            grab_offset_x,
            original_track_id: clip.track_id,
            current_x: start_x,
            current_y: start_y,
        });
    }
}

pub fn update_midi_clip_drag(r: &mut Resonance, x: f32, y: f32) {
    let original_track_id = r
        .interaction
        .midi_clip_drag
        .as_ref()
        .map(|d| d.original_track_id);
    let Some(orig) = original_track_id else {
        return;
    };
    let target_track_id = r.track_id_at_arrange_y(y).unwrap_or(orig);
    let bpm = r.transport.bpm;
    let num = r.transport.time_sig_num;
    let sample_rate = r.sample_rate;
    let zoom = r.viewport.zoom;
    let scroll = r.viewport.scroll_offset;
    if let Some(ref mut drag) = r.interaction.midi_clip_drag {
        drag.current_x = x;
        drag.current_y = y;
        let seconds = ((x - drag.grab_offset_x) + scroll) / zoom;
        let raw = if seconds < 0.0 {
            0u64
        } else {
            (seconds as f64 * sample_rate as f64) as u64
        };
        let new_start = crate::timeline::snap_sample_to_grid(raw, bpm, num, sample_rate, zoom);
        let clip_id = drag.clip_id;
        if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == clip_id) {
            clip.start_sample = new_start;
            clip.track_id = target_track_id;
        }
    }
}

pub fn end_midi_clip_drag(r: &mut Resonance) {
    if let Some(drag) = r.interaction.midi_clip_drag.take() {
        if let Some(clip) = r.midi_clips.iter().find(|c| c.id == drag.clip_id) {
            r.engine.send(AudioCommand::MoveMidiClip {
                clip_id: drag.clip_id,
                new_start_sample: clip.start_sample,
                new_track_id: clip.track_id,
            });
        }
    }
}

pub fn start_midi_clip_trim(r: &mut Resonance, clip_id: ClipId, edge: ClipEdge, anchor_x: f32) {
    if let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) {
        r.interaction.selected_midi_clip = Some(clip_id);
        r.interaction.selected_clip = None;
        r.interaction.selected_track = Some(clip.track_id);
        r.interaction.midi_clip_trim = Some(MidiClipTrimState {
            clip_id,
            edge,
            original_start_sample: clip.start_sample,
            original_duration_ticks: clip.duration_ticks,
            original_trim_start_ticks: clip.trim_start_ticks,
            original_trim_end_ticks: clip.trim_end_ticks,
            anchor_x,
        });
    }
}

pub fn update_midi_clip_trim(r: &mut Resonance, x: f32) {
    let Some(trim) = r.interaction.midi_clip_trim.clone() else {
        return;
    };
    let delta_px = x - trim.anchor_x;
    let delta_seconds = delta_px as f64 / r.viewport.zoom as f64;
    let samples_per_tick =
        (r.sample_rate as f64 * 60.0 / r.transport.bpm as f64) / TICKS_PER_QUARTER_NOTE as f64;
    let delta_samples_signed = (delta_seconds * r.sample_rate as f64) as i64;
    let total_ticks = trim.original_duration_ticks
        + trim.original_trim_start_ticks
        + trim.original_trim_end_ticks;
    let min_ticks = TICKS_PER_QUARTER_NOTE;
    // Snap the dragged edge's final sample-space position to the grid,
    // then translate the resulting delta back into ticks. This keeps
    // MIDI clip edges aligned with the arrange-view bar/beat lines
    // without needing a tick-domain snap.
    let snap = |sample: u64| -> u64 {
        crate::timeline::snap_sample_to_grid(
            sample,
            r.transport.bpm,
            r.transport.time_sig_num,
            r.sample_rate,
            r.viewport.zoom,
        )
    };
    let original_duration_samples = (trim.original_duration_ticks as f64 * samples_per_tick) as u64;

    match trim.edge {
        ClipEdge::Left => {
            let original_edge = trim.original_start_sample;
            let raw_target = (original_edge as i64 + delta_samples_signed).max(0) as u64;
            let snapped_delta_samples = snap(raw_target) as i64 - original_edge as i64;
            let snapped_delta_ticks = (snapped_delta_samples as f64 / samples_per_tick) as i64;
            let max_trim = total_ticks
                .saturating_sub(trim.original_trim_end_ticks)
                .saturating_sub(min_ticks);
            let new_trim_start = if snapped_delta_ticks >= 0 {
                (trim.original_trim_start_ticks + snapped_delta_ticks as u64).min(max_trim)
            } else {
                trim.original_trim_start_ticks
                    .saturating_sub((-snapped_delta_ticks) as u64)
            };
            let trim_delta = new_trim_start as i64 - trim.original_trim_start_ticks as i64;
            let sample_delta = (trim_delta as f64 * samples_per_tick) as i64;
            let new_start = (trim.original_start_sample as i64 + sample_delta).max(0) as u64;
            let new_duration = total_ticks
                .saturating_sub(new_trim_start)
                .saturating_sub(trim.original_trim_end_ticks);
            if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == trim.clip_id) {
                clip.start_sample = new_start;
                clip.trim_start_ticks = new_trim_start;
                clip.duration_ticks = new_duration;
            }
        }
        ClipEdge::Right => {
            let original_edge = trim.original_start_sample + original_duration_samples;
            let raw_target = (original_edge as i64 + delta_samples_signed).max(0) as u64;
            let snapped_delta_samples = snap(raw_target) as i64 - original_edge as i64;
            let snapped_delta_ticks = (snapped_delta_samples as f64 / samples_per_tick) as i64;
            let max_trim = total_ticks
                .saturating_sub(trim.original_trim_start_ticks)
                .saturating_sub(min_ticks);
            let new_trim_end = if snapped_delta_ticks <= 0 {
                (trim.original_trim_end_ticks + (-snapped_delta_ticks) as u64).min(max_trim)
            } else {
                trim.original_trim_end_ticks
                    .saturating_sub(snapped_delta_ticks as u64)
            };
            let new_duration = total_ticks
                .saturating_sub(trim.original_trim_start_ticks)
                .saturating_sub(new_trim_end);
            if let Some(clip) = r.midi_clips.iter_mut().find(|c| c.id == trim.clip_id) {
                clip.trim_end_ticks = new_trim_end;
                clip.duration_ticks = new_duration;
            }
        }
    }
}

pub fn end_midi_clip_trim(r: &mut Resonance) {
    if let Some(trim) = r.interaction.midi_clip_trim.take() {
        if let Some(clip) = r.midi_clips.iter().find(|c| c.id == trim.clip_id) {
            r.engine.send(AudioCommand::TrimMidiClip {
                clip_id: trim.clip_id,
                new_start_sample: clip.start_sample,
                trim_start_ticks: clip.trim_start_ticks,
                trim_end_ticks: clip.trim_end_ticks,
            });
        }
    }
}

/// Initialize the piano roll editor state for the given MIDI clip.
pub fn open_midi_editor(r: &mut Resonance, clip_id: ClipId) {
    if let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) {
        // Vocal tracks render in the vocal-roll variant, which uses a
        // local note range (the singer's tessitura) rather than the
        // full 0..127 keyboard. Start it at scroll_y = 0 so the entire
        // range is in view; the standard piano roll keeps its
        // historical 5-octave default scroll so it lands around C4.
        let is_vocal = r
            .registry
            .tracks
            .iter()
            .find(|t| t.id == clip.track_id)
            .map(|t| t.track_type == resonance_audio::types::TrackType::Vocal)
            .unwrap_or(false);
        let scroll_y = if is_vocal { 0.0 } else { 60.0 * 5.0 };
        r.interaction.editing_midi_clip = Some(MidiEditorState {
            clip_id,
            track_id: clip.track_id,
            scroll_y,
            zoom_x: 0.5,
            zoom_y: 12.0,
            snap_ticks: TICKS_PER_QUARTER_NOTE / 4,
            selected_note: None,
        });
    }
}
