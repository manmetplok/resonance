//! Roadmap group (5): the audio and MIDI clips themselves, and the vocal
//! audio-clip map rebuilt from them (ARCH-01 A-13d).
//!
//! Shaped like `globals::Transport`: after a `ClearAll` (`old = None`) the
//! mirror is emptied and every clip is loaded; on the diff path each clip
//! is moved / trimmed / re-faded / re-gained / reloaded by diff against
//! `old`. The diff path relies on `structurally_compatible`: the clip-id
//! sets are equal (and an audio clip's WAV and length unchanged), so it
//! never adds or deletes a clip. A clip id missing from `old` is skipped
//! there as defence in depth rather than loaded.

use std::collections::HashMap;
use std::path::Path;

use resonance_audio::types::{AudioCommand, ClipId, MidiNote};

use super::{Reconcile, ReconcileCtx};
use crate::project::{fade_curve_from_tag, ProjectClip, ProjectFile, ProjectMidiClip};
use crate::state::{ClipState, MidiClipState};
use crate::Resonance;

/// The directory a clip's `audio_file` resolves against. Always set on the
/// full paths; an untitled project's diff restore resolves against `""`, as
/// before.
fn project_dir<'a>(ctx: &ReconcileCtx<'a>) -> &'a Path {
    ctx.project_dir.unwrap_or(Path::new(""))
}

/// Audio clips: the engine's clip set and the `r.clips` mirror, including
/// each clip's pool link (`asset_ref`, doc #175) — hence at the head of
/// `Clips`, before the Content stage's `Pool`, which counts those links.
///
/// * After a `ClearAll`: the mirror is emptied, then every clip goes out as
///   `LoadClipFromWav` (absolute path under the project dir), followed by
///   `SetClipFade` / `SetClipGain` only when non-default (the load carries
///   neither; legacy / unfaded projects stay quiet). The WAVs an undo
///   reloads exist because `snapshot_for_undo` sent `PersistClipWavs`
///   (FU-V5b); the engine bumps its clip-id allocator past each loaded id
///   (STATE-08).
/// * Diff: `TrimClip` if the trim changed, else `MoveClip` if the start or
///   track changed; then `SetClipFade` / `SetClipGain` when they changed;
///   the mirror is updated in place (peaks and tuning kept).
pub(crate) struct AudioClips;

impl Reconcile for AudioClips {
    const NAME: &'static str = "audio_clips";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let Some(old) = old else {
            r.clips.clear();
            let dir = project_dir(ctx);
            for pc in &new.clips {
                load_audio_clip(r, pc, dir);
            }
            return;
        };
        let old_by_id: HashMap<u64, &ProjectClip> = old.clips.iter().map(|c| (c.id, c)).collect();
        for pc in &new.clips {
            if let Some(&oc) = old_by_id.get(&pc.id) {
                apply_audio_clip(r, oc, pc);
            }
        }
    }
}

fn load_audio_clip(r: &mut Resonance, pc: &ProjectClip, dir: &Path) {
    let _ = r.engine.send(AudioCommand::LoadClipFromWav {
        clip_id: pc.id,
        track_id: pc.track_id,
        start_sample: pc.start_sample,
        path: dir.join(&pc.audio_file),
        name: pc.name.clone(),
        trim_start_frames: pc.trim_start_frames,
        trim_end_frames: pc.trim_end_frames,
    });

    // Fades & per-clip gain (epic #18, doc #156). `LoadClipFromWav`
    // carries no fade/gain, so push them explicitly after the clip exists;
    // the engine clamps and echoes them back. Only when non-default.
    let fade_in_curve = fade_curve_from_tag(&pc.fade_in_curve);
    let fade_out_curve = fade_curve_from_tag(&pc.fade_out_curve);
    if pc.fade_in_frames != 0 || pc.fade_out_frames != 0 {
        let _ = r.engine.send(AudioCommand::SetClipFade {
            clip_id: pc.id,
            fade_in_frames: pc.fade_in_frames,
            fade_in_curve,
            fade_out_frames: pc.fade_out_frames,
            fade_out_curve,
        });
    }
    if pc.gain_db != 0.0 {
        let _ = r.engine.send(AudioCommand::SetClipGain {
            clip_id: pc.id,
            gain_db: pc.gain_db,
        });
    }

    r.clips.push(ClipState {
        id: pc.id,
        track_id: pc.track_id,
        start_sample: pc.start_sample,
        duration_samples: audio_clip_duration(pc),
        name: pc.name.clone(),
        total_frames: pc.total_frames,
        trim_start_frames: pc.trim_start_frames,
        trim_end_frames: pc.trim_end_frames,
        fade_in_frames: pc.fade_in_frames,
        fade_in_curve,
        fade_out_frames: pc.fade_out_frames,
        fade_out_curve,
        gain_db: pc.gain_db,
        waveform_peaks: Vec::new(), // Populated by the ClipImported event.
        vocal_tuning: None,         // Re-derived on demand when the pitch editor opens.
        // The pool link (doc #175); `Pool` reconciles it against the pool
        // and recomputes usage once every clip is in place.
        asset_ref: pc.asset_ref.map(crate::state::pool::AssetRef::new),
    });
}

fn apply_audio_clip(r: &mut Resonance, oc: &ProjectClip, pc: &ProjectClip) {
    let trim_changed =
        oc.trim_start_frames != pc.trim_start_frames || oc.trim_end_frames != pc.trim_end_frames;
    let moved = oc.start_sample != pc.start_sample || oc.track_id != pc.track_id;
    if trim_changed {
        let _ = r.engine.send(AudioCommand::TrimClip {
            clip_id: pc.id,
            new_start_sample: pc.start_sample,
            trim_start_frames: pc.trim_start_frames,
            trim_end_frames: pc.trim_end_frames,
        });
    } else if moved {
        let _ = r.engine.send(AudioCommand::MoveClip {
            clip_id: pc.id,
            new_start_sample: pc.start_sample,
            new_track_id: pc.track_id,
        });
    }

    // Fades & gain, independent of the trim/move above: an undo that only
    // changes a fade length, curve or gain must still reach the engine.
    // Curves round-trip as tags, so compare the parsed `FadeCurve`
    // (normalising unknown / legacy tags) rather than the strings.
    let fo_in = fade_curve_from_tag(&oc.fade_in_curve);
    let fn_in = fade_curve_from_tag(&pc.fade_in_curve);
    let fo_out = fade_curve_from_tag(&oc.fade_out_curve);
    let fn_out = fade_curve_from_tag(&pc.fade_out_curve);
    let fade_changed = oc.fade_in_frames != pc.fade_in_frames
        || oc.fade_out_frames != pc.fade_out_frames
        || fo_in != fn_in
        || fo_out != fn_out;
    if fade_changed {
        let _ = r.engine.send(AudioCommand::SetClipFade {
            clip_id: pc.id,
            fade_in_frames: pc.fade_in_frames,
            fade_in_curve: fn_in,
            fade_out_frames: pc.fade_out_frames,
            fade_out_curve: fn_out,
        });
    }
    if oc.gain_db != pc.gain_db {
        let _ = r.engine.send(AudioCommand::SetClipGain {
            clip_id: pc.id,
            gain_db: pc.gain_db,
        });
    }

    if let Some(cs) = r.clips.iter_mut().find(|c| c.id == pc.id) {
        cs.start_sample = pc.start_sample;
        cs.track_id = pc.track_id;
        cs.trim_start_frames = pc.trim_start_frames;
        cs.trim_end_frames = pc.trim_end_frames;
        cs.name = pc.name.clone();
        cs.duration_samples = audio_clip_duration(pc);
        cs.fade_in_frames = pc.fade_in_frames;
        cs.fade_in_curve = fn_in;
        cs.fade_out_frames = pc.fade_out_frames;
        cs.fade_out_curve = fn_out;
        cs.gain_db = pc.gain_db;
        // An undo that relinked or cleared the pool link is reflected.
        cs.asset_ref = pc.asset_ref.map(crate::state::pool::AssetRef::new);
    }
}

fn audio_clip_duration(pc: &ProjectClip) -> u64 {
    pc.total_frames
        .saturating_sub(pc.trim_start_frames)
        .saturating_sub(pc.trim_end_frames)
}

/// MIDI clips: the engine's clip set and the `r.midi_clips` mirror, notes
/// from [`ReconcileCtx::midi_notes`] (the `ProjectFile` carries none). After
/// `AudioClips` (the order both paths always had), before `ClipLyrics`
/// (pads to the restored note counts) and `DerivedClips` (filters against
/// the restored ids).
///
/// * After a `ClearAll`: the mirror is emptied, then every clip goes out as
///   `LoadMidiClipDirect`.
/// * Diff: a clip whose notes or length changed is reloaded by
///   `DeleteMidiClip` + `LoadMidiClipDirect` (keeps the id, so the track
///   binding and the derived-map keys stay valid); else `TrimMidiClip` if
///   the trim changed, else `MoveMidiClip` if the start or track changed.
///   Notes are compared against the live mirror, not `old` (the snapshot
///   file has no notes).
pub(crate) struct MidiClips;

impl Reconcile for MidiClips {
    const NAME: &'static str = "midi_clips";

    fn reconcile(r: &mut Resonance, old: Option<&ProjectFile>, new: &ProjectFile, ctx: &ReconcileCtx<'_>) {
        let target_notes = |id: ClipId| -> Vec<MidiNote> {
            ctx.midi_notes.get(&id).cloned().unwrap_or_default()
        };
        let Some(old) = old else {
            r.midi_clips.clear();
            for pmc in &new.midi_clips {
                load_midi_clip(r, pmc, target_notes(pmc.id));
            }
            return;
        };
        let old_by_id: HashMap<u64, &ProjectMidiClip> =
            old.midi_clips.iter().map(|c| (c.id, c)).collect();
        let live_notes: HashMap<ClipId, Vec<MidiNote>> = r
            .midi_clips
            .iter()
            .map(|mc| (mc.id, mc.notes.clone()))
            .collect();
        for pmc in &new.midi_clips {
            let Some(&omc) = old_by_id.get(&pmc.id) else {
                continue;
            };
            let live = live_notes.get(&pmc.id).map(Vec::as_slice).unwrap_or(&[]);
            apply_midi_clip(r, omc, pmc, live, target_notes(pmc.id));
        }
    }
}

fn load_midi_clip(r: &mut Resonance, pmc: &ProjectMidiClip, notes: Vec<MidiNote>) {
    let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
        clip_id: pmc.id,
        track_id: pmc.track_id,
        start_sample: pmc.start_sample,
        duration_ticks: pmc.duration_ticks,
        notes: notes.clone(),
        name: pmc.name.clone(),
        trim_start_ticks: pmc.trim_start_ticks,
        trim_end_ticks: pmc.trim_end_ticks,
    });
    r.midi_clips.push(MidiClipState {
        id: pmc.id,
        track_id: pmc.track_id,
        start_sample: pmc.start_sample,
        duration_ticks: pmc.duration_ticks,
        name: pmc.name.clone(),
        notes,
        trim_start_ticks: pmc.trim_start_ticks,
        trim_end_ticks: pmc.trim_end_ticks,
    });
}

fn apply_midi_clip(
    r: &mut Resonance,
    omc: &ProjectMidiClip,
    pmc: &ProjectMidiClip,
    live_notes: &[MidiNote],
    notes: Vec<MidiNote>,
) {
    let notes_changed = !crate::update::project_io::replay_diff::midi_notes_equal(&notes, live_notes);
    let trim_changed =
        omc.trim_start_ticks != pmc.trim_start_ticks || omc.trim_end_ticks != pmc.trim_end_ticks;
    let moved = omc.start_sample != pmc.start_sample || omc.track_id != pmc.track_id;
    let duration_changed = omc.duration_ticks != pmc.duration_ticks;

    if notes_changed || duration_changed {
        let _ = r.engine.send(AudioCommand::DeleteMidiClip { clip_id: pmc.id });
        let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
            clip_id: pmc.id,
            track_id: pmc.track_id,
            start_sample: pmc.start_sample,
            duration_ticks: pmc.duration_ticks,
            notes: notes.clone(),
            name: pmc.name.clone(),
            trim_start_ticks: pmc.trim_start_ticks,
            trim_end_ticks: pmc.trim_end_ticks,
        });
    } else if trim_changed {
        let _ = r.engine.send(AudioCommand::TrimMidiClip {
            clip_id: pmc.id,
            new_start_sample: pmc.start_sample,
            trim_start_ticks: pmc.trim_start_ticks,
            trim_end_ticks: pmc.trim_end_ticks,
        });
    } else if moved {
        let _ = r.engine.send(AudioCommand::MoveMidiClip {
            clip_id: pmc.id,
            new_start_sample: pmc.start_sample,
            new_track_id: pmc.track_id,
        });
    }

    if let Some(mc) = r.midi_clips.iter_mut().find(|c| c.id == pmc.id) {
        mc.start_sample = pmc.start_sample;
        mc.track_id = pmc.track_id;
        mc.duration_ticks = pmc.duration_ticks;
        mc.trim_start_ticks = pmc.trim_start_ticks;
        mc.trim_end_ticks = pmc.trim_end_ticks;
        mc.name = pmc.name.clone();
        mc.notes = notes;
    }
}
