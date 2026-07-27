//! Lane regeneration: turn a section's chord progression + per-lane
//! generator config into MIDI clips on every placement of the section,
//! plus the cascade helpers that fan a single chord change or motif
//! seed bump out across every motif/chord-reading lane in the section.

use resonance_audio::types::{AudioCommand, MidiNote, TrackId, TICKS_PER_QUARTER_NOTE};

use crate::compose::{generate, DeriveKind, DrumVoiceMode, LaneGeneratorKind};
use crate::state::MidiClipState;

/// Replace every note on `pad_note` in `clip` with `new_notes`. Returns
/// `(removals, adds)` to feed into the engine command stream — removals
/// are sorted descending so callers can apply them without invalidating
/// later indices.
fn euclid_edits_for(
    clip: &MidiClipState,
    pad_note: u8,
    new_notes: Vec<MidiNote>,
) -> (Vec<usize>, Vec<MidiNote>) {
    let mut removals: Vec<usize> = clip
        .notes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (n.note == pad_note).then_some(i))
        .collect();
    removals.sort_unstable_by(|a, b| b.cmp(a));
    (removals, new_notes)
}

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
    let mut def = match r.compose.find_definition(definition_id) {
        Some(d) => d.clone(),
        None => return iced::Task::none(),
    };

    // Clone the lane config so we can mutate `def` (chord-track overlay,
    // below) without holding a borrow of its `lane_generators`.
    let Some(config) = def.lane_generators.get(&track_id).cloned() else {
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
            return super::vocal_render::roll_vocal_melody(r, definition_id, track_id);
        }
    };

    // Constrain regeneration to the user's pinned chord-track harmony
    // (doc #168, todo #445): pinned chord regions override the section's
    // generated chords and the chord track's key context supplies the
    // scale. Done after the vocal/drum early-returns — the vocal path
    // applies the same overlay inside `roll_vocal_melody`.
    apply_chord_track_harmony(r, definition_id, &mut def);

    let gen_params = match &config.kind {
        LaneGeneratorKind::Bass(p) => crate::compose::GenerateParams {
            bass: *p,
            ..Default::default()
        },
        LaneGeneratorKind::Melody(p) => crate::compose::GenerateParams {
            melody: *p,
            ..Default::default()
        },
        LaneGeneratorKind::Pad(p) => crate::compose::GenerateParams {
            pad: *p,
            ..Default::default()
        },
        _ => return iced::Task::none(),
    };

    // Fill-in-vocal-gaps mode: replace the chosen style's output with
    // a chord-tone arp that walks every silence in the section's
    // vocal lane(s). The user explicitly wants every available space
    // filled — a filter on top of the chosen style only ever removes
    // notes, so when the style itself doesn't put anything in the
    // gaps (Motif-style phrasing, dense vocals that overlap the arp
    // grid), the lane ends up empty.
    let notes = if matches!(&config.kind, LaneGeneratorKind::Melody(p) if p.fill_vocal_gaps) {
        let LaneGeneratorKind::Melody(p) = &config.kind else { unreachable!() };
        let vocal_spans = collect_section_vocal_spans(&def, r.transport.time_sig_num);
        let timed = generate::to_timed_chords(&def.chords);
        // 32nd-note margin — keeps the arp tail off the singer's onset
        // without eating the small silences between phrases.
        let min_gap = TICKS_PER_QUARTER_NOTE / 8;
        let section_end_ticks = def.length_bars as u64
            * r.transport.time_sig_num as u64
            * TICKS_PER_QUARTER_NOTE;
        let filled = resonance_music_theory::derive_melody_fill_vocal(
            &timed,
            p,
            &vocal_spans,
            section_end_ticks,
            TICKS_PER_QUARTER_NOTE as u32,
            min_gap,
        );
        filled
            .iter()
            .map(|n| MidiNote {
                note: n.note,
                velocity: n.velocity,
                start_tick: n.start_tick,
                duration_ticks: n.duration_ticks,
            })
            .collect()
    } else {
        generate::derive_notes(
            kind,
            &def.chords,
            def.scale,
            &gen_params,
            &def.motif_source,
            TICKS_PER_QUARTER_NOTE as u32,
            config.seed,
        )
    };

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
            let _ = r.engine
                .send(AudioCommand::DeleteMidiClip { clip_id: old_id });
        }

        let clip_id = r.compose.fresh_derived_clip_id();
        let start_sample = start_bar as u64 * samples_per_bar;
        let _ = r.engine.send(AudioCommand::LoadMidiClipDirect {
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

/// Overlay the global chord track's user-pinned regions onto a section
/// definition's chords and adopt the chord track's key context, so a
/// regenerated lane follows the user's pinned harmony rather than the
/// section's own auto-generated chords (doc #168, todo #445).
///
/// The section is anchored to its earliest placement on the timeline: an
/// unplaced section has no absolute position to map the chord track onto
/// (and renders nothing anyway), so it is left untouched. A section
/// placed more than once is anchored to its first placement — the
/// derived clip is reused across every placement, so a single harmonic
/// reading is all the rest of the pipeline can carry.
pub(crate) fn apply_chord_track_harmony(
    r: &crate::Resonance,
    definition_id: u64,
    def: &mut crate::compose::SectionDefinitionState,
) {
    let Some(start_bar) = r
        .compose
        .placements
        .iter()
        .filter(|p| p.definition_id == definition_id)
        .map(|p| p.start_bar)
        .min()
    else {
        return;
    };

    let samples_per_bar =
        compose_samples_per_bar(r.sample_rate, r.transport.bpm, r.transport.time_sig_num);
    let section_start_sample = start_bar as u64 * samples_per_bar;
    let samples_per_beat = r.sample_rate as f64 * 60.0 / r.transport.bpm as f64;

    def.chords = generate::overlay_pinned_chords(
        &def.chords,
        &r.chord_track,
        section_start_sample,
        samples_per_beat,
    );
    if let Some(scale) = r.chord_track.key_at(section_start_sample) {
        def.scale = Some(scale);
    }
}

/// Derive every vocal lane in the section to a flat list of
/// **phrase-level** `(start_tick, end_tick)` intervals for the
/// `MelodyParams::fill_vocal_gaps` path. Returns an empty vec when no
/// vocal lane exists or every vocal lane has an empty draft.
///
/// Phrase spans (not per-syllable notes) are what the fill generator
/// needs: the natural silences *between* syllables of one phrase are
/// big enough for the arp to wedge stubs into, so feeding raw notes
/// produces fill that rattles inside the vocal phrase instead of
/// complementing it. `vocal_phrase_spans` collapses each lyric line
/// down to one span (earliest onset → latest offset of its notes),
/// which is the unit the user thinks of as "the vocal is sounding."
fn collect_section_vocal_spans(
    def: &crate::compose::SectionDefinitionState,
    time_sig_num: u8,
) -> Vec<(u64, u64)> {
    let timed = generate::to_timed_chords(&def.chords);
    if timed.is_empty() {
        return Vec::new();
    }
    let beats_per_bar = time_sig_num.max(1) as u32;
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
    let mut all = Vec::new();
    for (_, cfg) in def.lane_generators.iter() {
        let LaneGeneratorKind::Vocal(params) = &cfg.kind else {
            continue;
        };
        if params.draft.is_empty() {
            continue;
        }
        let notes = resonance_music_theory::derive_vocal_with_motif(
            &timed,
            params,
            TICKS_PER_QUARTER_NOTE as u32,
            beats_per_bar,
            Some(&motif_intervals),
            cfg.seed,
        );
        all.extend(resonance_music_theory::vocal_phrase_spans(&notes, params));
    }
    all
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
                    let _ = r.engine
                        .send(AudioCommand::RemoveMidiNote { clip_id, note_index });
                }
                for note in adds {
                    let _ = r.engine
                        .send(AudioCommand::AddMidiNote { clip_id, note });
                }
            }
        }
    }
}
