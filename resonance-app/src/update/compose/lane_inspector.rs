//! Handlers for the per-track lane inspector messages: switch generator
//! kind, edit Bass/Melody/Pad/Drum parameters, and regenerate just this
//! lane (which bumps the lane's own seed — section-shared motif identity
//! is only touched by the chord-inspector's Regenerate motif button).

use iced::Task;

use resonance_audio::types::TrackId;
use resonance_music_theory::{BassParams, MelodyParams, PadParams, VocalParams};

use super::regenerate::regenerate_lane;
use crate::compose::messages::LaneInspectorMsg;
use crate::compose::{
    ComposeMessage, LaneGeneratorConfig, LaneGeneratorKind, LaneGeneratorKindTag,
};
use crate::message::Message;

pub(super) fn handle(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    msg: LaneInspectorMsg,
) -> Task<Message> {
    match msg {
        LaneInspectorMsg::SetGenerator(tag) => {
            if let Some(def) = r.compose.find_definition_mut(definition_id) {
                match tag {
                    LaneGeneratorKindTag::Manual => {
                        def.lane_generators.remove(&track_id);
                    }
                    LaneGeneratorKindTag::Bass => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Bass(BassParams::default()),
                                seed: definition_id.wrapping_mul(0x9E3779B97F4A7C15),
                            },
                        );
                    }
                    LaneGeneratorKindTag::Melody => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Melody(MelodyParams::default()),
                                seed: definition_id.wrapping_mul(0x517CC1B727220A95),
                            },
                        );
                    }
                    LaneGeneratorKindTag::Pad => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Pad(PadParams::default()),
                                seed: definition_id.wrapping_mul(0x6C62272E07BB0142),
                            },
                        );
                    }
                    LaneGeneratorKindTag::Vocal => {
                        def.lane_generators.insert(
                            track_id,
                            LaneGeneratorConfig {
                                kind: LaneGeneratorKind::Vocal(VocalParams::default()),
                                seed: definition_id.wrapping_mul(0xBF58476D1CE4E5B9),
                            },
                        );
                    }
                }
                r.compose.last_error = None;
            }
        }

        // Bass parameter updates
        LaneInspectorMsg::SetBassStyle(style) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.style = style;
                }
            });
        }
        LaneInspectorMsg::SetBassBaseNote(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.base_note = note;
                }
            });
        }
        LaneInspectorMsg::SetBassVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.velocity = v;
                }
            });
        }
        LaneInspectorMsg::SetBassMotifMode(mode) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.motif_mode = mode;
                }
            });
        }
        LaneInspectorMsg::SetBassMotifPhrase(phrase) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Bass(p) = kind {
                    p.motif_phrase = phrase;
                }
            });
        }

        // Melody parameter updates
        LaneInspectorMsg::SetMelodyStyle(style) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.style = style;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRegisterLow(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.register.0 = note;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRegisterHigh(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.register.1 = note;
                }
            });
        }
        LaneInspectorMsg::SetMelodyNoteValue(ticks) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.note_value_ticks = ticks;
                }
            });
        }
        LaneInspectorMsg::SetMelodyRestDensity(d) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.rest_density = d;
                }
            });
        }
        LaneInspectorMsg::SetMelodyVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.velocity = v;
                }
            });
        }
        LaneInspectorMsg::SetMelodyArticulation(a) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.articulation = a;
                }
            });
        }
        LaneInspectorMsg::SetMelodyContour(contour) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.contour = contour;
                }
            });
        }
        LaneInspectorMsg::SetMelodyPhraseLen(len) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Melody(p) = kind {
                    p.phrase_len = len;
                }
            });
        }

        // Pad parameter updates
        LaneInspectorMsg::SetPadRegisterLow(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.register.0 = note;
                }
            });
        }
        LaneInspectorMsg::SetPadRegisterHigh(note) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.register.1 = note;
                }
            });
        }
        LaneInspectorMsg::SetPadVelocity(v) => {
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Pad(p) = kind {
                    p.velocity = v;
                }
            });
        }

        // ------------------------------------------------------------------
        // Vocal lyrics
        // ------------------------------------------------------------------
        LaneInspectorMsg::SetVocalTheme(text) => {
            update_vocal(r, definition_id, track_id, |p| {
                // Mirror the prototype's 240-char cap.
                p.theme = text.chars().take(240).collect();
            });
        }
        LaneInspectorMsg::SetVocalMood(m) => {
            update_vocal(r, definition_id, track_id, |p| p.mood = m);
        }
        LaneInspectorMsg::SetVocalPov(pov) => {
            update_vocal(r, definition_id, track_id, |p| p.pov = pov);
        }
        LaneInspectorMsg::SetVocalRhyme(rhyme) => {
            update_vocal(r, definition_id, track_id, |p| p.rhyme = rhyme);
        }
        LaneInspectorMsg::SetVocalLines(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.lines = n.clamp(1, 16);
            });
        }
        LaneInspectorMsg::SetVocalSyllablesMin(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.syllables_min = n.clamp(1, p.syllables_max.max(1));
            });
        }
        LaneInspectorMsg::SetVocalSyllablesMax(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.syllables_max = n.clamp(p.syllables_min, 24);
            });
        }
        LaneInspectorMsg::ToggleVocalMatchSyllables => {
            update_vocal(r, definition_id, track_id, |p| {
                p.match_syllables_to_melody = !p.match_syllables_to_melody;
            });
        }
        LaneInspectorMsg::ToggleVocalAvoidCliches => {
            update_vocal(r, definition_id, track_id, |p| {
                p.avoid_cliches = !p.avoid_cliches;
            });
        }
        LaneInspectorMsg::ToggleVocalLockLine(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                if let Some(line) = p.draft.iter_mut().find(|l| l.n == n) {
                    line.locked = !line.locked;
                }
            });
        }
        LaneInspectorMsg::RerollUnlockedLyrics => {
            roll_vocal_lyrics(r, definition_id, track_id, 0x9E3779B97F4A7C15);
        }

        // ------------------------------------------------------------------
        // Vocal melody
        // ------------------------------------------------------------------
        LaneInspectorMsg::SetVocalVoiceType(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.voice = v;
                p.range = v.default_range();
            });
        }
        LaneInspectorMsg::SetVocalRangeLow(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.range.0 = n.min(p.range.1);
            });
        }
        LaneInspectorMsg::SetVocalRangeHigh(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.range.1 = n.max(p.range.0);
            });
        }
        LaneInspectorMsg::SetVocalContour(c) => {
            update_vocal(r, definition_id, track_id, |p| p.contour = c);
        }
        LaneInspectorMsg::SetVocalSyllableMode(m) => {
            update_vocal(r, definition_id, track_id, |p| p.syllable_mode = m);
        }
        LaneInspectorMsg::SetVocalChordToneAnchor(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.chord_tone_anchor = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalLeapRange(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.leap_range = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalPhraseLength(n) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.phrase_length_bars = n.clamp(1, 8);
            });
        }
        LaneInspectorMsg::SetVocalBreath(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.breath = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::ToggleVocalStayInScale => {
            update_vocal(r, definition_id, track_id, |p| {
                p.stay_in_scale = !p.stay_in_scale;
            });
        }
        LaneInspectorMsg::ToggleVocalAvoidClashes => {
            update_vocal(r, definition_id, track_id, |p| {
                p.avoid_clashes = !p.avoid_clashes;
            });
        }

        // ------------------------------------------------------------------
        // Vocal voice & delivery
        // ------------------------------------------------------------------
        LaneInspectorMsg::SetVocalTimbre(t) => {
            update_vocal(r, definition_id, track_id, |p| p.timbre = t);
        }
        LaneInspectorMsg::SetVocalVibrato(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.vibrato = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalArticulation(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.articulation = v.clamp(0.0, 1.0);
            });
        }
        LaneInspectorMsg::SetVocalConsonantEmphasis(v) => {
            update_vocal(r, definition_id, track_id, |p| {
                p.consonant_emphasis = v.clamp(0.0, 1.0);
            });
        }

        // ------------------------------------------------------------------
        // Vocal generate actions
        // ------------------------------------------------------------------
        LaneInspectorMsg::GenerateVocalLyricsOnly => {
            roll_vocal_lyrics(r, definition_id, track_id, 0xBF58476D1CE4E5B9);
        }
        LaneInspectorMsg::GenerateVocalMelodyOnly => {
            bump_lane_seed(r, definition_id, track_id, 0x94D049BB133111EB);
            return roll_vocal_melody(r, definition_id, track_id);
        }
        LaneInspectorMsg::GenerateVocalAll => {
            roll_vocal_lyrics(r, definition_id, track_id, 0xBF58476D1CE4E5B9);
            bump_lane_seed(r, definition_id, track_id, 0xBF58476D1CE4E5B9);
            return roll_vocal_melody(r, definition_id, track_id);
        }

        // Drum voice mode
        LaneInspectorMsg::SetDrumVoiceMode { pad_index, mode } => {
            ensure_drum_config(r, definition_id, track_id);
            update_lane_gen(r, definition_id, track_id, |kind| {
                if let LaneGeneratorKind::Drum(dc) = kind {
                    dc.voices.insert(pad_index, mode);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidSteps { pad_index, steps } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { steps: s, .. } = mode {
                    *s = steps.max(1);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidHits { pad_index, hits } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { hits: h, steps, .. } = mode {
                    *h = hits.min(*steps);
                }
            });
        }
        LaneInspectorMsg::SetDrumEuclidRotation {
            pad_index,
            rotation,
        } => {
            update_drum_voice(r, definition_id, track_id, pad_index, |mode| {
                if let crate::compose::DrumVoiceMode::Euclidean { rotation: rot, .. } = mode {
                    *rot = rotation;
                }
            });
        }

        LaneInspectorMsg::Regenerate => {
            // Bump the lane's own seed and re-derive only this lane. For Motif
            // lanes this varies the per-lane surface (phrase contours, rest
            // density holes) without touching the section-shared motif —
            // those identity bits only change via the chord inspector's
            // "Regenerate motif" button.
            bump_lane_seed(r, definition_id, track_id, 0x9E3779B97F4A7C15);
            return regenerate_lane(r, definition_id, track_id);
        }
    }
    Task::none()
}

/// Bump a lane's seed by `seed_mix + 1`. Centralised so the vocal regen
/// path (which dispatches through `regenerate_lane`) can reuse the same
/// seed without double-bumping in `roll_vocal_melody`.
fn bump_lane_seed(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    seed_mix: u64,
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            cfg.seed = cfg.seed.wrapping_add(seed_mix).wrapping_add(1);
        }
    }
}

/// Mutate a lane's generator kind in-place.
fn update_lane_gen(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl FnOnce(&mut LaneGeneratorKind),
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            f(&mut cfg.kind);
        }
        r.compose.last_error = None;
    }
}

/// Ensure a drum lane config exists for the given track.
fn ensure_drum_config(r: &mut crate::Resonance, definition_id: u64, track_id: TrackId) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        def.lane_generators
            .entry(track_id)
            .or_insert_with(|| LaneGeneratorConfig {
                kind: LaneGeneratorKind::Drum(crate::compose::DrumLaneConfig::default()),
                seed: 0,
            });
    }
}

/// Mutate the vocal params of a lane in-place. Skips silently when the
/// lane has a different generator kind installed.
fn update_vocal(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    f: impl FnOnce(&mut VocalParams),
) {
    update_lane_gen(r, definition_id, track_id, |kind| {
        if let LaneGeneratorKind::Vocal(p) = kind {
            f(p);
        }
    });
}

/// Roll a fresh lyric draft for the vocal lane. Bumps the seed first so
/// repeated presses don't produce the same draft. Locked lines stay put
/// — `generate_lyrics` preserves them and anchors the rhyme pattern to
/// their bucket.
fn roll_vocal_lyrics(
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
/// random surface must call [`bump_lane_seed`] beforehand. This split
/// avoids the previous double-bump where `Regenerate → regenerate_lane
/// → roll_vocal_melody` all bumped the seed in turn.
pub(super) fn roll_vocal_melody(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> Task<Message> {
    use crate::compose::messages::VocalAudioReadyData;
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
    let notes = resonance_music_theory::derive_vocal(
        &timed,
        &params,
        TICKS_PER_QUARTER_NOTE as u32,
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

    // Install the MIDI clip + tear down any prior audio clip on every
    // placement. The audio clip will be re-installed when the background
    // render finishes (or skipped silently when SVS models aren't
    // available).
    let placement_starts: Vec<(u64, u64)> = placements
        .iter()
        .map(|(pid, start_bar)| (*pid, *start_bar as u64 * samples_per_bar))
        .collect();
    install_vocal_midi(r, definition_id, track_id, &placement_starts, duration_ticks, &midi_notes, &name);
    tear_down_old_vocal_audio(r, definition_id, track_id);

    // Bump the in-flight epoch for this (def, track) so the completion
    // handler can detect and discard stale renders. Without this, two
    // back-to-back regen presses would both install their audio clips
    // (the second tear-down can't see the first's still-in-flight
    // clip because it isn't in `vocal_audio_clips` yet) and the mixer
    // would sum both, producing the doubled-voice distortion the user
    // reported.
    let epoch_entry = r
        .compose
        .vocal_render_epoch
        .entry((definition_id, track_id))
        .or_insert(0);
    *epoch_entry = epoch_entry.wrapping_add(1);
    let render_epoch = *epoch_entry;

    r.compose.last_error = None;

    // Spawn the SVS render off the UI thread. `Task::perform` schedules
    // the async block on iced's executor; `tokio::task::spawn_blocking`
    // hops onto a dedicated blocking thread so the diffusion model
    // doesn't stall the executor.
    let bpm = r.transport.bpm;
    let engine_sr = r.sample_rate;
    let dest_dir = vocal_audio_dir(r);
    let clip_name = name.clone();
    Task::perform(
        async move {
            tokio::task::spawn_blocking(move || {
                render_vocal_wav(&midi_notes, &params, bpm, engine_sr, &dest_dir)
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
            // SVS models missing: silent — MIDI-only mode still works.
            Ok(None) => Message::Tick,
            Err(error) => Message::Compose(ComposeMessage::VocalAudioFailed { error }),
        },
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

    // Stale render — a newer regen was queued while this one was
    // grinding through the diffusion model. Drop the result on the
    // floor (and unlink the orphan WAV) so we don't stack audio clips.
    let current_epoch = r
        .compose
        .vocal_render_epoch
        .get(&(definition_id, track_id))
        .copied()
        .unwrap_or(0);
    if render_epoch != current_epoch {
        unlink_if_exists(&wav_path);
        return;
    }

    for (placement_id, start_sample) in placements {
        // Tear down any prior audio clip on this placement (handles the
        // case where two renders raced). `tear_down_old_vocal_audio`
        // already cleared everything for the (def, track) pair, so this
        // is just defensive.
        if let Some((old_id, old_path)) = r
            .compose
            .vocal_audio_clips
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
        r.compose.vocal_audio_clips.insert(
            (definition_id, placement_id, track_id),
            (audio_clip_id, wav_path.clone()),
        );
    }
}

/// Send `LoadMidiClipDirect` for every placement, replacing any prior
/// derived MIDI clip. Stays sync — the staff has to update right away.
fn install_vocal_midi(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    placements: &[(u64, u64)],
    duration_ticks: u64,
    midi_notes: &[resonance_audio::types::MidiNote],
    name: &str,
) {
    use resonance_audio::types::AudioCommand;
    for &(placement_id, start_sample) in placements {
        if let Some(old_id) =
            r.compose
                .derived_clips
                .remove(&(definition_id, placement_id, track_id))
        {
            r.engine
                .send(AudioCommand::DeleteMidiClip { clip_id: old_id });
        }
        let clip_id = r.compose.fresh_derived_clip_id();
        r.engine.send(AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes: midi_notes.to_vec(),
            name: name.to_string(),
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        });
        r.compose
            .derived_clips
            .insert((definition_id, placement_id, track_id), clip_id);
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
    use resonance_audio::types::AudioCommand;
    let stale: Vec<((u64, u64, TrackId), (resonance_audio::types::ClipId, std::path::PathBuf))> = r
        .compose
        .vocal_audio_clips
        .iter()
        .filter(|((d, _p, t), _)| *d == definition_id && *t == track_id)
        .map(|(k, v)| (*k, v.clone()))
        .collect();
    for (key, (clip_id, path)) in stale {
        r.engine.send(AudioCommand::DeleteClip { clip_id });
        unlink_if_exists(&path);
        r.compose.vocal_audio_clips.remove(&key);
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
///
/// `engine_sample_rate` is the audio device's output rate; the SVS
/// model runs at its own fixed rate (44.1 kHz on TIGER) and the
/// renderer resamples to match so the mixer's frame-for-frame playback
/// doesn't pitch-shift the audio.
fn render_vocal_wav(
    midi_notes: &[resonance_audio::types::MidiNote],
    params: &VocalParams,
    bpm: f32,
    engine_sample_rate: u32,
    dest_dir: &std::path::Path,
) -> Result<Option<(std::path::PathBuf, u64, u64)>, String> {
    use crate::compose::vocal_svs;
    use resonance_audio::types::TICKS_PER_QUARTER_NOTE;

    let rendered = match vocal_svs::render_vocal_clip(
        midi_notes,
        params,
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

/// Mutate a specific drum voice's mode in-place.
fn update_drum_voice(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    pad_index: usize,
    f: impl FnOnce(&mut crate::compose::DrumVoiceMode),
) {
    if let Some(def) = r.compose.find_definition_mut(definition_id) {
        if let Some(cfg) = def.lane_generators.get_mut(&track_id) {
            if let LaneGeneratorKind::Drum(dc) = &mut cfg.kind {
                if let Some(mode) = dc.voices.get_mut(&pad_index) {
                    f(mode);
                }
            }
        }
        r.compose.last_error = None;
    }
}
