//! Vocal-lane render pipeline. Splits cleanly from the message-routing
//! `lane_inspector` module: this file owns the side-effects of producing
//! audible vocals — deriving the MIDI clip, queuing the off-thread SVS
//! render, installing the resulting WAV at every section placement, and
//! the lifecycle bookkeeping (epochs, tear-downs, WAV cleanup) that
//! keeps back-to-back regen presses from stacking clips.

use iced::Task;

use resonance_audio::types::TrackId;
use resonance_music_theory::VocalParams;

use crate::compose::{ComposeMessage, LaneGeneratorKind};
use crate::message::Message;

/// Roll a fresh lyric draft for the vocal lane. Bumps the seed first so
/// repeated presses don't produce the same draft. Locked lines stay put
/// — `generate_lyrics` preserves them and anchors the rhyme pattern to
/// their bucket.
pub(super) fn roll_vocal_lyrics(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    seed_mix: u64,
) {
    let Some(def) = r.compose.find_definition_mut(definition_id) else {
        return;
    };
    let Some(cfg) = def.lane_generators.get_mut(&track_id) else {
        return;
    };
    let LaneGeneratorKind::Vocal(params) = &mut cfg.kind else {
        return;
    };
    cfg.seed = cfg.seed.wrapping_add(seed_mix).wrapping_add(1);
    let seed = cfg.seed;
    params.draft = resonance_music_theory::generate_lyrics(params, seed);
    r.compose.last_error = None;
}

/// Generate a fresh melody MIDI clip for the vocal lane and queue the
/// SVS audio render off-thread. The MIDI side is installed synchronously
/// so the staff updates immediately; the WAV arrives later via the
/// `VocalAudioReady` message dispatched by the returned `Task`.
///
/// Uses the lane config's *current* seed — callers that want a fresh
/// random surface must call `bump_lane_seed` beforehand. This split
/// avoids the previous double-bump where `Regenerate → regenerate_lane
/// → roll_vocal_melody` all bumped the seed in turn.
pub(super) fn roll_vocal_melody(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Task<Message> {
    use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};

    let Some(def) = r.compose.find_definition(definition_id).cloned() else {
        return Task::none();
    };
    let Some(cfg) = def.lane_generators.get(&track_id).cloned() else {
        return Task::none();
    };
    let LaneGeneratorKind::Vocal(params) = cfg.kind else {
        return Task::none();
    };
    if def.chords.is_empty() || params.draft.is_empty() {
        return Task::none();
    }

    let timed = crate::compose::generate::to_timed_chords(&def.chords);
    let beats_per_bar = r.transport.time_sig_num.max(1) as u32;
    let motif_intervals: Vec<i8> = timed
        .first()
        .map(|first| {
            resonance_music_theory::motif_intervals(
                &def.motif_source,
                first.chord,
                def.scale,
            )
        })
        .unwrap_or_default();
    let notes = resonance_music_theory::derive_vocal_with_motif(
        &timed,
        &params,
        TICKS_PER_QUARTER_NOTE as u32,
        beats_per_bar,
        Some(&motif_intervals),
        cfg.seed,
    );
    if notes.is_empty() {
        return Task::none();
    }

    let time_sig_num = r.transport.time_sig_num;
    let samples_per_bar =
        super::regenerate::compose_samples_per_bar(r.sample_rate, r.transport.bpm, time_sig_num);
    let duration_ticks = def.length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;

    let track_name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.as_str())
        .unwrap_or("Vocal");
    let name = format!("{} \u{00B7} {}", def.name, track_name);

    let midi_notes: Vec<MidiNote> = notes
        .iter()
        .map(|n| MidiNote {
            note: n.note,
            velocity: n.velocity,
            start_tick: n.start_tick,
            duration_ticks: n.duration_ticks,
        })
        .collect();

    let placements: Vec<(u64, u32)> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.id, p.start_bar))
        .collect();

    let placement_starts: Vec<(u64, u64)> = placements
        .iter()
        .map(|(pid, start_bar)| (*pid, *start_bar as u64 * samples_per_bar))
        .collect();
    let initial_lyrics: Vec<String> = vec![String::new(); midi_notes.len()];
    VocalMidiInstall {
        definition_id,
        track_id,
        placements: &placement_starts,
        duration_ticks,
        midi_notes: &midi_notes,
        lyrics: &initial_lyrics,
        name: &name,
    }
    .install(r);
    enqueue_vocal_render(
        r,
        definition_id,
        track_id,
        midi_notes,
        initial_lyrics.clone(),
        params,
        placement_starts,
        name,
    )
}

/// Shared off-thread vocal render path. Tears down the prior audio
/// clip, bumps the in-flight epoch (stale-result protection against
/// back-to-back presses), and spawns the SVS pipeline on a blocking
/// thread. The two callers — `roll_vocal_melody` (full regenerate)
/// and `rerender_vocal_audio` (notes-only) — differ only in how they
/// produce `midi_notes` and `lyrics`; everything after that is
/// identical, so it lives here.
#[allow(clippy::too_many_arguments)]
fn enqueue_vocal_render(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    midi_notes: Vec<resonance_audio::types::MidiNote>,
    lyrics: Vec<String>,
    params: resonance_music_theory::VocalParams,
    placement_starts: Vec<(u64, u64)>,
    clip_name: String,
) -> Task<Message> {
    use crate::compose::messages::VocalAudioReadyData;
    tear_down_old_vocal_audio(r, definition_id, track_id);

    let epoch_entry = r
        .compose
        .vocal_audio
        .render_epoch
        .entry((definition_id, track_id))
        .or_insert(0);
    *epoch_entry = epoch_entry.wrapping_add(1);
    let render_epoch = *epoch_entry;

    r.compose.last_error = None;

    let bpm = r.transport.bpm;
    let engine_sr = r.sample_rate;
    let dest_dir = vocal_audio_dir(r);
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                render_vocal_wav(&midi_notes, &params, &lyrics, bpm, engine_sr, &dest_dir)
            })
            .await
            .unwrap_or_else(|join_err| Err(format!("vocal render task join: {join_err}")))
        },
        move |result| match result {
            Ok(Some((wav_path, trim_start, trim_end))) => Message::Compose(
                ComposeMessage::VocalAudioReady(Box::new(VocalAudioReadyData {
                    definition_id,
                    track_id,
                    wav_path,
                    placements: placement_starts.clone(),
                    clip_name: clip_name.clone(),
                    trim_start_frames: trim_start,
                    trim_end_frames: trim_end,
                    render_epoch,
                })),
            ),
            Ok(None) => Message::Tick,
            Err(error) => Message::Compose(ComposeMessage::VocalAudioFailed { error }),
        },
    )
}

/// Re-run the SVS render on the *existing* MIDI clip for this vocal
/// lane, without re-deriving notes or rolling lyrics. Used when the
/// user has hand-edited notes in the vocal roll and wants to hear
/// what those edits sound like.
pub(super) fn rerender_vocal_audio(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Task<Message> {
    use resonance_audio::types::MidiNote;

    let Some(def) = r.compose.find_definition(definition_id).cloned() else {
        return Task::none();
    };
    let Some(cfg) = def.lane_generators.get(&track_id).cloned() else {
        return Task::none();
    };
    let LaneGeneratorKind::Vocal(params) = cfg.kind else {
        return Task::none();
    };

    let placements: Vec<(u64, u32)> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.id, p.start_bar))
        .collect();
    if placements.is_empty() {
        r.compose.last_error =
            Some("Place this section before re-rendering vocals.".to_string());
        return Task::none();
    }

    let derived_clip_id = placements.iter().find_map(|(pid, _)| {
        r.compose
            .derived_clips
            .get(&(definition_id, *pid, track_id))
            .copied()
    });
    let Some(clip_id) = derived_clip_id else {
        r.compose.last_error =
            Some("Generate a vocal first \u{2014} no MIDI clip to render.".to_string());
        return Task::none();
    };
    let (midi_notes, clip_name) = {
        let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
            r.compose.last_error =
                Some("Vocal MIDI clip vanished \u{2014} regenerate the melody.".to_string());
            return Task::none();
        };
        if clip.notes.is_empty() {
            r.compose.last_error = Some(
                "Vocal MIDI clip has no notes \u{2014} draw or generate before rendering."
                    .to_string(),
            );
            return Task::none();
        }
        let notes: Vec<MidiNote> = clip.notes.clone();
        (notes, clip.name.clone())
    };
    let lyrics = r
        .compose
        .vocal_audio
        .clip_lyrics
        .get(&clip_id)
        .cloned()
        .unwrap_or_else(|| vec![String::new(); midi_notes.len()]);

    let time_sig_num = r.transport.time_sig_num;
    let samples_per_bar =
        super::regenerate::compose_samples_per_bar(r.sample_rate, r.transport.bpm, time_sig_num);
    let placement_starts: Vec<(u64, u64)> = placements
        .iter()
        .map(|(pid, start_bar)| (*pid, *start_bar as u64 * samples_per_bar))
        .collect();

    enqueue_vocal_render(
        r,
        definition_id,
        track_id,
        midi_notes,
        lyrics,
        params,
        placement_starts,
        clip_name,
    )
}

/// Apply the vocal audio render result: send `LoadClipFromWav` to the
/// engine for every snapshotted placement and remember the resulting
/// clip ids (+ path) so the next regen can tear them down cleanly.
pub(super) fn handle_vocal_audio_ready(
    r: &mut crate::Resonance,
    data: crate::compose::messages::VocalAudioReadyData,
) {
    use resonance_audio::types::AudioCommand;

    let crate::compose::messages::VocalAudioReadyData {
        definition_id,
        track_id,
        wav_path,
        placements,
        clip_name,
        trim_start_frames,
        trim_end_frames,
        render_epoch,
    } = data;

    let current_epoch = r
        .compose
        .vocal_audio
        .render_epoch
        .get(&(definition_id, track_id))
        .copied()
        .unwrap_or(0);
    if render_epoch != current_epoch {
        unlink_if_exists(&wav_path);
        return;
    }

    for (placement_id, start_sample) in placements {
        if let Some((old_id, old_path)) = r
            .compose
            .vocal_audio
            .clips
            .remove(&(definition_id, placement_id, track_id))
        {
            r.engine
                .send(AudioCommand::DeleteClip { clip_id: old_id });
            unlink_if_exists(&old_path);
        }

        let audio_clip_id = r.compose.fresh_derived_clip_id();
        r.engine.send(AudioCommand::LoadClipFromWav {
            clip_id: audio_clip_id,
            track_id,
            start_sample,
            path: wav_path.clone(),
            name: clip_name.clone(),
            trim_start_frames,
            trim_end_frames,
        });
        r.compose.vocal_audio.clips.insert(
            (definition_id, placement_id, track_id),
            (audio_clip_id, wav_path.clone()),
        );
    }
}

/// Bundled inputs for installing a freshly-derived vocal MIDI clip
/// across every placement of a definition. Replaces the prior 8-arg
/// `install_vocal_midi` function — too many bare parallel arguments
/// hid a real mixed-responsibility problem.
struct VocalMidiInstall<'a> {
    definition_id: u64,
    track_id: TrackId,
    placements: &'a [(u64, u64)],
    duration_ticks: u64,
    midi_notes: &'a [resonance_audio::types::MidiNote],
    lyrics: &'a [String],
    name: &'a str,
}

impl VocalMidiInstall<'_> {
    fn install(&self, r: &mut crate::Resonance) {
        use resonance_audio::types::AudioCommand;
        for &(placement_id, start_sample) in self.placements {
            if let Some(old_id) =
                r.compose
                    .derived_clips
                    .remove(&(self.definition_id, placement_id, self.track_id))
            {
                r.engine
                    .send(AudioCommand::DeleteMidiClip { clip_id: old_id });
                r.compose.vocal_audio.clip_lyrics.remove(&old_id);
            }
            let clip_id = r.compose.fresh_derived_clip_id();
            r.engine.send(AudioCommand::LoadMidiClipDirect {
                clip_id,
                track_id: self.track_id,
                start_sample,
                duration_ticks: self.duration_ticks,
                notes: self.midi_notes.to_vec(),
                name: self.name.to_string(),
                trim_start_ticks: 0,
                trim_end_ticks: 0,
            });
            r.compose
                .derived_clips
                .insert((self.definition_id, placement_id, self.track_id), clip_id);
            let mut padded: Vec<String> = self.lyrics.to_vec();
            padded.resize(self.midi_notes.len(), String::new());
            r.compose.vocal_audio.clip_lyrics.insert(clip_id, padded);
        }
    }
}

/// Drop every previously-installed vocal audio clip on this (def, track)
/// pair from both the engine and disk. Run before the new audio is
/// installed so we don't leak WAV files.
///
/// On Linux it's safe to `unlink` a file the engine still has mmap'd —
/// the kernel keeps the inode alive until the mapping is dropped and
/// reclaims the disk space then.
fn tear_down_old_vocal_audio(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) {
    use resonance_audio::types::{AudioCommand, ClipId};
    type VocalAudioKey = (u64, u64, TrackId);
    type VocalAudioEntry = (ClipId, std::path::PathBuf);
    let stale: Vec<(VocalAudioKey, VocalAudioEntry)> = r
        .compose
        .vocal_audio
        .clips
        .iter()
        .filter(|((d, _p, t), _)| *d == definition_id && *t == track_id)
        .map(|(k, v)| (*k, v.clone()))
        .collect();
    for (key, (clip_id, path)) in stale {
        r.engine.send(AudioCommand::DeleteClip { clip_id });
        unlink_if_exists(&path);
        r.compose.vocal_audio.clips.remove(&key);
    }
}

/// Best-effort file delete. Missing files (e.g. a previous render
/// failed to write or was already cleaned up) are silently ignored;
/// any other error is surfaced via stderr but does not fail the regen.
fn unlink_if_exists(path: &std::path::Path) {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("[vocal] unlink {}: {e}", path.display()),
    }
}

/// Destination directory for rendered vocal WAVs. Prefers the loaded
/// project's `audio/` subdirectory so saves capture the clip; falls
/// back to a per-process temp dir for unsaved sessions.
fn vocal_audio_dir(r: &crate::Resonance) -> std::path::PathBuf {
    r.io
        .project_path
        .as_ref()
        .and_then(|p| p.parent().map(|d| d.join("audio")))
        .unwrap_or_else(|| std::env::temp_dir().join("resonance_vocal"))
}

/// Off-thread render entry point. Runs the SVS pipeline + writes the WAV.
/// Returns `Ok(None)` when the SVS model dir isn't installed (silent
/// fallback to MIDI-only mode), `Ok(Some(path))` on success.
fn render_vocal_wav(
    midi_notes: &[resonance_audio::types::MidiNote],
    params: &VocalParams,
    lyrics: &[String],
    bpm: f32,
    engine_sample_rate: u32,
    dest_dir: &std::path::Path,
) -> Result<Option<(std::path::PathBuf, u64, u64)>, String> {
    use crate::compose::vocal_svs;
    use resonance_audio::types::TICKS_PER_QUARTER_NOTE;

    let rendered = match vocal_svs::render_vocal_clip_with_lyrics(
        midi_notes,
        params,
        lyrics,
        TICKS_PER_QUARTER_NOTE as u32,
        bpm,
        engine_sample_rate,
    ) {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(None),
        Err(e) => return Err(format!("SVS render: {e}")),
    };

    let filename = format!(
        "vocal_{}.wav",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let path = dest_dir.join(filename);
    vocal_svs::write_stereo_wav(&path, &rendered.samples_stereo, rendered.sample_rate)
        .map_err(|e| format!("write WAV {}: {e}", path.display()))?;
    Ok(Some((path, rendered.trim_start_frames, rendered.trim_end_frames)))
}
