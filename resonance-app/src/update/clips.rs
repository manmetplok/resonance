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
            let _ = r.engine.send(AudioCommand::DeleteClip { clip_id: id });
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
        // Fade/gain drag handling (state machine + engine commands). The
        // timeline (todo #318) emits these on the right hits; the undo
        // snapshot is captured/committed by the `Begin`/`Commit`
        // classification in `undo::classify` (same as trim/move), so the
        // handlers below only mutate the live `ClipState` mirror and, on
        // gesture end, send the matching engine command.
        ClipMessage::StartClipFadeDrag {
            clip_id,
            edge,
            anchor_x,
        } => {
            start_clip_fade_drag(r, clip_id, edge, anchor_x);
        }
        ClipMessage::UpdateClipFadeDrag(x) => {
            update_clip_fade_drag(r, x);
        }
        ClipMessage::EndClipFadeDrag => {
            end_clip_fade_drag(r);
        }
        ClipMessage::StartClipGainDrag { clip_id, anchor_y } => {
            start_clip_gain_drag(r, clip_id, anchor_y);
        }
        ClipMessage::UpdateClipGainDrag(y) => {
            update_clip_gain_drag(r, y);
        }
        ClipMessage::EndClipGainDrag => {
            end_clip_gain_drag(r);
        }
        // Inspector flyout edits (emitted by todo #319). Each is a discrete,
        // atomic edit (`UndoAction::Record` in `undo::classify`): mutate the
        // live mirror and send the matching engine command.
        ClipMessage::SetClipFadeInMs { clip_id, ms } => {
            set_clip_fade_in_ms(r, clip_id, ms);
        }
        ClipMessage::SetClipFadeOutMs { clip_id, ms } => {
            set_clip_fade_out_ms(r, clip_id, ms);
        }
        ClipMessage::SetClipGainDb { clip_id, gain_db } => {
            set_clip_gain_db(r, clip_id, gain_db);
        }
        ClipMessage::SetClipFadeInCurve { clip_id, curve } => {
            set_clip_fade_curve(r, clip_id, ClipEdge::Left, curve);
        }
        ClipMessage::SetClipFadeOutCurve { clip_id, curve } => {
            set_clip_fade_curve(r, clip_id, ClipEdge::Right, curve);
        }
        ClipMessage::ResetClipFadeGain { clip_id } => {
            reset_clip_fade_gain(r, clip_id);
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
        let new_start = crate::view::timeline::snap_sample_to_grid(raw, bpm, num, sample_rate, zoom);
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
            let _ = r.engine.send(AudioCommand::MoveClip {
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
        crate::view::timeline::snap_sample_to_grid(
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
            let _ = r.engine.send(AudioCommand::TrimClip {
                clip_id: trim.clip_id,
                new_start_sample: clip.start_sample,
                trim_start_frames: clip.trim_start_frames,
                trim_end_frames: clip.trim_end_frames,
            });
        }
    }
}

// -- Audio clip fade handles -------------------------------------------

/// Begin a fade-handle drag. Snapshots the clip's start + visible length
/// so the live pointer→fade-length conversion stays stable as the mirror
/// is mutated mid-drag (mirrors [`start_clip_trim`]). `_anchor_x` is unused
/// — the fade handle tracks the pointer x directly (handle x = ramp end,
/// design doc #153) — but kept in the message for symmetry with trim.
pub fn start_clip_fade_drag(r: &mut Resonance, clip_id: ClipId, edge: ClipEdge, _anchor_x: f32) {
    if let Some(clip) = r.clips.iter().find(|c| c.id == clip_id) {
        r.interaction.selected_clip = Some(clip_id);
        r.interaction.selected_track = Some(clip.track_id);
        r.interaction.clip_fade_drag = Some(FadeDragState {
            clip_id,
            edge,
            original_start_sample: clip.start_sample,
            original_duration_samples: clip.duration_samples,
        });
    }
}

/// Update the active fade drag from the pointer x. Converts the pointer
/// into a fade length in frames against the clip's start (fade-in) or end
/// (fade-out) edge, clamped to the clip's audible length, and writes it to
/// the live mirror. No engine command yet — that's sent on `End`.
pub fn update_clip_fade_drag(r: &mut Resonance, x: f32) {
    let Some(drag) = r.interaction.clip_fade_drag.clone() else {
        return;
    };
    let zoom = r.viewport.zoom;
    let scroll = r.viewport.scroll_offset;
    let sample_rate = r.sample_rate;
    // Clip edges in canvas pixels (same mapping as `hit_test::clip_rect`).
    let left_px = drag.original_start_sample as f32 / sample_rate as f32 * zoom - scroll;
    let right_px = (drag.original_start_sample + drag.original_duration_samples) as f32
        / sample_rate as f32
        * zoom
        - scroll;
    // Length in pixels of the dragged ramp, then to frames. A fade can't be
    // longer than the clip is audible (matches the engine's clamp).
    let len_px = match drag.edge {
        ClipEdge::Left => (x - left_px).max(0.0),
        ClipEdge::Right => (right_px - x).max(0.0),
    };
    let frames = px_to_frames(len_px, zoom, sample_rate).min(drag.original_duration_samples);
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == drag.clip_id) {
        match drag.edge {
            ClipEdge::Left => clip.fade_in_frames = frames,
            ClipEdge::Right => clip.fade_out_frames = frames,
        }
    }
}

/// Commit the fade drag: read the (clamped) fade values back from the live
/// mirror and push the full `SetClipFade` to the engine. The undo entry is
/// committed by the `EndClipFadeDrag` → `Commit` classification.
pub fn end_clip_fade_drag(r: &mut Resonance) {
    if let Some(drag) = r.interaction.clip_fade_drag.take() {
        if let Some(clip) = r.clips.iter().find(|c| c.id == drag.clip_id) {
            send_clip_fade(r, clip_snapshot(clip));
        }
    }
}

// -- Audio clip gain bead ----------------------------------------------

/// dB change per pixel of vertical drag on the clip-gain bead. A small,
/// precise sensitivity: a full 96px track-height drag moves ~14 dB, so the
/// whole gain range is reachable without the bead feeling twitchy.
pub const GAIN_DB_PER_PX: f32 = 0.15;

/// Begin a clip-gain drag. Gain is a vertical gesture, so the anchor y and
/// the gain at grab time are captured; each move computes an absolute
/// target from the total delta (no per-frame accumulation drift).
pub fn start_clip_gain_drag(r: &mut Resonance, clip_id: ClipId, anchor_y: f32) {
    if let Some(clip) = r.clips.iter().find(|c| c.id == clip_id) {
        r.interaction.selected_clip = Some(clip_id);
        r.interaction.selected_track = Some(clip.track_id);
        r.interaction.clip_gain_drag = Some(GainDragState {
            clip_id,
            anchor_y,
            original_gain_db: clip.gain_db,
        });
    }
}

/// Update the active gain drag from the pointer y. Dragging up (smaller y)
/// increases gain; the result is clamped to the engine's accepted range
/// and written to the live mirror.
pub fn update_clip_gain_drag(r: &mut Resonance, y: f32) {
    let Some(drag) = r.interaction.clip_gain_drag.clone() else {
        return;
    };
    let delta_db = (drag.anchor_y - y) * GAIN_DB_PER_PX;
    let new_gain = clamp_gain_db(drag.original_gain_db + delta_db);
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == drag.clip_id) {
        clip.gain_db = new_gain;
    }
}

/// Commit the gain drag: push the live mirror's gain to the engine.
pub fn end_clip_gain_drag(r: &mut Resonance) {
    if let Some(drag) = r.interaction.clip_gain_drag.take() {
        if let Some(clip) = r.clips.iter().find(|c| c.id == drag.clip_id) {
            let gain_db = clip.gain_db;
            let _ = r.engine.send(AudioCommand::SetClipGain {
                clip_id: drag.clip_id,
                gain_db,
            });
        }
    }
}

// -- Inspector flyout edits (todo #319) --------------------------------

/// Set the fade-in length from a millisecond value (inspector numeric
/// field). Converts to frames, clamps to the clip's audible length, writes
/// the mirror, and sends the full `SetClipFade`.
pub fn set_clip_fade_in_ms(r: &mut Resonance, clip_id: ClipId, ms: f32) {
    let sample_rate = r.sample_rate;
    let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) else {
        return;
    };
    let frames = ms_to_frames(ms, sample_rate).min(clip.duration_samples);
    clip.fade_in_frames = frames;
    let snap = clip_snapshot(clip);
    send_clip_fade(r, snap);
}

/// Set the fade-out length from a millisecond value (inspector numeric
/// field).
pub fn set_clip_fade_out_ms(r: &mut Resonance, clip_id: ClipId, ms: f32) {
    let sample_rate = r.sample_rate;
    let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) else {
        return;
    };
    let frames = ms_to_frames(ms, sample_rate).min(clip.duration_samples);
    clip.fade_out_frames = frames;
    let snap = clip_snapshot(clip);
    send_clip_fade(r, snap);
}

/// Set the clip gain from a dB value (inspector numeric field). Clamped to
/// the engine's range, mirrored, and sent.
pub fn set_clip_gain_db(r: &mut Resonance, clip_id: ClipId, gain_db: f32) {
    let gain_db = clamp_gain_db(gain_db);
    if let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) {
        clip.gain_db = gain_db;
        let _ = r
            .engine
            .send(AudioCommand::SetClipGain { clip_id, gain_db });
    }
}

/// Choose a fade curve for one edge (inspector curve picker). The fade
/// lengths are unchanged; the full `SetClipFade` re-sends both lengths and
/// both curves from the mirror.
pub fn set_clip_fade_curve(r: &mut Resonance, clip_id: ClipId, edge: ClipEdge, curve: FadeCurve) {
    let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) else {
        return;
    };
    match edge {
        ClipEdge::Left => clip.fade_in_curve = curve,
        ClipEdge::Right => clip.fade_out_curve = curve,
    }
    let snap = clip_snapshot(clip);
    send_clip_fade(r, snap);
}

/// Reset a clip's fades and gain to defaults — no fade, default curves,
/// unity gain (inspector "Reset to default"). Sends both `SetClipFade` and
/// `SetClipGain` so the engine and mirror return to the pristine state.
pub fn reset_clip_fade_gain(r: &mut Resonance, clip_id: ClipId) {
    let Some(clip) = r.clips.iter_mut().find(|c| c.id == clip_id) else {
        return;
    };
    clip.fade_in_frames = 0;
    clip.fade_in_curve = FadeCurve::default();
    clip.fade_out_frames = 0;
    clip.fade_out_curve = FadeCurve::default();
    clip.gain_db = 0.0;
    let snap = clip_snapshot(clip);
    send_clip_fade(r, snap);
    let _ = r.engine.send(AudioCommand::SetClipGain {
        clip_id,
        gain_db: 0.0,
    });
}

// -- shared helpers ----------------------------------------------------

/// The fade fields of a clip, captured so we can release the `&clip` borrow
/// before calling `r.engine.send` (which borrows `r`).
#[derive(Clone, Copy)]
struct ClipFadeSnapshot {
    clip_id: ClipId,
    fade_in_frames: u64,
    fade_in_curve: FadeCurve,
    fade_out_frames: u64,
    fade_out_curve: FadeCurve,
}

fn clip_snapshot(clip: &ClipState) -> ClipFadeSnapshot {
    ClipFadeSnapshot {
        clip_id: clip.id,
        fade_in_frames: clip.fade_in_frames,
        fade_in_curve: clip.fade_in_curve,
        fade_out_frames: clip.fade_out_frames,
        fade_out_curve: clip.fade_out_curve,
    }
}

/// Send the full `SetClipFade` command from a captured fade snapshot. The
/// engine re-clamps the lengths and echoes `ClipFadeChanged`, keeping the
/// mirror authoritative.
fn send_clip_fade(r: &Resonance, snap: ClipFadeSnapshot) {
    let _ = r.engine.send(AudioCommand::SetClipFade {
        clip_id: snap.clip_id,
        fade_in_frames: snap.fade_in_frames,
        fade_in_curve: snap.fade_in_curve,
        fade_out_frames: snap.fade_out_frames,
        fade_out_curve: snap.fade_out_curve,
    });
}

/// Convert a pixel length on the timeline to a frame count at the current
/// zoom and sample rate. Rounds to the nearest frame.
fn px_to_frames(px: f32, zoom: f32, sample_rate: u32) -> u64 {
    if px <= 0.0 || zoom <= 0.0 {
        return 0;
    }
    let seconds = px / zoom;
    (seconds as f64 * sample_rate as f64).round().max(0.0) as u64
}

/// Convert a millisecond duration to a frame count at the sample rate.
fn ms_to_frames(ms: f32, sample_rate: u32) -> u64 {
    if ms <= 0.0 {
        return 0;
    }
    ((ms as f64 / 1000.0) * sample_rate as f64).round().max(0.0) as u64
}

/// Clamp a gain value to the engine's accepted range. A `NaN` collapses to
/// unity, matching the engine's own guard so the mirror never diverges.
fn clamp_gain_db(gain_db: f32) -> f32 {
    if gain_db.is_nan() {
        0.0
    } else {
        gain_db.clamp(
            resonance_audio::MIN_CLIP_GAIN_DB,
            resonance_audio::MAX_CLIP_GAIN_DB,
        )
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
        let new_start = crate::view::timeline::snap_sample_to_grid(raw, bpm, num, sample_rate, zoom);
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
            let _ = r.engine.send(AudioCommand::MoveMidiClip {
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
    let delta_samples_signed = (delta_seconds * r.sample_rate as f64) as i64;
    let total_ticks = trim.original_duration_ticks
        + trim.original_trim_start_ticks
        + trim.original_trim_end_ticks;
    let min_ticks = TICKS_PER_QUARTER_NOTE;
    let sample_rate = r.sample_rate;
    let tempo_map = &r.tempo_map;
    // Snap the dragged edge's final sample-space position to the grid,
    // then translate the resulting delta back into ticks. This keeps
    // MIDI clip edges aligned with the arrange-view bar/beat lines
    // without needing a tick-domain snap.
    let snap = |sample: u64| -> u64 {
        crate::view::timeline::snap_sample_to_grid(
            sample,
            r.transport.bpm,
            r.transport.time_sig_num,
            sample_rate,
            r.viewport.zoom,
        )
    };
    // Convert a sample-space delta against `anchor_sample` into a
    // signed tick-space delta, integrating tempo changes via the bar
    // table. A scalar `samples_per_tick` would skew the result whenever
    // the trim region spans a tempo change.
    let sample_delta_to_tick_delta = |anchor_sample: u64, new_sample: u64| -> i64 {
        let anchor_tick = tempo_map.sample_to_abs_tick(anchor_sample, sample_rate) as i64;
        let new_tick = tempo_map.sample_to_abs_tick(new_sample, sample_rate) as i64;
        new_tick - anchor_tick
    };

    match trim.edge {
        ClipEdge::Left => {
            let original_edge = trim.original_start_sample;
            let raw_target = (original_edge as i64 + delta_samples_signed).max(0) as u64;
            let snapped_target = snap(raw_target);
            let snapped_delta_ticks = sample_delta_to_tick_delta(original_edge, snapped_target);
            let max_trim = total_ticks
                .saturating_sub(trim.original_trim_end_ticks)
                .saturating_sub(min_ticks);
            let new_trim_start = if snapped_delta_ticks >= 0 {
                (trim.original_trim_start_ticks + snapped_delta_ticks as u64).min(max_trim)
            } else {
                trim.original_trim_start_ticks
                    .saturating_sub((-snapped_delta_ticks) as u64)
            };
            // Project the clamped `new_trim_start` back into sample
            // space using the bar table: the new visible start sits at
            // (original_start tick) + (new_trim_start - original_trim_start)
            // ticks in absolute tick coordinates. `tick_to_abs_sample`
            // with a clip_start of 0 is the inverse of `sample_to_abs_tick`,
            // so this round-trips through the tempo map cleanly.
            let original_start_abs_tick =
                tempo_map.sample_to_abs_tick(trim.original_start_sample, sample_rate) as i64;
            let new_start_abs_tick = original_start_abs_tick + new_trim_start as i64
                - trim.original_trim_start_ticks as i64;
            let new_start = if new_start_abs_tick <= 0 {
                0
            } else {
                tempo_map.tick_to_abs_sample(0, new_start_abs_tick as u64, sample_rate)
            };
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
            let original_edge = tempo_map.tick_to_abs_sample(
                trim.original_start_sample,
                trim.original_duration_ticks,
                sample_rate,
            );
            let raw_target = (original_edge as i64 + delta_samples_signed).max(0) as u64;
            let snapped_target = snap(raw_target);
            let snapped_delta_ticks = sample_delta_to_tick_delta(original_edge, snapped_target);
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
            let _ = r.engine.send(AudioCommand::TrimMidiClip {
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
            selected_notes: std::collections::BTreeSet::new(),
        });
    }
}
