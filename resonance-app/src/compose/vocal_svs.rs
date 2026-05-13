//! Bridge between the Compose vocal generator and the `svs-poc` DiffSinger
//! pipeline. Given the MIDI notes produced by `derive_vocal` plus the lane's
//! `VocalParams`, this module builds a one-segment DiffSinger score, runs
//! the acoustic + vocoder ONNX pipeline, and writes a stereo 32-bit-float
//! WAV the engine can mmap via `AudioCommand::LoadClipFromWav`.
//!
//! Each syllable from the lyric draft is run through
//! `resonance_music_theory::g2p` (CMU dictionary + rule-based fallback)
//! to produce ARPAbet phonemes; the note's duration is shared across
//! those phonemes (consonants short, vowel long) so words like "morning"
//! actually sound like "morning". The f0 curve is piecewise constant per
//! note with a ~40 ms portamento between adjacent notes; an optional
//! 5 Hz vibrato modulates the curve when the user's `vibrato` slider is
//! non-zero.

use std::path::PathBuf;

use resonance_audio::types::MidiNote;
use resonance_music_theory::{g2p, VocalParams, VocalTimbre, VoiceType};
use svs_poc::ds::{DsSegment, SampleCurve};
use svs_poc::pipeline::{self, PipelineArgs};
use svs_poc::stages::common::ExecutionProvider;

/// Where to look for the DiffSinger acoustic + vocoder configs. Resolution
/// order:
///   1. `RESONANCE_SVS_MODELS_DIR` env var
///   2. workspace-root `experiments/svs-poc/models/` (PoC default)
///
/// Returns `None` if neither location contains the expected config files
/// — callers should treat that as "SVS unavailable, skip silently".
fn locate_models() -> Option<(PathBuf, PathBuf)> {
    let candidates: Vec<PathBuf> = std::iter::once(std::env::var_os("RESONANCE_SVS_MODELS_DIR"))
        .flatten()
        .map(PathBuf::from)
        .chain(std::iter::once(default_models_dir()))
        .collect();

    for root in candidates {
        let acoustic = root.join("singer/extracted/dsacoustic/dsconfig.yaml");
        // The TIGER voicebank ships its own vocoder
        // (`tgm_hifigan.onnx`, r03) under the singer dir. That's the
        // vocoder the acoustic model was trained against; using a
        // different `tgm_hifigan_v110.onnx` produces noticeably rougher
        // audio because the mel-spectrogram statistics don't match.
        // Prefer the bundled vocoder, fall back to the generic one
        // (`models/vocoder/dsvocoder/`) for setups that only have the
        // generic version installed.
        let bundled_vocoder = root.join("singer/extracted/dsvocoder/vocoder.yaml");
        let generic_vocoder = root.join("vocoder/dsvocoder/vocoder.yaml");
        let vocoder = if bundled_vocoder.exists() {
            bundled_vocoder
        } else {
            generic_vocoder
        };
        if acoustic.exists() && vocoder.exists() {
            return Some((acoustic, vocoder));
        }
    }
    None
}

/// Workspace-relative default — the SVS PoC ships its model dir at
/// `experiments/svs-poc/models/`. Anchored against the binary's
/// `CARGO_MANIFEST_DIR` so it resolves from a `cargo run` in any subdir.
fn default_models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../experiments/svs-poc/models")
}

/// Output of `render_vocal_clip` — stereo-interleaved f32 samples ready
/// for `transcode_to_wav` / `write_stereo_f32_wav`, plus their rate
/// and the number of leading/trailing frames that are silence pads
/// (so the engine clip can trim past them via `trim_start_frames` /
/// `trim_end_frames`).
pub struct RenderedVocal {
    pub samples_stereo: Vec<f32>,
    pub sample_rate: u32,
    pub trim_start_frames: u64,
    pub trim_end_frames: u64,
}

/// Leading + trailing silence pad inserted into every rendered
/// segment, in seconds. Matches the reference `.ds` fixtures'
/// convention; the engine clip trims past it so timeline alignment
/// stays correct.
const SEGMENT_PAD_SEC: f64 = 0.3;

/// Render a vocal MIDI clip + lyric draft to a singing waveform. Returns
/// `None` when the SVS model files aren't installed (the PoC's download
/// script hasn't been run, env var unset, etc.) so the caller can fall
/// back to its existing MIDI-only behaviour silently.
pub fn render_vocal_clip(
    notes: &[MidiNote],
    params: &VocalParams,
    ticks_per_quarter: u32,
    bpm: f32,
    engine_sample_rate: u32,
) -> Result<Option<RenderedVocal>, String> {
    if notes.is_empty() {
        return Ok(None);
    }
    let Some((acoustic_config, vocoder_config)) = locate_models() else {
        return Ok(None);
    };

    let segment = build_segment(notes, params, ticks_per_quarter, bpm);
    if segment.ph_seq.is_empty() {
        return Ok(None);
    }

    let args = PipelineArgs {
        ds_file: PathBuf::new(),
        acoustic_config,
        vocoder_config,
        out: PathBuf::new(),
        execution_provider: ExecutionProvider::Cpu,
        device_index: 0,
        speaker: Some(speaker_for_voice(params.voice).to_string()),
        // The TIGER voicebank's working CLI command uses speedup=20.
        // (We previously thought 20 produced diffusion artifacts, but
        // those artifacts were actually from feeding the mel into the
        // wrong vocoder.)
        speedup: 20,
        depth: 1000,
    };

    // Compute the time intervals of every AP/SP phoneme in the
    // segment (start_sec, end_sec). We use these to gate the rendered
    // audio: the neural vocoder doesn't produce true silence during
    // AP — it emits a ~-50 dB hiss that the user perceives as constant
    // noise between vocal lines. Hard-muting the AP regions (with a
    // ~5 ms fade so we don't get clicks at the boundaries) is what
    // turns the gap into actual silence.
    let mut ap_intervals: Vec<(f64, f64)> = Vec::new();
    let mut t_cursor = 0.0_f64;
    for (ph, dur) in segment.ph_seq.iter().zip(segment.ph_dur.iter()) {
        let start = t_cursor;
        t_cursor += *dur;
        if ph == "AP" || ph == "SP" {
            ap_intervals.push((start, t_cursor));
        }
    }

    let rendered = pipeline::render_segments(&[segment], &args)
        .map_err(|e| format!("svs render: {e:#}"))?;
    let model_sr = rendered.sample_rate;

    let mut mono: Vec<f32> = rendered.samples;

    // Apply the AP gate. Convert each AP interval to sample indices,
    // zero the body, and apply a short linear fade at each boundary
    // so the transition from voiced → silence → voiced doesn't click.
    let fade_samples = (model_sr as f64 * 0.005).max(1.0) as usize; // 5 ms
    for (start_sec, end_sec) in &ap_intervals {
        let start_idx = (*start_sec * model_sr as f64) as usize;
        let end_idx = (*end_sec * model_sr as f64) as usize;
        if start_idx >= mono.len() {
            continue;
        }
        let end_idx = end_idx.min(mono.len());
        if end_idx <= start_idx {
            continue;
        }
        // Fade out into AP at the start of the interval. We only fade
        // the LAST `fade_samples` of the voiced run preceding this AP,
        // not the AP itself, so that the AP body is fully silent.
        let fade_in_end = start_idx;
        let fade_in_start = fade_in_end.saturating_sub(fade_samples);
        let fade_len = fade_in_end.saturating_sub(fade_in_start);
        for (k, sample_idx) in (fade_in_start..fade_in_end).enumerate() {
            let t = (k + 1) as f32 / (fade_len + 1) as f32;
            mono[sample_idx] *= 1.0 - t;
        }
        // Mute the AP body completely.
        for s in mono[start_idx..end_idx].iter_mut() {
            *s = 0.0;
        }
        // Fade in from AP at the end of the interval. We fade the
        // FIRST `fade_samples` of the voiced run following this AP.
        let fade_out_start = end_idx;
        let fade_out_end = (end_idx + fade_samples).min(mono.len());
        let fade_len = fade_out_end - fade_out_start;
        for (k, sample_idx) in (fade_out_start..fade_out_end).enumerate() {
            let t = (k + 1) as f32 / (fade_len + 1) as f32;
            mono[sample_idx] *= t;
        }
    }

    // Peak-aware safety scale. Neural vocoders occasionally peak above
    // 1.0 — when that hits the engine's master clamp at [-1, 1] (see
    // `mixer/master.rs:97-105`) it produces audible hard-clip
    // distortion. Detect the peak and apply just enough gain reduction
    // to leave ~1 dB of headroom. Quiet renders pass through unchanged.
    let peak = mono.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
    let headroom_target = 0.89; // ≈ -1 dB
    let safety_gain: f32 = if peak > headroom_target {
        headroom_target / peak
    } else {
        1.0
    };

    // Engine expects stereo-interleaved f32. The vocoder emits mono — just
    // duplicate the channel and apply the safety gain in the same pass.
    let mut stereo = Vec::with_capacity(mono.len() * 2);
    for &s in &mono {
        let v = s * safety_gain;
        stereo.push(v);
        stereo.push(v);
    }

    // The engine's mixer reads clip samples 1:1 with timeline frames —
    // it does not resample on the fly. If the SVS WAV is 44.1 kHz but
    // the audio device runs at 48 kHz (or 96 kHz, etc.) the clip would
    // play back at the wrong speed, sounding pitched-up and distorted.
    // Resample here so the WAV's frame rate matches the engine's.
    let (samples_stereo, out_sr) = if model_sr != engine_sample_rate {
        let resampled = resonance_audio::decode::linear_resample(
            &stereo,
            model_sr,
            engine_sample_rate,
        );
        (resampled, engine_sample_rate)
    } else {
        (stereo, model_sr)
    };

    // The segment was rendered with `SEGMENT_PAD_SEC` of AP on each
    // end (matches the reference `.ds` fixtures so the model ramps in
    // and out cleanly). Translate that pad into frame counts at the
    // *final* sample rate so the engine's clip trim skips past it on
    // playback — keeps the timeline alignment between MIDI and audio.
    let pad_frames = (SEGMENT_PAD_SEC * out_sr as f64) as u64;

    Ok(Some(RenderedVocal {
        samples_stereo,
        sample_rate: out_sr,
        trim_start_frames: pad_frames,
        trim_end_frames: pad_frames,
    }))
}

/// Build a single `DsSegment` covering every note in the clip. Each
/// note's syllable text (drawn from `params.draft`) is run through G2P
/// to produce ARPAbet phonemes; the note's duration is split between
/// them with consonants getting a short slice and the vowel getting
/// the remainder. `f0_seq` is sampled at a fixed `f0_timestep` interval
/// over the whole segment.
fn build_segment(
    notes: &[MidiNote],
    params: &VocalParams,
    ticks_per_quarter: u32,
    bpm: f32,
) -> DsSegment {
    // Seconds per tick at the section's tempo. Vocal-lane MIDI clips use
    // `TICKS_PER_QUARTER_NOTE` as their tick rate, same as everywhere else
    // in the app.
    let seconds_per_tick = 60.0 / (bpm.max(1.0) as f64 * ticks_per_quarter as f64);

    // Resolve syllable phonemes by walking the lyric draft word by
    // word. CMU dict gives us proper English pronunciation for each
    // word; multi-syllable words have their phoneme stream split
    // across the matching syllable notes so e.g. `hou·ses` becomes
    // `[hh aw] [z ah z]` on two notes (vs. our old rule-based
    // `[hh aw s] [eh s]` which sang as "house-ess").
    let syllable_phonemes: Vec<Vec<&'static str>> = g2p::phonemes_for_draft(&params.draft);
    let consonant_emphasis = params.consonant_emphasis.clamp(0.0, 1.0) as f64;
    // Consonant target duration in seconds. `consonant_emphasis` slides
    // between a brisk 35 ms (low) and a deliberate 85 ms (high). Capped
    // later to half the note's duration so a fast syllable still has a
    // recognisable vowel.
    let cons_dur_target = 0.035 + 0.050 * consonant_emphasis;

    let mut ph_seq: Vec<String> = Vec::new();
    let mut ph_dur: Vec<f64> = Vec::new();
    let mut note_seq: Vec<String> = Vec::new();
    let mut note_dur: Vec<f64> = Vec::new();
    let mut note_seq_midi: Vec<i32> = Vec::new();

    // Leading silence pad. The hand-crafted reference fixtures all
    // start with a 0.3 s `AP` so the model has time to ramp up cleanly;
    // skipping this produces an attack click on the first phoneme.
    ph_seq.push("AP".to_string());
    ph_dur.push(SEGMENT_PAD_SEC);
    note_seq.push("rest".to_string());
    note_dur.push(SEGMENT_PAD_SEC);
    note_seq_midi.push(0);

    let note_name_cache: Vec<String> = notes
        .iter()
        .map(|n| midi_to_diffsinger_note(n.note))
        .collect();

    // Walk notes back-to-back. We never insert AP between adjacent
    // syllables — the reference fixtures (`twinkle.ds`,
    // `hello_tiger.ds`) keep phonemes flowing continuously and let
    // the model handle syllable boundaries naturally. Each note's
    // effective sing duration is the time *until the next note* (or
    // its stated `duration_ticks` for the final note), so any
    // articulation-trim gap is absorbed automatically. Real silences
    // (gaps > 0.4 s between consecutive notes, which only happens at
    // genuine breath / rest points) still become an explicit AP.
    for (i, n) in notes.iter().enumerate() {
        let next_start_tick = notes
            .get(i + 1)
            .map(|nx| nx.start_tick)
            .unwrap_or(n.start_tick + n.duration_ticks);
        let slot_ticks = next_start_tick.saturating_sub(n.start_tick);
        let slot_sec = (slot_ticks as f64 * seconds_per_tick).max(0.05);

        // For genuine silences (long gaps to the next note), cap the
        // sing duration and put the rest into a trailing AP. Threshold
        // chosen so half-bar pauses become rests but typical syllable
        // spacing doesn't.
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

        // Split `sing_sec` across phonemes: each consonant gets up to
        // `cons_dur_target`, capped so consonants never eat more than
        // half the syllable. The vowel(s) absorb the remainder evenly.
        let n_cons = phonemes.iter().filter(|p| g2p::is_consonant(p)).count();
        let n_vow = phonemes.len().saturating_sub(n_cons).max(1);
        let cons_total_cap = sing_sec * 0.5;
        let cons_each = if n_cons > 0 {
            (cons_dur_target).min(cons_total_cap / n_cons as f64)
        } else {
            0.0
        };
        let vow_total = (sing_sec - cons_each * n_cons as f64).max(0.05);
        let vow_each = vow_total / n_vow as f64;

        let note_name = &note_name_cache[i];
        for ph in phonemes {
            let d = if g2p::is_consonant(ph) {
                cons_each
            } else {
                vow_each
            };
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

    // Trailing silence pad, mirroring the leading AP.
    ph_seq.push("AP".to_string());
    ph_dur.push(SEGMENT_PAD_SEC);
    note_seq.push("rest".to_string());
    note_dur.push(SEGMENT_PAD_SEC);
    note_seq_midi.push(0);

    // f0_seq: piecewise constant pitch following the note sequence. The
    // pipeline resamples this to its internal frame rate; we just need a
    // grid dense enough to capture every note boundary.
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
    // Fill unvoiced frames (rests, leading/trailing AP) with a
    // continuous carrier pitch. The reference fixtures keep f0 > 0
    // throughout the segment — silence is signalled by the phoneme
    // being "AP", not by f0 being zero. Zeroing f0 instead causes the
    // vocoder to emit subtle noise during the silence pads (the user's
    // "noise in silent parts" report). Forward-fill from the next
    // voiced frame for the leading pad, then back-fill from the
    // previous voiced frame for everything else.
    let first_voiced_idx = f0_samples.iter().position(|v| *v > 0.0);
    if let Some(first_idx) = first_voiced_idx {
        let leading_hz = f0_samples[first_idx];
        for v in f0_samples.iter_mut().take(first_idx) {
            *v = leading_hz;
        }
        let mut last_voiced = leading_hz;
        for v in f0_samples.iter_mut().skip(first_idx) {
            if *v > 0.0 {
                last_voiced = *v;
            } else {
                *v = last_voiced;
            }
        }
    }

    // Smooth f0 step jumps between adjacent voiced notes with a brief
    // ~40 ms linear portamento. The reference fixtures train the model
    // on real human pitch curves that always slide between notes, so
    // hard pitch steps at every syllable boundary push the acoustic
    // model into a regime it doesn't render cleanly. Skips frames
    // that are exactly equal to the previous (no slide needed).
    let portamento_frames = (0.040_f64 / f0_timestep).round() as usize;
    if portamento_frames >= 2 && f0_samples.len() > portamento_frames {
        let snapshot = f0_samples.clone();
        let mut last_change_idx = 0usize;
        let mut last_val = snapshot[0];
        for (i, &cur) in snapshot.iter().enumerate().skip(1) {
            if (cur - last_val).abs() > 0.5 {
                // Pitch change detected at index i. Linearly ramp the
                // previous `portamento_frames` from `last_val` (the
                // pre-change pitch) to `cur`.
                let start = i.saturating_sub(portamento_frames).max(last_change_idx);
                let span = i.saturating_sub(start);
                if span >= 1 {
                    for (offset, sample) in f0_samples[start..i].iter_mut().enumerate() {
                        let t = (offset + 1) as f64 / (span + 1) as f64;
                        *sample = last_val * (1.0 - t) + cur * t;
                    }
                }
                last_val = cur;
                last_change_idx = i;
            }
        }
    }

    // Vibrato: sinusoidal modulation of the f0 curve at ~5 Hz. The
    // slider scales the peak deviation up to ~20 cents at max —
    // anything wider sounds like pitch instability rather than
    // expressive vibrato. The pipeline keeps the curve in Hz so we
    // apply the cent offset multiplicatively: f * 2^(cents/1200).
    let vibrato_depth = params.vibrato.clamp(0.0, 1.0) as f64;
    if vibrato_depth > 0.001 {
        let max_cents = 20.0_f64;
        let rate_hz = 5.0_f64;
        let two_pi = std::f64::consts::TAU;
        for (i, v) in f0_samples.iter_mut().enumerate() {
            if *v > 0.0 {
                let t = i as f64 * f0_timestep;
                let cents = max_cents * vibrato_depth * (two_pi * rate_hz * t).sin();
                *v *= 2.0_f64.powf(cents / 1200.0);
            }
        }
    }

    // Gender curve drives the TIGER acoustic model's character knob
    // (`use_key_shift_embed` is the only other per-frame embed the YAML
    // enables; `breathiness`/`tension`/`voicing`/`energy` are disabled
    // on this model so we don't send those — they'd be silently
    // ignored). Convention: −1 darker, +1 brighter. The `timbre` chip
    // selects the position; the curve is constant across the segment.
    let curve_len = f0_samples.len();
    let gender_value = match params.timbre {
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

/// Pick a TIGER speaker based on the voice type. The acoustic config ships
/// 41 speakers; we map our six voice categories to the most-tonal subset
/// of the seven `tiger_*` voices that come bundled with .emb files.
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

/// MIDI note → "C4" / "D#5" / "Bb3" notation accepted by DiffSinger's
/// `note_seq`. Mirrors `note_name_to_midi`'s inverse semantics.
fn midi_to_diffsinger_note(midi: u8) -> String {
    const SHARP: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (midi as i32 / 12) - 1;
    let pc = midi as usize % 12;
    format!("{}{}", SHARP[pc], octave)
}

fn midi_to_hz(midi: u8) -> f64 {
    // A4 (MIDI 69) = 440 Hz.
    440.0 * (2.0_f64).powf((midi as f64 - 69.0) / 12.0)
}

/// Write stereo-interleaved f32 samples to a WAV file compatible with
/// `ClipSource::open_wav`. Delegates to the engine's `transcode_to_wav`
/// so the in-RAM and SVS-rendered code paths share one WAV writer.
pub fn write_stereo_wav(
    path: &std::path::Path,
    samples: &[f32],
    sample_rate: u32,
) -> Result<(), String> {
    resonance_audio::transcode_to_wav(path, samples, sample_rate)
}
