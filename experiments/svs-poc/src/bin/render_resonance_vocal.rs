//! CLI driver mirroring the Resonance app's vocal pipeline. Constructs
//! a DiffSinger segment exactly the way `resonance-app/src/compose/
//! vocal_svs.rs::build_segment` does (lyric → G2P → phoneme durations
//! → f0 with AP padding and continuous carrier pitch), runs the
//! pipeline, and writes a WAV. Used to A/B against the integration's
//! output — if this WAV sounds clean but the integration's doesn't,
//! the bug is in the engine playback path, not the SVS pipeline /
//! segment construction.
//!
//! The G2P here is a verbatim copy of `vocal_g2p.rs` so the phoneme
//! sequence matches what the app generates.

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use svs_poc::audio::write_mono_f32_wav;
use svs_poc::ds::{DsSegment, SampleCurve};
use svs_poc::pipeline::{self, PipelineArgs};
use svs_poc::stages::common::ExecutionProvider;

use resonance_music_theory::{
    derive_vocal, generate_lyrics, Chord, ChordQuality, PitchClass, TimedChord, VocalParams,
    VocalTimbre, VoiceType,
};

#[derive(Parser, Debug)]
struct Args {
    /// Path to the dsacoustic dsconfig.yaml.
    #[arg(long)]
    acoustic_config: PathBuf,
    /// Path to the vocoder yaml.
    #[arg(long)]
    vocoder_config: PathBuf,
    /// Output WAV.
    #[arg(long)]
    out: PathBuf,
    /// Speaker label (defaults based on voice).
    #[arg(long)]
    speaker: Option<String>,
    /// Diffusion speedup.
    #[arg(long, default_value_t = 10)]
    speedup: i32,
    /// Voice type to mirror VocalParams::voice. Default Alto.
    #[arg(long, default_value = "alto")]
    voice: String,
    /// Tempo (matches demo).
    #[arg(long, default_value_t = 90.0)]
    bpm: f32,
    /// Seed.
    #[arg(long, default_value_t = 0xC0FFEE)]
    seed: u64,
}

const SEGMENT_PAD_SEC: f64 = 0.3;
const TICKS_PER_QUARTER: u32 = 480;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    // Demo's B-minor chord progression (mirrors `seed_demo_content`).
    let chords = [
        Chord::new(PitchClass::B, ChordQuality::Min),
        Chord::new(PitchClass::B, ChordQuality::Min),
        Chord::new(PitchClass::Fs, ChordQuality::Maj),
        Chord::new(PitchClass::G, ChordQuality::Maj),
        Chord::new(PitchClass::E, ChordQuality::Min),
    ];
    let timed: Vec<TimedChord> = chords
        .iter()
        .enumerate()
        .map(|(i, c)| TimedChord {
            chord: *c,
            start_beat: i as u32 * 4,
            duration_beats: 4,
        })
        .collect();

    let voice = match args.voice.to_lowercase().as_str() {
        "soprano" => VoiceType::Soprano,
        "mezzosoprano" | "mezzo" => VoiceType::MezzoSoprano,
        "alto" => VoiceType::Alto,
        "tenor" => VoiceType::Tenor,
        "baritone" => VoiceType::Baritone,
        "bass" => VoiceType::Bass,
        _ => VoiceType::Alto,
    };
    let mut params = VocalParams::default();
    params.voice = voice;
    params.range = voice.default_range();
    params.draft = generate_lyrics(&params, args.seed);
    println!("Draft lines:");
    for line in &params.draft {
        println!("  {}.[{}] ({} syl) {}", line.n, line.rhyme, line.syllables, line.text);
    }

    let notes = derive_vocal(&timed, &params, TICKS_PER_QUARTER, args.seed);
    println!("Generated {} notes", notes.len());

    let midi_notes: Vec<MidiNote> = notes
        .iter()
        .map(|n| MidiNote {
            note: n.note,
            start_tick: n.start_tick,
            duration_ticks: n.duration_ticks,
        })
        .collect();

    let segment = build_segment(&midi_notes, &params, TICKS_PER_QUARTER, args.bpm);
    println!(
        "Segment: {} phonemes, total {:.2}s",
        segment.ph_seq.len(),
        segment.ph_dur.iter().sum::<f64>()
    );
    // Detailed dump for diagnosis
    println!("\nPhoneme stream:");
    let mut t = 0.0;
    for (i, (ph, dur)) in segment.ph_seq.iter().zip(segment.ph_dur.iter()).enumerate() {
        let midi = segment.note_seq_midi.get(i).copied().unwrap_or(0);
        let nname = if midi > 0 {
            midi_to_diffsinger_note(midi as u8)
        } else {
            "rest".to_string()
        };
        println!("  t={:5.2}s ph={:<4} dur={:.3}s note={}", t, ph, dur, nname);
        t += dur;
    }

    let speaker = args
        .speaker
        .clone()
        .unwrap_or_else(|| speaker_for_voice(voice).to_string());

    let pipeline_args = PipelineArgs {
        ds_file: PathBuf::new(),
        acoustic_config: args.acoustic_config,
        vocoder_config: args.vocoder_config,
        out: args.out.clone(),
        execution_provider: ExecutionProvider::Cpu,
        device_index: 0,
        speaker: Some(speaker),
        speedup: args.speedup,
        depth: 1000,
    };
    let rendered = pipeline::render_segments(&[segment.clone()], &pipeline_args)
        .context("running SVS pipeline")?;

    let mut mono = rendered.samples;
    // Apply the same AP gating as the app integration does.
    let mut t_cursor = 0.0_f64;
    let mut ap_intervals: Vec<(f64, f64)> = Vec::new();
    for (ph, dur) in segment.ph_seq.iter().zip(segment.ph_dur.iter()) {
        let start = t_cursor;
        t_cursor += *dur;
        if ph == "AP" || ph == "SP" {
            ap_intervals.push((start, t_cursor));
        }
    }
    let fade = (rendered.sample_rate as f64 * 0.005).max(1.0) as usize;
    for (s_sec, e_sec) in &ap_intervals {
        let si = (*s_sec * rendered.sample_rate as f64) as usize;
        let ei = ((*e_sec * rendered.sample_rate as f64) as usize).min(mono.len());
        if si >= mono.len() || ei <= si {
            continue;
        }
        let fade_in_start = si.saturating_sub(fade);
        let fl = si - fade_in_start;
        for (k, idx) in (fade_in_start..si).enumerate() {
            let t = (k + 1) as f32 / (fl + 1) as f32;
            mono[idx] *= 1.0 - t;
        }
        for s in mono[si..ei].iter_mut() {
            *s = 0.0;
        }
        let fade_out_end = (ei + fade).min(mono.len());
        let fl = fade_out_end - ei;
        for (k, idx) in (ei..fade_out_end).enumerate() {
            let t = (k + 1) as f32 / (fl + 1) as f32;
            mono[idx] *= t;
        }
    }

    // Same peak safety as integration.
    let peak = mono.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
    if peak > 0.89 {
        let g = 0.89 / peak;
        for s in mono.iter_mut() {
            *s *= g;
        }
    }

    write_mono_f32_wav(&args.out, &mono, rendered.sample_rate)?;
    println!(
        "Wrote {} samples @ {} Hz to {} (peak {:.3})",
        mono.len(),
        rendered.sample_rate,
        args.out.display(),
        peak
    );
    let _ = SEGMENT_PAD_SEC;
    Ok(())
}

#[derive(Clone)]
struct MidiNote {
    note: u8,
    start_tick: u64,
    duration_ticks: u64,
}

// G2P is provided by `resonance_music_theory::g2p`. The CLI uses the
// same module the app integration does so they're guaranteed to
// produce identical phoneme sequences for the same lyric input.
use resonance_music_theory::g2p::{is_consonant, phonemes_for_draft};

// ===========================================================================
// Segment construction — port of vocal_svs::build_segment
// ===========================================================================

fn build_segment(
    notes: &[MidiNote],
    params: &VocalParams,
    ticks_per_quarter: u32,
    bpm: f32,
) -> DsSegment {
    let seconds_per_tick = 60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter as f64);
    let syllable_phonemes = phonemes_for_draft(&params.draft);
    let consonant_emphasis = params.consonant_emphasis.clamp(0.0, 1.0);
    let cons_dur_target = 0.035 + 0.050 * consonant_emphasis as f64;

    let mut ph_seq: Vec<String> = Vec::new();
    let mut ph_dur: Vec<f64> = Vec::new();
    let mut note_seq: Vec<String> = Vec::new();
    let mut note_dur: Vec<f64> = Vec::new();
    let mut note_seq_midi: Vec<i32> = Vec::new();

    ph_seq.push("AP".to_string());
    ph_dur.push(SEGMENT_PAD_SEC);
    note_seq.push("rest".to_string());
    note_dur.push(SEGMENT_PAD_SEC);
    note_seq_midi.push(0);

    let note_name_cache: Vec<String> = notes
        .iter()
        .map(|n| midi_to_diffsinger_note(n.note))
        .collect();

    for (i, n) in notes.iter().enumerate() {
        let next_start_tick = notes
            .get(i + 1)
            .map(|nx| nx.start_tick)
            .unwrap_or(n.start_tick + n.duration_ticks);
        let slot_ticks = next_start_tick.saturating_sub(n.start_tick);
        let slot_sec = (slot_ticks as f64 * seconds_per_tick).max(0.05);
        let sing_sec_cap = (n.duration_ticks as f64 * seconds_per_tick).max(0.05);
        let (sing_sec, ap_sec) = if slot_sec > sing_sec_cap + 0.4 {
            (sing_sec_cap, slot_sec - sing_sec_cap)
        } else {
            (slot_sec, 0.0)
        };

        let fallback = vec!["ah"];
        let phonemes: &[&'static str] = syllable_phonemes
            .get(i)
            .map(|v| v.as_slice())
            .unwrap_or(&fallback);
        let n_cons = phonemes.iter().filter(|p| is_consonant(p)).count();
        let n_vow = phonemes.len().saturating_sub(n_cons).max(1);
        let cons_total_cap = sing_sec * 0.5;
        let cons_each = if n_cons > 0 {
            cons_dur_target.min(cons_total_cap / n_cons as f64)
        } else {
            0.0
        };
        let vow_total = (sing_sec - cons_each * n_cons as f64).max(0.05);
        let vow_each = vow_total / n_vow as f64;

        let note_name = &note_name_cache[i];
        for ph in phonemes {
            let d = if is_consonant(ph) { cons_each } else { vow_each };
            ph_seq.push((*ph).to_string());
            ph_dur.push(d);
            note_seq.push(note_name.clone());
            note_dur.push(d);
            note_seq_midi.push(n.note as i32);
        }

        if ap_sec > 0.0 {
            ph_seq.push("AP".to_string());
            ph_dur.push(ap_sec);
            note_seq.push("rest".to_string());
            note_dur.push(ap_sec);
            note_seq_midi.push(0);
        }
    }

    ph_seq.push("AP".to_string());
    ph_dur.push(SEGMENT_PAD_SEC);
    note_seq.push("rest".to_string());
    note_dur.push(SEGMENT_PAD_SEC);
    note_seq_midi.push(0);

    // f0_seq construction
    let f0_timestep = 0.005_f64;
    let total_sec: f64 = ph_dur.iter().sum();
    let n_samples = (total_sec / f0_timestep).ceil() as usize + 1;
    let mut f0_samples = Vec::with_capacity(n_samples);
    let mut t = 0.0;
    let mut idx = 0;
    let mut accum = note_dur.first().copied().unwrap_or(0.0);
    for _ in 0..n_samples {
        while t > accum && idx + 1 < note_dur.len() {
            idx += 1;
            accum += note_dur[idx];
        }
        let midi = note_seq_midi.get(idx).copied().unwrap_or(0);
        let hz = if midi <= 0 { 0.0 } else { midi_to_hz(midi as u8) };
        f0_samples.push(hz);
        t += f0_timestep;
    }
    // Forward / back-fill so unvoiced frames carry the neighbouring pitch.
    let first_voiced_idx = f0_samples.iter().position(|v| *v > 0.0);
    if let Some(first_idx) = first_voiced_idx {
        let leading = f0_samples[first_idx];
        for v in f0_samples.iter_mut().take(first_idx) {
            *v = leading;
        }
        let mut last = leading;
        for v in f0_samples.iter_mut().skip(first_idx) {
            if *v > 0.0 {
                last = *v;
            } else {
                *v = last;
            }
        }
    }
    // Portamento — match `vocal_svs.rs` so CLI dump and app integration
    // sound identical.
    let portamento_frames = (0.040_f64 / f0_timestep).round() as usize;
    if portamento_frames >= 2 && f0_samples.len() > portamento_frames {
        let snapshot = f0_samples.clone();
        let mut last_change_idx = 0usize;
        let mut last_val = snapshot[0];
        for i in 1..snapshot.len() {
            let cur = snapshot[i];
            if (cur - last_val).abs() > 0.5 {
                let start = i.saturating_sub(portamento_frames).max(last_change_idx);
                let span = i.saturating_sub(start);
                if span >= 1 {
                    for k in start..i {
                        let t = (k - start + 1) as f64 / (span + 1) as f64;
                        f0_samples[k] = last_val * (1.0 - t) + cur as f64 * t;
                    }
                }
                last_val = cur;
                last_change_idx = i;
            }
        }
    }
    // Vibrato.
    let vibrato = params.vibrato.clamp(0.0, 1.0);
    if vibrato > 0.001 {
        let two_pi = std::f64::consts::TAU;
        for (i, v) in f0_samples.iter_mut().enumerate() {
            if *v > 0.0 {
                let t = i as f64 * f0_timestep;
                let cents = 20.0 * vibrato as f64 * (two_pi * 5.0 * t).sin();
                *v *= 2.0_f64.powf(cents / 1200.0);
            }
        }
    }

    let curve_len = f0_samples.len();
    let gender_value: f64 = match params.timbre {
        VocalTimbre::Airy => 0.20,
        VocalTimbre::Warm => -0.10,
        VocalTimbre::Edged => -0.05,
        VocalTimbre::Bright => 0.35,
    };
    let gender = SampleCurve {
        samples: vec![gender_value; curve_len],
        timestep: f0_timestep,
    };

    DsSegment {
        offset: 0.0,
        ph_seq,
        ph_dur,
        ph_num: Vec::new(),
        note_seq_midi,
        note_dur,
        note_slur: Vec::new(),
        f0: SampleCurve {
            samples: f0_samples,
            timestep: f0_timestep,
        },
        gender,
        velocity: SampleCurve::default(),
        energy: SampleCurve::default(),
        breathiness: SampleCurve::default(),
        voicing: SampleCurve::default(),
        tension: SampleCurve::default(),
    }
}

fn speaker_for_voice(voice: VoiceType) -> &'static str {
    match voice {
        VoiceType::Soprano => "tiger_glam",
        VoiceType::MezzoSoprano => "tiger_fresh",
        VoiceType::Alto => "tiger_disco",
        VoiceType::Tenor => "tiger_royal",
        VoiceType::Baritone => "tiger_electric",
        VoiceType::Bass => "tiger_mystic",
    }
}

fn midi_to_diffsinger_note(midi: u8) -> String {
    const SHARP: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (midi as i32 / 12) - 1;
    let pc = midi as usize % 12;
    format!("{}{}", SHARP[pc], octave)
}

fn midi_to_hz(midi: u8) -> f64 {
    440.0 * (2.0_f64).powf((midi as f64 - 69.0) / 12.0)
}
