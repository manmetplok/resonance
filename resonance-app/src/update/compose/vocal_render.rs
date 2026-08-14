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
    cfg.seed = crate::util::bump_seed(cfg.seed, seed_mix);
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

    let Some(mut def) = r.compose.find_definition(definition_id).cloned() else {
        return Task::none();
    };
    let Some(cfg) = def.lane_generators.get(&track_id).cloned() else {
        return Task::none();
    };
    let LaneGeneratorKind::Vocal(params) = cfg.kind else {
        return Task::none();
    };
    if let Err(e) = params.validate() {
        r.compose.last_error = Some(format!("Vocal params invalid: {e}"));
        return Task::none();
    }
    if def.chords.is_empty() || params.draft.is_empty() {
        return Task::none();
    }

    // Constrain the vocal melody to the user's pinned chord-track harmony
    // (doc #168, todo #445), mirroring the instrument lanes in
    // `regenerate_lane`.
    super::regenerate::apply_chord_track_harmony(r, definition_id, &mut def);

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
        .map(|(pid, start_bar)| (*pid, r.tempo_map.bar_to_sample(*start_bar)))
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
        // A fresh roll has no edited clip yet, so no per-syllable
        // overrides apply — only the project dictionary + auto path.
        None,
    )
}

// ---------------------------------------------------------------------------
// Control endpoint (ba doc #265, todo #1156)
// ---------------------------------------------------------------------------

/// Whether a track has a vocal lane in any section — used by the control
/// endpoint to turn a "no lane" case into a precise error before it
/// synthesizes a mutating message.
pub(crate) fn track_has_vocal_lane(r: &crate::Resonance, track_id: TrackId) -> bool {
    r.compose.definitions.iter().any(|d| {
        matches!(
            d.lane_generators.get(&track_id).map(|c| &c.kind),
            Some(LaneGeneratorKind::Vocal(_))
        )
    })
}

/// Every section definition whose `track_id` lane is a vocal generator,
/// placed lanes first in placement order (ties broken by definition id
/// so the order is stable), then unplaced ones in creation order. A
/// definition placed several times appears once — a lane renders once
/// and its audio fans out to every placement.
///
/// This is the whole track: `vocal.render` with no `section_id` renders
/// all of it. Addressing only the head of this list is what left every
/// lane but the first frozen at its previous audio (ba doc #271 V2).
pub(crate) fn vocal_definitions_for_track(
    r: &crate::Resonance,
    track_id: TrackId,
) -> Vec<u64> {
    let is_vocal_lane = |definition_id: u64| {
        matches!(
            r.compose
                .find_definition(definition_id)
                .and_then(|d| d.lane_generators.get(&track_id))
                .map(|c| &c.kind),
            Some(LaneGeneratorKind::Vocal(_))
        )
    };

    let mut placed: Vec<(u32, u64)> = r
        .compose
        .placements
        .iter()
        .filter(|p| is_vocal_lane(p.definition_id))
        .map(|p| (p.start_bar, p.definition_id))
        .collect();
    placed.sort_by_key(|&(bar, def)| (bar, def));

    let mut out: Vec<u64> = Vec::new();
    for (_, def) in placed {
        if !out.contains(&def) {
            out.push(def);
        }
    }
    for def in &r.compose.definitions {
        if is_vocal_lane(def.id) && !out.contains(&def.id) {
            out.push(def.id);
        }
    }
    out
}

/// Every `(definition_id, track_id)` vocal lane in the project, ordered
/// by placement then track id. Backs a `vocal.render` that names no
/// track at all — "every vocal track".
pub(crate) fn all_vocal_lanes(r: &crate::Resonance) -> Vec<(u64, TrackId)> {
    let mut tracks: Vec<TrackId> = Vec::new();
    for def in &r.compose.definitions {
        for (track_id, cfg) in &def.lane_generators {
            if matches!(cfg.kind, LaneGeneratorKind::Vocal(_)) && !tracks.contains(track_id) {
                tracks.push(*track_id);
            }
        }
    }
    tracks.sort_unstable();
    tracks
        .into_iter()
        .flat_map(|track_id| {
            vocal_definitions_for_track(r, track_id)
                .into_iter()
                .map(move |def| (def, track_id))
        })
        .collect()
}

/// The first section (in placement order, then creation order) whose
/// `track_id` lane is a vocal generator. The control lyric methods
/// address a track; this picks the lane they act on when the caller
/// didn't (couldn't) name a section.
pub(crate) fn first_vocal_definition(r: &crate::Resonance, track_id: TrackId) -> Option<u64> {
    vocal_definitions_for_track(r, track_id).first().copied()
}

/// `vocal.set_lyrics`: replace the lane's whole draft from bulk text.
pub(crate) fn control_set_lyrics(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    text: &str,
) {
    super::lane_inspector::update_vocal(r, definition_id, track_id, |p| {
        super::vocal_lyrics::rebuild_draft_from_bulk(p, text);
    });
    super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
}

/// `vocal.set_line`: replace one 0-based lyric line. Returns `false`
/// (leaving state untouched) when the index is out of range.
pub(crate) fn control_set_line(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    line_index: usize,
    text: &str,
) -> bool {
    let in_range = r
        .compose
        .find_definition(definition_id)
        .and_then(|d| d.lane_generators.get(&track_id))
        .and_then(|c| match &c.kind {
            LaneGeneratorKind::Vocal(p) => Some(line_index < p.draft.len()),
            _ => None,
        })
        .unwrap_or(false);
    if !in_range {
        return false;
    }
    super::lane_inspector::update_vocal(r, definition_id, track_id, |p| {
        if let Some(line) = p.draft.get_mut(line_index) {
            line.text = text.to_owned();
            line.syllables =
                resonance_music_theory::count_syllables(text).min(255) as u8;
            // A hand-set line is locked so a later re-roll preserves it,
            // matching the per-line editor's behaviour.
            line.locked = true;
        }
    });
    super::vocal_lyrics::sync_bulk_lyrics_from_draft(r, definition_id, track_id);
    true
}

/// `vocal.render`: set the lane's voicebank and synthesise the notes
/// currently in the lane's MIDI clip. Returns the async render `Task`
/// (or `Task::none()` when the lane can't render yet — the caller has
/// already validated the lane exists; a `Task::none()` here means no
/// generated clip / no notes, surfaced as `compose.last_error`).
///
/// **Renders, never generates.** This drives `rerender_vocal_audio`
/// (notes-only), not `roll_vocal_melody` (full regenerate) — the same
/// split the GUI exposes as "Re-render audio" versus "Generate". Wired
/// to the generate path, `vocal.render` silently discarded whatever the
/// client had authored into the clip and re-derived a melody from the
/// lane's seed, so three different authored note sets rendered to
/// byte-identical audio while every call reported success (ba doc #271
/// V1). Melody generation belongs to `vocal.generate`, which exists for
/// exactly that (doc #269 FR-2).
///
/// Lyrics still come from the lane's live draft: `enqueue_vocal_render`
/// resolves them out of `VocalParams::draft`, and the clip's
/// `clip_lyrics` entry is only a per-note annotation overlay — so a
/// `vocal.set_lyrics` between generate and render is picked up here.
pub(crate) fn control_render(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    voicebank: resonance_music_theory::VocalVoicebank,
) -> Task<Message> {
    super::lane_inspector::update_vocal(r, definition_id, track_id, |p| {
        p.voicebank = voicebank;
    });
    rerender_vocal_audio(r, definition_id, track_id)
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
    clip_id: Option<resonance_audio::types::ClipId>,
) -> Task<Message> {
    use crate::compose::messages::VocalAudioReadyData;

    // Resolve pronunciation (override > project-dict > global-dict >
    // CMU-auto) and gate every phoneme through the active voicebank
    // *before* touching the existing audio. A blocked phoneme aborts the
    // render with a report and leaves the current clip intact rather than
    // corrupting the segment. (#494)
    let assigned = match resolve_and_validate(r, &params, &lyrics, midi_notes.len(), clip_id) {
        Ok(assigned) => assigned,
        Err(message) => {
            r.compose.last_error = Some(message);
            return Task::none();
        }
    };

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

    // Per-clip content-addressed render cache: an edit only re-renders the
    // sub-clip segments it touched. Shared into the blocking render thread
    // via `Arc<Mutex>`; the cache's `last_plan` is read back afterwards for
    // the "N of M segments changed" overlay (#495).
    let render_cache = r
        .compose
        .vocal_audio
        .render_cache
        .entry((definition_id, track_id))
        .or_default()
        .clone();

    let bpm = r.transport.bpm;
    let engine_sr = r.sample_rate;

    // Where the rendered audio actually goes on the timeline — the
    // section start advanced by the lane's first note. See
    // [`vocal_audio_start`] for why the offset is needed at all (ba doc
    // #272 V-1). Only the audio moves: the lane's MIDI clip still starts
    // at the section boundary and carries its own per-note ticks.
    let lead_ticks = midi_notes.first().map(|n| n.start_tick).unwrap_or(0);
    let audio_starts: Vec<(u64, u64)> = placement_starts
        .iter()
        .map(|&(placement_id, section_start)| {
            (
                placement_id,
                crate::compose::vocal_svs::vocal_audio_start(
                    &r.tempo_map,
                    section_start,
                    lead_ticks,
                    engine_sr,
                ),
            )
        })
        .collect();

    let dest_dir = vocal_audio_dir(r);
    // The lane's editable expression overlay (dynamics/tension/breathiness/
    // pitch bend, doc #154). Cloned out of compose state before the render
    // moves off-thread; an un-edited lane has no entry, so a default
    // (all-`Auto`) bundle reproduces the pre-overlay audio exactly.
    let curves = r
        .compose
        .expression_curves(definition_id, track_id)
        .cloned()
        .unwrap_or_default();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                render_vocal_wav(
                    &midi_notes,
                    &params,
                    &assigned,
                    &curves,
                    bpm,
                    engine_sr,
                    &dest_dir,
                    &render_cache,
                )
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
                    placements: audio_starts.clone(),
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

    // Prefer the derived-clip mapping, but fall back to matching a clip by
    // the placement's start sample. The mapping is not always populated for
    // a lane loaded from disk, and without this fallback such a lane is
    // permanently unrenderable while `song.vocal` cheerfully reports it as
    // having notes — the two must agree. `song.vocal`'s `lane_clip` and the
    // `lane_has_notes` pre-flight resolve it the same way.
    let derived_clip_id = placements
        .iter()
        .find_map(|(pid, _)| {
            r.compose
                .derived_clips
                .get(&(definition_id, *pid, track_id))
                .copied()
        })
        .or_else(|| {
            placements.iter().find_map(|(_, start_bar)| {
                let start = r.tempo_map.bar_to_sample(*start_bar);
                r.midi_clips
                    .iter()
                    .find(|c| {
                        c.track_id == track_id
                            && c.start_sample == start
                            // Never adopt a clip another lane already owns.
                            && !r.compose.derived_clips.values().any(|id| *id == c.id)
                    })
                    .map(|c| c.id)
            })
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

    let placement_starts: Vec<(u64, u64)> = placements
        .iter()
        .map(|(pid, start_bar)| (*pid, r.tempo_map.bar_to_sample(*start_bar)))
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
        // Re-rendering an existing clip: its per-syllable pronunciation
        // overrides (if any) apply on top of the dictionary + auto path.
        Some(clip_id),
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
            let _ = r.engine
                .send(AudioCommand::DeleteClip { clip_id: old_id });
            unlink_if_exists(&old_path);
        }

        let audio_clip_id = r.compose.fresh_derived_clip_id();
        let _ = r.engine.send(AudioCommand::LoadClipFromWav {
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
                let _ = r.engine
                    .send(AudioCommand::DeleteMidiClip { clip_id: old_id });
                r.compose.vocal_audio.clip_lyrics.remove(&old_id);
                r.midi_clips.retain(|c| c.id != old_id);
            }
            let clip_id = r.compose.fresh_derived_clip_id();
            let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
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
            // Mirror the clip into `r.midi_clips` synchronously, the way
            // `notes.create_clip` does (ba todo #1162): the control
            // endpoint's `vocal.generate` returns this clip_id and a
            // follow-up `song.notes` / `notes.*` resolves the target
            // through `r.midi_clips`, which would otherwise still be
            // racing the async `MidiClipCreated` echo. That echo's
            // `clip_created` handler skips ids already present, so the
            // round trip stays a no-op once it lands.
            if !r.midi_clips.iter().any(|c| c.id == clip_id) {
                r.midi_clips.push(crate::state::MidiClipState {
                    id: clip_id,
                    track_id: self.track_id,
                    start_sample,
                    duration_ticks: self.duration_ticks,
                    name: self.name.to_string(),
                    notes: self.midi_notes.to_vec(),
                    trim_start_ticks: 0,
                    trim_end_ticks: 0,
                });
            }
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
        let _ = r.engine.send(AudioCommand::DeleteClip { clip_id });
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

/// Resolve the clip's per-note pronunciation and gate every phoneme
/// through the active voicebank, returning the substituted syllable
/// stream ready for [`render_vocal_wav`] or a user-facing error listing
/// the syllables that can't be sung. (#494)
///
/// Precedence is `override > project-dict > global-dict > CMU-auto`: the
/// clip's overrides (only present for a re-render of an edited clip) win,
/// then the project dictionary. The global (user-config) dictionary has
/// no on-disk store yet — a later todo wires it; until then it is empty.
fn resolve_and_validate(
    r: &crate::Resonance,
    params: &VocalParams,
    annotations: &[String],
    note_count: usize,
    clip_id: Option<resonance_audio::types::ClipId>,
) -> Result<Vec<resonance_music_theory::g2p::AssignedSyllable>, String> {
    use crate::compose::vocal_svs;

    let empty = std::collections::HashMap::new();
    let overrides = clip_id
        .and_then(|c| r.compose.pronunciation.clip_overrides(c))
        .unwrap_or(&empty);
    let resolved = vocal_svs::resolve_clip_pronunciation(
        &params.draft,
        annotations,
        note_count,
        overrides,
        &r.compose.pronunciation.project_dictionary,
        &[],
    );
    vocal_svs::validate_for_voicebank(&resolved, params.voicebank).map_err(|invalid| describe_invalid(&invalid))
}

/// Render the blocked-phoneme report into a single status-bar line. Caps
/// the number of syllables spelled out so a wholesale-bad draft doesn't
/// produce a wall of text.
fn describe_invalid(invalid: &[crate::compose::vocal_svs::InvalidSyllable]) -> String {
    const MAX_SHOWN: usize = 6;
    let shown = invalid.len().min(MAX_SHOWN);
    let mut parts: Vec<String> = invalid
        .iter()
        .take(shown)
        .map(|s| {
            let label = if s.label.is_empty() {
                "?".to_string()
            } else {
                s.label.clone()
            };
            format!(
                "note {} \u{201c}{}\u{201d}: {} ({})",
                s.note_index + 1,
                label,
                s.phoneme,
                s.reason.as_str()
            )
        })
        .collect();
    if invalid.len() > shown {
        parts.push(format!("+{} more", invalid.len() - shown));
    }
    format!(
        "Can\u{2019}t render vocals \u{2014} {} phoneme(s) the voicebank can\u{2019}t sing: {}",
        invalid.len(),
        parts.join("; ")
    )
}

/// Off-thread render entry point. Runs the SVS pipeline + writes the WAV.
/// Returns `Ok(None)` when the SVS model dir isn't installed (silent
/// fallback to MIDI-only mode), `Ok(Some(path))` on success.
#[allow(clippy::too_many_arguments)]
fn render_vocal_wav(
    midi_notes: &[resonance_audio::types::MidiNote],
    params: &VocalParams,
    assigned: &[resonance_music_theory::g2p::AssignedSyllable],
    curves: &crate::compose::ExpressionCurves,
    bpm: f32,
    engine_sample_rate: u32,
    dest_dir: &std::path::Path,
    render_cache: &std::sync::Mutex<crate::compose::vocal_svs::SvsRenderCache>,
) -> Result<Option<(std::path::PathBuf, u64, u64)>, String> {
    use crate::compose::vocal_svs;
    use resonance_audio::types::TICKS_PER_QUARTER_NOTE;

    let mut cache = render_cache
        .lock()
        .map_err(|_| "vocal render cache poisoned".to_string())?;
    let rendered = match vocal_svs::render_vocal_clip(
        midi_notes,
        params,
        assigned,
        curves,
        TICKS_PER_QUARTER_NOTE as u32,
        bpm,
        engine_sample_rate,
        &mut cache,
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
