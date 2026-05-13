//! Lane regeneration: turn a section's chord progression + per-lane
//! generator config into MIDI clips on every placement of the section,
//! plus the cascade helpers that fan a single chord change or motif
//! seed bump out across every motif/chord-reading lane in the section.

use resonance_audio::types::{AudioCommand, MidiNote, TrackId, TICKS_PER_QUARTER_NOTE};

use crate::compose::{generate, DeriveKind, DrumVoiceMode, LaneGeneratorKind};
use crate::update::drumroll::euclid_edits_for;

/// When the section's shared motif changes, re-derive every Motif-style
/// lane in the section. Other lanes (RootHold, Walking, ArpUp, etc.) are
/// untouched because they don't read from `def.motif`. Drum lanes get a
/// second pass: each pad in `Motif` mode has its hits replaced in place
/// without disturbing other pads on the same clip.
pub(super) fn propagate_motif_change(r: &mut crate::Resonance, definition_id: u64) {
    let melodic_tracks: Vec<TrackId> = r
        .compose
        .find_definition(definition_id)
        .map(|def| {
            def.lane_generators
                .iter()
                .filter(|(_, cfg)| match &cfg.kind {
                    LaneGeneratorKind::Bass(p) => {
                        p.style == resonance_music_theory::BassStyle::Motif
                    }
                    LaneGeneratorKind::Melody(p) => {
                        p.style == resonance_music_theory::MelodyStyle::Motif
                    }
                    _ => false,
                })
                .map(|(tid, _)| *tid)
                .collect()
        })
        .unwrap_or_default();

    for tid in melodic_tracks {
        // Motif propagation doesn't have a Task return channel — vocal
        // audio render is dropped here, MIDI still updates inline.
        let _ = regenerate_lane(r, definition_id, tid);
    }

    // Drum lanes: snapshot the (track_id, pad_index) pairs that are in
    // Motif mode, then replay the rhythm onto every overlapping clip.
    let drum_targets: Vec<(TrackId, Vec<usize>)> = r
        .compose
        .find_definition(definition_id)
        .map(|def| {
            def.lane_generators
                .iter()
                .filter_map(|(tid, cfg)| match &cfg.kind {
                    LaneGeneratorKind::Drum(dc) => {
                        let pads: Vec<usize> = dc
                            .voices
                            .iter()
                            .filter_map(|(pad, mode)| {
                                matches!(mode, DrumVoiceMode::Motif).then_some(*pad)
                            })
                            .collect();
                        if pads.is_empty() {
                            None
                        } else {
                            Some((*tid, pads))
                        }
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    for (tid, pads) in drum_targets {
        regenerate_drum_motif_voices(r, definition_id, tid, &pads);
    }
}

/// When chords change, regenerate all instrument lanes that have a
/// chord-reading generator (Bass, Melody, Pad).
pub(super) fn propagate_chord_change(r: &mut crate::Resonance, definition_id: u64) {
    let track_ids: Vec<TrackId> = r
        .compose
        .find_definition(definition_id)
        .map(|def| {
            def.lane_generators
                .iter()
                .filter(|(_, cfg)| {
                    matches!(
                        cfg.kind,
                        LaneGeneratorKind::Bass(_)
                            | LaneGeneratorKind::Melody(_)
                            | LaneGeneratorKind::Pad(_)
                            | LaneGeneratorKind::Vocal(_)
                    )
                })
                .map(|(tid, _)| *tid)
                .collect()
        })
        .unwrap_or_default();

    for tid in track_ids {
        // Chord-change propagation runs sync from a non-Task context.
        // The MIDI side of every lane (including vocal) updates inline
        // inside `regenerate_lane`; we drop the off-thread vocal audio
        // render task here — the user re-renders audio on demand via
        // the right-rail "Generate vocal" button.
        let _ = regenerate_lane(r, definition_id, tid);
    }
}

/// Regenerate a single instrument lane, producing MIDI clips for all
/// placements of the section. For vocal lanes this returns a real
/// `Task<Message>` driving the off-thread SVS render; for the other
/// kinds it returns `Task::none()` (all work is synchronous).
pub(super) fn regenerate_lane(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
) -> iced::Task<crate::message::Message> {
    let def = match r.compose.find_definition(definition_id) {
        Some(d) => d.clone(),
        None => return iced::Task::none(),
    };

    let Some(config) = def.lane_generators.get(&track_id) else {
        return iced::Task::none();
    };

    if def.chords.is_empty() {
        return iced::Task::none();
    }

    let kind = match &config.kind {
        LaneGeneratorKind::Bass(_) => DeriveKind::Bass,
        LaneGeneratorKind::Melody(_) => DeriveKind::Lead,
        LaneGeneratorKind::Pad(_) => DeriveKind::Pad,
        LaneGeneratorKind::Drum(_) => return iced::Task::none(),
        // Vocal lanes derive both a MIDI clip (notes) and an audio clip
        // (SVS-synthesised waveform). Dispatch to the dedicated path
        // instead of reusing the chord-only DeriveKind pipeline. The
        // seed has already been bumped by the caller (see
        // `bump_lane_seed`); `roll_vocal_melody` uses the current seed
        // verbatim so we don't double-bump.
        LaneGeneratorKind::Vocal(_) => {
            return super::lane_inspector::roll_vocal_melody(r, definition_id, track_id);
        }
    };

    let gen_params = match &config.kind {
        LaneGeneratorKind::Bass(p) => {
            let mut gp = crate::compose::GenerateParams::default();
            gp.bass = p.clone();
            gp
        }
        LaneGeneratorKind::Melody(p) => {
            let mut gp = crate::compose::GenerateParams::default();
            gp.melody = p.clone();
            gp
        }
        LaneGeneratorKind::Pad(p) => {
            let mut gp = crate::compose::GenerateParams::default();
            gp.pad = p.clone();
            gp
        }
        _ => return iced::Task::none(),
    };

    let notes = generate::derive_notes(
        kind,
        &def.chords,
        def.scale,
        &gen_params,
        &def.motif_source,
        TICKS_PER_QUARTER_NOTE as u32,
        config.seed,
    );

    let time_sig_num = r.transport.time_sig_num;
    let samples_per_bar = compose_samples_per_bar(r.sample_rate, r.transport.bpm, time_sig_num);
    let duration_ticks = def.length_bars as u64 * time_sig_num as u64 * TICKS_PER_QUARTER_NOTE;

    let track_name = r
        .registry
        .tracks
        .iter()
        .find(|t| t.id == track_id)
        .map(|t| t.name.as_str())
        .unwrap_or("Track");
    let name = format!("{} · {}", def.name, track_name);

    let placements: Vec<(u64, u32)> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| (p.id, p.start_bar))
        .collect();

    for (placement_id, start_bar) in placements {
        if let Some(old_id) =
            r.compose
                .derived_clips
                .remove(&(definition_id, placement_id, track_id))
        {
            r.engine
                .send(AudioCommand::DeleteMidiClip { clip_id: old_id });
        }

        let clip_id = r.compose.fresh_derived_clip_id();
        let start_sample = start_bar as u64 * samples_per_bar;
        r.engine.send(AudioCommand::LoadMidiClipDirect {
            clip_id,
            track_id,
            start_sample,
            duration_ticks,
            notes: notes.clone(),
            name: name.clone(),
            trim_start_ticks: 0,
            trim_end_ticks: 0,
        });
        r.compose
            .derived_clips
            .insert((definition_id, placement_id, track_id), clip_id);
    }

    r.compose.last_error = None;
    iced::Task::none()
}

pub(super) fn compose_samples_per_bar(sample_rate: u32, bpm: f32, time_sig_num: u8) -> u64 {
    let samples_per_beat = sample_rate as f64 * 60.0 / bpm as f64;
    (samples_per_beat * time_sig_num as f64) as u64
}

/// Replace every motif-mode pad's hits on every drum clip overlapping
/// the given section. Hits derive from the section's chord progression
/// + shared motif; manual / euclidean pads on the same clip stay put.
fn regenerate_drum_motif_voices(
    r: &mut crate::Resonance,
    definition_id: u64,
    track_id: TrackId,
    pad_indices: &[usize],
) {
    let Some(def) = r.compose.find_definition(definition_id) else {
        return;
    };
    if def.chords.is_empty() || pad_indices.is_empty() {
        return;
    }
    let chords = def.chords.clone();
    let motif_source = def.motif_source.clone();
    let length_bars = def.length_bars;

    let timed = generate::to_timed_chords(&chords);
    let hits = resonance_music_theory::derive_motif_rhythm(
        &timed,
        &motif_source,
        TICKS_PER_QUARTER_NOTE as u32,
    );
    if hits.is_empty() {
        return;
    }

    let base_velocity = r.compose.drumroll.default_velocity;
    let pad_notes: Vec<(usize, u8)> = pad_indices
        .iter()
        .filter_map(|i| r.compose.drumroll.pad_map.get(*i).map(|p| (*i, p.note)))
        .collect();

    let placement_starts: Vec<u32> = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| p.start_bar)
        .collect();

    let samples_per_bar = compose_samples_per_bar(
        r.sample_rate,
        r.transport.bpm,
        r.transport.time_sig_num,
    );

    for start_bar in placement_starts {
        let section_start = (start_bar as u64) * samples_per_bar;
        let section_end = ((start_bar + length_bars) as u64) * samples_per_bar;

        let clip_ids: Vec<u64> = r
            .midi_clips
            .iter()
            .filter(|c| {
                if c.track_id != track_id {
                    return false;
                }
                let clip_end = r.tempo_map.tick_to_abs_sample(
                    c.start_sample,
                    c.duration_ticks,
                    r.sample_rate,
                );
                clip_end > section_start && c.start_sample < section_end
            })
            .map(|c| c.id)
            .collect();

        for clip_id in clip_ids {
            for (_pad_idx, pad_note) in &pad_notes {
                let Some(clip) = r.midi_clips.iter().find(|c| c.id == clip_id) else {
                    continue;
                };
                let new_notes: Vec<MidiNote> = hits
                    .iter()
                    .filter(|h| h.start_tick < clip.duration_ticks)
                    .map(|h| {
                        let velocity = if h.accent {
                            (base_velocity + 0.1).min(1.0)
                        } else {
                            base_velocity
                        };
                        let max_dur = clip.duration_ticks - h.start_tick;
                        MidiNote {
                            note: *pad_note,
                            velocity,
                            start_tick: h.start_tick,
                            duration_ticks: h.duration_ticks.min(max_dur).max(1),
                        }
                    })
                    .collect();
                let (removals, adds) = euclid_edits_for(clip, *pad_note, new_notes);
                for note_index in removals {
                    r.engine
                        .send(AudioCommand::RemoveMidiNote { clip_id, note_index });
                }
                for note in adds {
                    r.engine
                        .send(AudioCommand::AddMidiNote { clip_id, note });
                }
            }
        }
    }
}
