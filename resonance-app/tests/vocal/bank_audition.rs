//! Blind bank audition: render the committed test phrase
//! (`fixtures/bank_audition_song.json`) through the real vocal-render
//! path — registry, resolver, segment, render — on every installed
//! voicebank × speaker, so Jorrit can judge sound quality without
//! knowing which file is which (vocal-mcp-control-and-quality.md §12
//! slice 6, D30).
//!
//! Opt-in, `#[ignore]`d: runs only when `RESONANCE_AUDITION_OUT` names an
//! output directory. Scans banks the same way the real app does
//! (`VoicebankRoots::for_machine`), so `RESONANCE_SVS_MODELS_DIR` and/or a
//! bank dropped into the user voicebank folder both count; with nothing
//! installed it no-ops rather than failing (a fresh checkout has no
//! models — they are git-ignored). With none found this prints why and
//! returns.
//!
//! This test writes plainly-named files (`<bank>__<speaker>.wav`) plus a
//! `manifest.json` listing them — so a direct `cargo test -p resonance-app
//! --test vocal -- --ignored audition` still produces something
//! inspectable on its own, just not blind. `scripts/vocal-audition.py` is
//! the intended entry point: it runs this test, then shuffles
//! `manifest.json`'s entries behind random tokens, renames the files and
//! writes `key.json` (token → bank/speaker), which `--reveal` prints.
//!
//! Every render is checked for silence before anything is written: a
//! bank/speaker that came out silent fails the test loudly instead of
//! leaving a silent WAV in the pile for Jorrit to puzzle over (a silent
//! render that nobody catches is the thing
//! `docs/features/compose/vocal-rendering.md` warns never to ship as a
//! golden, and the same applies here).

use std::path::PathBuf;

use resonance_app::compose::expression::ExpressionCurves;
use resonance_app::compose::vocal_svs::{
    render_vocal_clip, resolve_clip, validate_for_voicebank, write_stereo_wav, SvsRenderCache,
    VoicebankRegistry, VoicebankRoots,
};
use resonance_audio::types::{MidiNote, TICKS_PER_QUARTER_NOTE};
use resonance_music_theory::derive::LyricLine;
use resonance_music_theory::{VocalParams, VoicebankId};
use serde::Deserialize;

const SR: u32 = 44_100;
/// Below this, a "render" is noise-floor silence, not singing.
const SILENCE_FLOOR_DBFS: f64 = -45.0;

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
    lyrics: String,
    notes: Vec<FixtureNote>,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!("fixtures/bank_audition_song.json"))
        .expect("fixtures/bank_audition_song.json parses")
}

fn midi_notes(song: &Fixture) -> Vec<MidiNote> {
    let tpq = TICKS_PER_QUARTER_NOTE as f64;
    song.notes
        .iter()
        .map(|n| MidiNote {
            note: n.pitch,
            velocity: n.velocity,
            start_tick: (n.start_beat * tpq).round() as u64,
            duration_ticks: (n.duration_beat * tpq).round() as u64,
        })
        .collect()
}

fn rms_db(stereo: &[f32]) -> f64 {
    if stereo.is_empty() {
        return f64::NEG_INFINITY;
    }
    let ms = stereo.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / stereo.len() as f64;
    10.0 * ms.max(1e-24).log10()
}

/// One bank × speaker combination's result, for `manifest.json`.
struct Rendered {
    bank: String,
    speaker: Option<String>,
    file: String,
    duration_sec: f64,
    rms_db: f64,
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

    let registry = VoicebankRegistry::scan(VoicebankRoots::for_machine());
    let banks: Vec<_> = registry.installed().cloned().collect();
    if banks.is_empty() {
        eprintln!(
            "no installed voicebanks found (RESONANCE_SVS_MODELS_DIR and the user \
             voicebank dir are both empty or unset); skipped"
        );
        return;
    }

    let song = fixture();
    let notes = midi_notes(&song);
    let draft = vec![LyricLine {
        n: 1,
        rhyme: 'A',
        syllables: notes.len() as u8,
        text: song.lyrics.clone(),
        locked: false,
    }];

    let mut entries: Vec<Rendered> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    for installed in &banks {
        let id = installed.id.clone();
        let profile = registry.profile(&VoicebankId::new(&id));
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
            let resolved = resolve_clip(&draft, &[], notes.len(), &Default::default(), &[], &[]);
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
                ..VocalParams::default()
            };
            let rendered = render_vocal_clip(
                &notes,
                &params,
                &assigned,
                &ExpressionCurves::default(),
                TICKS_PER_QUARTER_NOTE as u32,
                song.bpm,
                SR,
                &mut SvsRenderCache::new(),
                &profile,
                None,
            );
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
            let file = format!("{id}__{file_speaker}.wav");
            let path = out_dir.join(&file);
            write_stereo_wav(&path, &rendered.samples_stereo, rendered.sample_rate)
                .unwrap_or_else(|e| panic!("{label}: writing {path:?} failed: {e}"));
            let frames = rendered.samples_stereo.len() / 2;
            let duration_sec = frames as f64 / rendered.sample_rate as f64;
            eprintln!("{label}: wrote {file} ({duration_sec:.2}s, {db:.1} dBFS)");
            entries.push(Rendered {
                bank: id.clone(),
                speaker: speaker.clone(),
                file,
                duration_sec,
                rms_db: db,
            });
        }
    }

    let manifest = serde_json::json!({
        "entries": entries.iter().map(|e| serde_json::json!({
            "bank": e.bank,
            "speaker": e.speaker,
            "file": e.file,
            "duration_sec": e.duration_sec,
            "rms_db": e.rms_db,
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
