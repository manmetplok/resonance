//! Blind bank audition: render the committed test phrase
//! (`fixtures/bank_audition_song.json`) through the real vocal-render
//! path — registry, resolver (with the bank's own dictionary), segment,
//! render — on every installed voicebank × speaker, so Jorrit can judge
//! sound quality without knowing which file is which
//! (vocal-mcp-control-and-quality.md §12 slice 6, D30), and so
//! `scripts/vocal-intelligibility.py` can transcribe each take and score
//! it against the lyric.
//!
//! The phrase is two plain lines, syllabified with `·` so it fills its
//! notes exactly (checked the way `song.vocal`'s `counts_mismatch` checks
//! a lane, before anything renders). Its melody is written in C and
//! transposed per bank × speaker so its duration-weighted centre sits in
//! the middle of that voice's comfortable range
//! ([`BankProfile::comfortable_range`]) — one fixed melody sang TIGER (a
//! male voice) up to D5.
//!
//! `audition` is opt-in, `#[ignore]`d: it runs only when
//! `RESONANCE_AUDITION_OUT` names an output directory. It scans banks the
//! same way the real app does (`VoicebankRoots::for_machine`), so
//! `RESONANCE_SVS_MODELS_DIR` and/or a bank dropped into the user
//! voicebank folder both count; with nothing installed it prints why and
//! returns. `RESONANCE_AUDITION_BANKS=tiger,lilia` limits it to those
//! banks; `RESONANCE_AUDITION_DUMP=1` also writes each take's render units
//! (`<bank>__<speaker>.units.json`: the phonemes, durations, notes and
//! language ids the model was fed), for comparing against what a bank's
//! own tools would send; `RESONANCE_AUDITION_FIXTURE=<file>` sings another
//! phrase of the same shape.
//!
//! It writes plainly-named files (`<bank>__<speaker>.wav`) plus a
//! `manifest.json` listing them with the lyric text and the transposition
//! — so a direct `cargo test -p resonance-app --test vocal -- --ignored
//! audition` still produces something inspectable on its own, just not
//! blind. `scripts/vocal-audition.py` shuffles them behind random tokens;
//! `scripts/vocal-intelligibility.py` scores them.
//!
//! Every render is checked for silence before anything is written: a
//! bank/speaker that came out silent fails the test loudly instead of
//! leaving a silent WAV in the pile.

use std::path::PathBuf;

use resonance_app::compose::vocal_svs::{
    render_vocal_clip, resolve_clip_with_bank, split_render_units, validate_for_voicebank,
    write_stereo_wav, BankProfile, SvsRenderCache, VoicebankRegistry, VoicebankRoots,
};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::g2p::{self, PhonemeDictionary};
use resonance_music_theory::{VocalParams, VoicebankId};
use serde::Deserialize;

const SR: u32 = 44_100;
/// Below this, a "render" is noise-floor silence, not singing.
const SILENCE_FLOOR_DBFS: f64 = -45.0;
/// The widest melody the audition may sing: an octave and a third.
const MAX_SPAN_SEMITONES: u8 = 16;

#[derive(Deserialize)]
struct FixtureNote {
    start_beat: f64,
    duration_beat: f64,
    pitch: u8,
    velocity: f32,
}

#[derive(Deserialize)]
struct Fixture {
    bpm: f32,
    lines: Vec<String>,
    text: String,
    notes: Vec<FixtureNote>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/bank_audition_song.json"))
        .expect("fixtures/bank_audition_song.json parses")
}

/// The phrase `audition` sings: the committed fixture, or the file
/// `RESONANCE_AUDITION_FIXTURE` names (same shape) — a second phrase for
/// checking that a change helps beyond the one it was measured on.
fn audition_fixture() -> Fixture {
    match std::env::var_os("RESONANCE_AUDITION_FIXTURE").filter(|v| !v.is_empty()) {
        Some(path) => {
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("RESONANCE_AUDITION_FIXTURE {path:?}: {e}"));
            serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("RESONANCE_AUDITION_FIXTURE {path:?}: {e}"))
        }
        None => fixture(),
    }
}

fn draft(song: &Fixture) -> Vec<LyricLine> {
    song.lines
        .iter()
        .enumerate()
        .map(|(i, text)| LyricLine {
            n: i as u8 + 1,
            rhyme: 'A',
            syllables: 0,
            text: text.clone(),
            locked: false,
        })
        .collect()
}

fn midi_notes(song: &Fixture, transpose: i32) -> Vec<MidiNote> {
    let tpq = TICKS_PER_QUARTER_NOTE as f64;
    song.notes
        .iter()
        .map(|n| MidiNote {
            note: (n.pitch as i32 + transpose).clamp(0, 127) as u8,
            velocity: n.velocity,
            start_tick: (n.start_beat * tpq).round() as u64,
            duration_ticks: (n.duration_beat * tpq).round() as u64,
        })
        .collect()
}

/// Semitones that move the melody's duration-weighted centre to the
/// middle of `(low, high)`, then keep it inside the range where it fits.
fn transpose_into(song: &Fixture, (low, high): (u8, u8)) -> i32 {
    let total: f64 = song.notes.iter().map(|n| n.duration_beat).sum();
    let centre: f64 =
        song.notes.iter().map(|n| n.pitch as f64 * n.duration_beat).sum::<f64>() / total;
    let target = (low as f64 + high as f64) / 2.0;
    let mut shift = (target - centre).round() as i32;
    let min = song.notes.iter().map(|n| n.pitch as i32).min().unwrap_or(0);
    let max = song.notes.iter().map(|n| n.pitch as i32).max().unwrap_or(0);
    if max + shift > high as i32 {
        shift = high as i32 - max;
    }
    if min + shift < low as i32 {
        shift = low as i32 - min;
    }
    shift
}

/// The fixture's syllables + holds against its notes, resolved with
/// `bank`'s dictionary exactly as `song.vocal` counts a lane: `Err`
/// names the mismatch.
fn check_counts(song: &Fixture, bank: Option<&PhonemeDictionary>) -> Result<(), String> {
    let empty = PhonemeDictionary::new();
    let layers = g2p::DictionaryLayers {
        project: &empty,
        bank,
    };
    let resolved = g2p::resolve_draft_with_layers(&draft(song), layers);
    let units = g2p::note_unit_count(&resolved);
    if units == song.notes.len() {
        Ok(())
    } else {
        Err(format!(
            "the lyric fills {units} notes but the melody has {}: {:?}",
            song.notes.len(),
            resolved.iter().map(|s| s.label.as_str()).collect::<Vec<_>>()
        ))
    }
}

fn rms_db(stereo: &[f32]) -> f64 {
    if stereo.is_empty() {
        return f64::NEG_INFINITY;
    }
    let ms = stereo.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / stereo.len() as f64;
    10.0 * ms.max(1e-24).log10()
}

#[test]
fn the_audition_phrase_fills_its_notes_and_fits_every_builtin_voice() {
    let song = fixture();
    check_counts(&song, None).unwrap();
    let pitches: Vec<u8> = song.notes.iter().map(|n| n.pitch).collect();
    let span = pitches.iter().max().unwrap() - pitches.iter().min().unwrap();
    assert!(span <= MAX_SPAN_SEMITONES, "span {span} semitones");

    // The melody resolves cleanly (no fallback `ah`, one written hold, one
    // written breath) and sits inside every built-in voice's range once
    // transposed.
    let registry = VoicebankRegistry::scan(VoicebankRoots::default());
    let resolved = resolve_clip_with_bank(
        &draft(&song),
        &[],
        song.notes.len(),
        &Default::default(),
        &[],
        &[],
        None,
    );
    assert_eq!(resolved.iter().filter(|a| a.is_slur).count(), 1);
    assert_eq!(resolved.iter().filter(|a| a.breath_before).count(), 1);
    assert!(resolved.iter().all(|a| !a.phonemes.is_empty()));
    for id in ["tiger", "lilia", "meiji"] {
        let profile = registry.profile(&VoicebankId::new(id));
        let mut speakers: Vec<Option<String>> =
            profile.speakers().into_iter().map(|s| Some(s.id)).collect();
        if speakers.is_empty() {
            speakers.push(None);
        }
        for speaker in speakers {
            let range = profile.comfortable_range(speaker.as_deref());
            let shift = transpose_into(&song, (range.low, range.high));
            for n in midi_notes(&song, shift) {
                assert!(
                    (range.low..=range.high).contains(&n.note),
                    "{id}/{speaker:?}: note {} outside {}..={} after {shift:+}",
                    n.note,
                    range.low,
                    range.high
                );
            }
            validate_for_voicebank(&resolved, &profile)
                .unwrap_or_else(|e| panic!("{id}: phonemes don't map: {e:?}"));
        }
    }
}

/// One bank × speaker combination's result, for `manifest.json`.
struct Rendered {
    bank: String,
    speaker: Option<String>,
    file: String,
    duration_sec: f64,
    rms_db: f64,
    transpose: i32,
    range: (u8, u8),
    /// Wall time of the render (session loads included).
    render_sec: f64,
}

/// A lane flag the audition sings with: `name=0` turns it off, `name=1`
/// on; unset keeps `default` (the lane default).
fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name).ok().as_deref().map(str::trim) {
        Some("0") | Some("false") => false,
        Some("1") | Some("true") => true,
        _ => default,
    }
}

fn wanted_banks() -> Option<Vec<String>> {
    std::env::var("RESONANCE_AUDITION_BANKS")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.split(',').map(|s| s.trim().to_ascii_lowercase()).collect())
}

/// The render units' scores as JSON — what the model is fed.
fn units_json(
    notes: &[MidiNote],
    params: &VocalParams,
    assigned: &[g2p::AssignedSyllable],
    bpm: f32,
    profile: &BankProfile,
) -> serde_json::Value {
    let units = split_render_units(
        notes,
        params,
        assigned,
        TICKS_PER_QUARTER_NOTE as u32,
        bpm,
        profile,
    );
    serde_json::json!(units
        .iter()
        .map(|u| serde_json::json!({
            "start_sec": u.start_sec,
            "ph_seq": u.segment.ph_seq,
            "ph_dur": u.segment.ph_dur,
            "note_seq_midi": u.segment.note_seq_midi,
            "languages": u.segment.languages,
            "f0": u.segment.f0.samples,
            "f0_timestep": u.segment.f0.timestep,
            "gender": u.segment.gender.samples,
            "tension": u.segment.tension.samples,
        }))
        .collect::<Vec<_>>())
}

#[test]
#[ignore = "needs RESONANCE_AUDITION_OUT and installed voicebanks; run scripts/vocal-audition.py"]
fn audition() {
    let Some(out_dir) = std::env::var_os("RESONANCE_AUDITION_OUT").filter(|v| !v.is_empty())
    else {
        eprintln!(
            "RESONANCE_AUDITION_OUT unset; skipped (run scripts/vocal-audition.py, which sets it)"
        );
        return;
    };
    let out_dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir)
        .unwrap_or_else(|e| panic!("create RESONANCE_AUDITION_OUT {out_dir:?}: {e}"));
    let dump = std::env::var_os("RESONANCE_AUDITION_DUMP").is_some_and(|v| v != "0");

    let registry = VoicebankRegistry::scan(VoicebankRoots::for_machine());
    let wanted = wanted_banks();
    let banks: Vec<_> = registry
        .installed()
        .filter(|b| wanted.as_ref().is_none_or(|w| w.contains(&b.id)))
        .cloned()
        .collect();
    if banks.is_empty() {
        eprintln!(
            "no installed voicebanks found (RESONANCE_SVS_MODELS_DIR and the user \
             voicebank dir are both empty or unset, or RESONANCE_AUDITION_BANKS matched \
             none); skipped"
        );
        return;
    }

    let song = audition_fixture();
    let draft = draft(&song);

    let mut entries: Vec<Rendered> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for installed in &banks {
        let id = installed.id.clone();
        let profile = registry.profile(&VoicebankId::new(&id));
        let bank_dictionary = profile.dictionary();
        // What the app checks before a render: the lyric fills the notes.
        if let Err(e) = check_counts(&song, bank_dictionary.as_deref()) {
            failures.push(format!("{id}: {e}"));
            continue;
        }
        let speakers = profile.speakers();
        let speaker_ids: Vec<Option<String>> = if speakers.is_empty() {
            vec![None]
        } else {
            speakers.into_iter().map(|s| Some(s.id)).collect()
        };

        for speaker in speaker_ids {
            let label = match &speaker {
                Some(s) => format!("{id}/{s}"),
                None => id.clone(),
            };
            let range = profile.comfortable_range(speaker.as_deref());
            let transpose = transpose_into(&song, (range.low, range.high));
            let notes = midi_notes(&song, transpose);
            // The same resolve + gate `plan_vocal_render` runs.
            let resolved = resolve_clip_with_bank(
                &draft,
                &[],
                notes.len(),
                &Default::default(),
                &[],
                &[],
                bank_dictionary.as_deref(),
            );
            let assigned = match validate_for_voicebank(&resolved, &profile) {
                Ok(a) => a,
                Err(invalid) => {
                    failures.push(format!("{label}: phonemes don't map onto this bank: {invalid:?}"));
                    continue;
                }
            };
            let params = VocalParams {
                voicebank: VoicebankId::new(&id),
                speaker: speaker.clone(),
                bank_timing: env_flag("RESONANCE_AUDITION_BANK_TIMING", true),
                bank_pitch: env_flag("RESONANCE_AUDITION_BANK_PITCH", true),
                ..VocalParams::default()
            };
            let started = std::time::Instant::now();
            let rendered = render_vocal_clip(
                &notes,
                &params,
                &assigned,
                TICKS_PER_QUARTER_NOTE as u32,
                song.bpm,
                SR,
                &mut SvsRenderCache::new(),
                &profile,
                None,
            );
            let render_sec = started.elapsed().as_secs_f64();
            let rendered = match rendered {
                Ok(Some(r)) => r,
                Ok(None) => {
                    failures.push(format!("{label}: no notes to render"));
                    continue;
                }
                Err(e) => {
                    failures.push(format!("{label}: render failed: {e}"));
                    continue;
                }
            };
            let db = rms_db(&rendered.samples_stereo);
            if db < SILENCE_FLOOR_DBFS {
                failures.push(format!(
                    "{label}: render came out silent ({db:.1} dBFS < {SILENCE_FLOOR_DBFS} \
                     dBFS floor) — not writing it"
                ));
                continue;
            }
            let file_speaker = speaker.as_deref().unwrap_or("default");
            let stem = format!("{id}__{file_speaker}");
            let file = format!("{stem}.wav");
            let path = out_dir.join(&file);
            write_stereo_wav(&path, &rendered.samples_stereo, rendered.sample_rate)
                .unwrap_or_else(|e| panic!("{label}: writing {path:?} failed: {e}"));
            if dump {
                let units = units_json(&notes, &params, &assigned, song.bpm, &profile);
                let units_path = out_dir.join(format!("{stem}.units.json"));
                std::fs::write(&units_path, serde_json::to_string_pretty(&units).unwrap())
                    .unwrap_or_else(|e| panic!("{label}: writing {units_path:?} failed: {e}"));
            }
            let frames = rendered.samples_stereo.len() / 2;
            let duration_sec = frames as f64 / rendered.sample_rate as f64;
            eprintln!(
                "{label}: wrote {file} ({duration_sec:.2}s, {db:.1} dBFS, transposed \
                 {transpose:+}, rendered in {render_sec:.2}s)"
            );
            entries.push(Rendered {
                bank: id.clone(),
                speaker: speaker.clone(),
                file,
                duration_sec,
                rms_db: db,
                transpose,
                range: (range.low, range.high),
                render_sec,
            });
        }
    }

    let manifest = serde_json::json!({
        "text": song.text,
        "entries": entries.iter().map(|e| serde_json::json!({
            "bank": e.bank,
            "speaker": e.speaker,
            "file": e.file,
            "duration_sec": e.duration_sec,
            "rms_db": e.rms_db,
            "transpose": e.transpose,
            "range": [e.range.0, e.range.1],
            "render_sec": e.render_sec,
        })).collect::<Vec<_>>(),
    });
    std::fs::write(
        out_dir.join("manifest.json"),
        serde_json::to_string_pretty(&manifest).expect("manifest serialises"),
    )
    .unwrap_or_else(|e| panic!("write manifest.json: {e}"));

    eprintln!(
        "{} of {} bank x speaker combinations rendered into {out_dir:?}",
        entries.len(),
        entries.len() + failures.len(),
    );
    assert!(
        failures.is_empty(),
        "every installed bank x speaker should render this phrase cleanly:\n{}",
        failures.join("\n")
    );
    assert!(!entries.is_empty(), "at least one bank x speaker should have rendered");
}
