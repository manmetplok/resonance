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

use std::path::{Path, PathBuf};

use resonance_audio::types::MidiNote;
use resonance_music_theory::{g2p, VocalParams, VocalTimbre, VocalVoicebank};
use svs_poc::ds::{DsSegment, SampleCurve};

/// Meiji's per-token language id for English-prefixed phonemes. Lifted
/// from `voicebanks/meiji/files/languages.json` (`"en": 3`). Silence
/// markers (`AP`/`SP`) get `0` since they're un-prefixed in the dict.
const MEIJI_LANG_EN: i64 = 3;
const MEIJI_LANG_SILENCE: i64 = 0;
use svs_poc::pipeline::{self, PipelineArgs};
use svs_poc::stages::common::ExecutionProvider;

/// Resolved on-disk paths for one voicebank. The render pipeline only
/// needs the two YAML files (it loads everything else by reference from
/// inside them), plus the optional speaker id we send via `spk_embed`.
struct VoicebankPaths {
    acoustic_config: PathBuf,
    vocoder_config: PathBuf,
    /// `Some(name)` for multi-speaker banks (TIGER, Meiji); `None` for
    /// single-speaker banks (Lilia).
    speaker: Option<String>,
}

/// Find the on-disk paths for `voicebank`. Resolution order:
///   1. `RESONANCE_SVS_MODELS_DIR` env var (workspace override)
///   2. workspace-root `experiments/svs-poc/models/` (PoC default)
///
/// Returns `None` when the requested voicebank isn't installed —
/// callers treat that as "SVS unavailable, skip silently".
fn locate_voicebank(voicebank: VocalVoicebank, params: &VocalParams) -> Option<VoicebankPaths> {
    let roots: Vec<PathBuf> = std::iter::once(std::env::var_os("RESONANCE_SVS_MODELS_DIR"))
        .flatten()
        .map(PathBuf::from)
        .chain(std::iter::once(default_models_dir()))
        .collect();

    for root in roots {
        if let Some(paths) = try_voicebank(&root, voicebank, params) {
            return Some(paths);
        }
    }
    None
}

fn try_voicebank(
    root: &Path,
    voicebank: VocalVoicebank,
    params: &VocalParams,
) -> Option<VoicebankPaths> {
    match voicebank {
        VocalVoicebank::Tiger => {
            let acoustic = root.join("singer/extracted/dsacoustic/dsconfig.yaml");
            // TIGER ships its own bundled vocoder
            // (`tgm_hifigan.onnx`, r03) trained against the same mel
            // statistics as the acoustic model. A foreign vocoder
            // (`tgm_hifigan_v110.onnx`) produces noticeably rougher
            // audio because the mel-spectrogram statistics don't match.
            // Prefer the bundled vocoder, fall back to the generic one
            // (`models/vocoder/dsvocoder/`) for setups that only have
            // the generic version installed.
            let bundled = root.join("singer/extracted/dsvocoder/vocoder.yaml");
            let generic = root.join("vocoder/dsvocoder/vocoder.yaml");
            let vocoder = if bundled.exists() { bundled } else { generic };
            if acoustic.exists() && vocoder.exists() {
                Some(VoicebankPaths {
                    acoustic_config: acoustic,
                    vocoder_config: vocoder,
                    speaker: Some(params.singer.speaker_id().to_string()),
                })
            } else {
                None
            }
        }
        VocalVoicebank::Lilia => {
            let acoustic = root.join("voicebanks/lilia/dsconfig.yaml");
            let vocoder = root.join("voicebanks/lilia/dsvocoder/vocoder.yaml");
            if acoustic.exists() && vocoder.exists() {
                Some(VoicebankPaths {
                    acoustic_config: acoustic,
                    vocoder_config: vocoder,
                    // Lilia is single-speaker.
                    speaker: None,
                })
            } else {
                None
            }
        }
        VocalVoicebank::Meiji => {
            let acoustic = root.join("voicebanks/meiji/configs/configs/dsconfig.yaml");
            let vocoder = root.join("voicebanks/meiji/configs/configs/dsvocoder/vocoder.yaml");
            if acoustic.exists() && vocoder.exists() {
                Some(VoicebankPaths {
                    acoustic_config: acoustic,
                    vocoder_config: vocoder,
                    speaker: Some(params.singer_meiji.speaker_id().to_string()),
                })
            } else {
                None
            }
        }
    }
}

/// Workspace-relative default — the SVS PoC ships its model dir at
/// `experiments/svs-poc/models/`. Anchored against the binary's
/// `CARGO_MANIFEST_DIR` so it resolves from a `cargo run` in any subdir.
fn default_models_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../experiments/svs-poc/models")
}

/// Replace any G2P-emitted phonemes that are missing from the chosen
/// voicebank's phoneme dict with the closest acceptable substitute.
/// Sending an unknown phoneme would land on token-id 0 (the `<PAD>`
/// reservation), which produces a downstream tensor-shape mismatch in
/// some voicebanks (Lilia's FastSpeech2 graph throws `Mul` broadcast
/// errors when PADs appear mid-sequence).
fn substitute_phoneme(voicebank: VocalVoicebank, ph: &'static str) -> &'static str {
    match voicebank {
        VocalVoicebank::Tiger => ph,
        VocalVoicebank::Lilia => match ph {
            // Lilia's MM phoneme set covers all of ARPAbet *except* the
            // voiced labiodental fricative `v`. Substitute its closest
            // English equivalent: the voiceless `f` (same place + manner
            // of articulation, just unvoiced). Singers won't notice in
            // most words; the alternative `b` would change place and
            // sound more obviously wrong.
            "v" => "f",
            other => other,
        },
        // Meiji uses language-prefixed ARPAbet. Substitution happens
        // *before* prefixing in build_segment, so we don't need to
        // touch any symbols here — the full English set is present.
        VocalVoicebank::Meiji => ph,
    }
}

/// Apply the voicebank's per-symbol naming convention. Meiji namespaces
/// every English phoneme with `en/` (e.g. `ah` → `en/ah`) but a small
/// shared inventory (`AP`, `SP`, `hh`, `cl`, ...) is left unprefixed so
/// it works across every language Meiji supports. TIGER and Lilia use
/// bare ARPAbet symbols.
fn voicebank_phoneme_name(voicebank: VocalVoicebank, ph: &str) -> String {
    match voicebank {
        VocalVoicebank::Tiger | VocalVoicebank::Lilia => ph.to_string(),
        VocalVoicebank::Meiji => {
            if meiji_is_universal(ph) {
                ph.to_string()
            } else {
                format!("en/{ph}")
            }
        }
    }
}

/// Phonemes that live in Meiji's "no-prefix" bucket (silence markers
/// and a handful of language-agnostic consonants). Meiji's `en/` set
/// notably lacks `en/hh`; the universal `hh` covers it.
fn meiji_is_universal(ph: &str) -> bool {
    matches!(ph, "AP" | "SP" | "hh" | "cl" | "ban" | "vf")
}

/// Per-token language id Meiji's `languages` ONNX input expects. `0`
/// for silence markers and the universal-consonant bucket; `3` for
/// English (matches Meiji's `languages.json: "en": 3`). Returns `None`
/// for voicebanks that don't accept a `languages` input.
fn voicebank_language_id(voicebank: VocalVoicebank, ph: &str) -> Option<i64> {
    match voicebank {
        VocalVoicebank::Tiger | VocalVoicebank::Lilia => None,
        VocalVoicebank::Meiji => Some(if meiji_is_universal(ph) {
            MEIJI_LANG_SILENCE
        } else {
            MEIJI_LANG_EN
        }),
    }
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
#[allow(dead_code)]
pub fn render_vocal_clip(
    notes: &[MidiNote],
    params: &VocalParams,
    ticks_per_quarter: u32,
    bpm: f32,
    engine_sample_rate: u32,
) -> Result<Option<RenderedVocal>, String> {
    render_vocal_clip_with_lyrics(notes, params, &[], ticks_per_quarter, bpm, engine_sample_rate)
}

/// Like [`render_vocal_clip`] but also accepts a per-note lyric slice
/// (parallel to `notes`). Notes whose lyric equals the OpenUtau slur
/// marker (`"+"`) get treated as melisma continuations of the previous
/// syllable instead of consuming a fresh phoneme list. An empty
/// `lyrics` slice is equivalent to the legacy "every note is its own
/// syllable" mode.
pub fn render_vocal_clip_with_lyrics(
    notes: &[MidiNote],
    params: &VocalParams,
    lyrics: &[String],
    ticks_per_quarter: u32,
    bpm: f32,
    engine_sample_rate: u32,
) -> Result<Option<RenderedVocal>, String> {
    if notes.is_empty() {
        return Ok(None);
    }
    let Some(paths) = locate_voicebank(params.voicebank, params) else {
        return Ok(None);
    };

    let segment = build_segment(notes, params, lyrics, ticks_per_quarter, bpm);
    if segment.ph_seq.is_empty() {
        return Ok(None);
    }

    let args = PipelineArgs {
        ds_file: PathBuf::new(),
        acoustic_config: paths.acoustic_config,
        vocoder_config: paths.vocoder_config,
        out: PathBuf::new(),
        execution_provider: ExecutionProvider::Cpu,
        device_index: 0,
        speaker: paths.speaker,
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
    lyrics: &[String],
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
    let (raw_phonemes, is_word_end) =
        g2p::phonemes_for_draft_with_word_boundaries(&params.draft);
    let syllable_phonemes: Vec<Vec<&'static str>> = raw_phonemes
        .into_iter()
        .map(|s| s.into_iter().map(|p| substitute_phoneme(params.voicebank, p)).collect())
        .collect();
    // Optional word-boundary SP injection. Off by default (the
    // reference DiffSinger fixtures intentionally flow phonemes
    // continuously). Set RESONANCE_WORD_BOUNDARY_SP_MS=N to insert
    // ~N ms of SP at the end of each word's last syllable for an A/B
    // listening test. Practical range: 20-80 ms.
    let word_boundary_sp_sec: f64 = std::env::var("RESONANCE_WORD_BOUNDARY_SP_MS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|ms| (ms / 1000.0).max(0.0).min(0.2))
        .unwrap_or(0.0);
    // Optional stop-closure pre-silence. English stops (B/P/T/D/K/G)
    // have an inherent closure phase the model handles internally;
    // explicit `cl` insertion will most likely double up the closure
    // and sound worse. Off by default — set RESONANCE_STOP_CLOSURE_MS=N
    // to prepend ~N ms of `cl` before each stop consonant for an A/B
    // listening test. Practical range: 5-20 ms.
    let stop_closure_sec: f64 = std::env::var("RESONANCE_STOP_CLOSURE_MS")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(|ms| (ms / 1000.0).max(0.0).min(0.05))
        .unwrap_or(0.0);
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
    // Per-token language ids (parallel to ph_seq). Only populated when
    // the voicebank exposes a `languages` ONNX input (Meiji); empty for
    // TIGER and Lilia, which the pipeline interprets as "skip this
    // input".
    let mut languages: Vec<i64> = Vec::new();
    // Per-entry note metadata, parallel to ph_dur. Drives the
    // dynamic tension curve (per-syllable velocity) and the vibrato
    // gate (which only applies vibrato to longer notes after a brief
    // onset delay).
    //
    // For each phoneme entry of a note: store the note's velocity,
    // the note's total sing duration, and how far into the note the
    // entry starts. AP entries (rests) get sentinel values that
    // disable both the tension modulator and vibrato.
    let mut entry_note_velocity: Vec<f32> = Vec::new();
    let mut entry_note_total_sec: Vec<f64> = Vec::new();
    let mut entry_note_start_offset: Vec<f64> = Vec::new();

    // Leading silence pad. The hand-crafted reference fixtures all
    // start with a 0.3 s `AP` so the model has time to ramp up cleanly;
    // skipping this produces an attack click on the first phoneme.
    ph_seq.push(voicebank_phoneme_name(params.voicebank, "AP"));
    ph_dur.push(SEGMENT_PAD_SEC);
    note_seq.push("rest".to_string());
    note_dur.push(SEGMENT_PAD_SEC);
    note_seq_midi.push(0);
    if let Some(id) = voicebank_language_id(params.voicebank, "AP") {
        languages.push(id);
    }
    entry_note_velocity.push(0.0);
    entry_note_total_sec.push(0.0);
    entry_note_start_offset.push(0.0);

    let note_name_cache: Vec<String> = notes
        .iter()
        .map(|n| midi_to_diffsinger_note(n.note))
        .collect();

    // Per-note slur lookup. OpenUtau-style: a lyric of `+` (or `-`)
    // means "continue the previous syllable's vowel" — that note
    // consumes no fresh phoneme list and inherits the last vowel of
    // the prior non-slur note. Empty `lyrics` disables slurring
    // (legacy 1:1 mapping).
    let is_slur_note = |i: usize| -> bool {
        lyrics
            .get(i)
            .map(|l| {
                let s = l.trim();
                s == "+" || s == "-"
            })
            .unwrap_or(false)
    };
    // Map each note index to the syllable phoneme list it should
    // sing. Slur notes inherit the previous non-slur note's index;
    // non-slur notes count their own position among non-slurs.
    let syllable_idx_per_note: Vec<usize> = {
        let mut out = Vec::with_capacity(notes.len());
        let mut next_syl: usize = 0;
        let mut last_syl: usize = 0;
        for i in 0..notes.len() {
            if is_slur_note(i) {
                out.push(last_syl);
            } else {
                out.push(next_syl);
                last_syl = next_syl;
                next_syl += 1;
            }
        }
        out
    };

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

        // Slur notes sing only the previous syllable's vowel for the
        // whole slot — no consonants, no new attack. This matches
        // OpenUtau / DiffSinger's slur semantics: the model gets a
        // single sustained vowel at the new pitch.
        let is_slur = is_slur_note(i);
        let syllable_idx = syllable_idx_per_note[i];
        let fallback = vec!["ah"];
        let phonemes_owned: Vec<&'static str> = if is_slur {
            // Pick the vowel of the previous (non-slur) syllable to
            // carry over. Falls back to the source syllable's last
            // phoneme when it has no vowel at all (degenerate input).
            let prev = syllable_phonemes
                .get(syllable_idx)
                .map(|v| v.as_slice())
                .unwrap_or(&fallback);
            let vowel = prev
                .iter()
                .rev()
                .find(|p| !g2p::is_consonant(p))
                .or_else(|| prev.last())
                .copied()
                .unwrap_or("ah");
            vec![vowel]
        } else {
            syllable_phonemes
                .get(syllable_idx)
                .cloned()
                .unwrap_or_else(|| fallback.clone())
        };
        let phonemes: &[&'static str] = &phonemes_owned;

        // Word-boundary SP: when this syllable is the LAST one of its
        // word (and the next syllable starts a new word), reserve a
        // small silence at the end of the singing slot. The reference
        // DiffSinger fixtures don't insert SP between words, so this
        // is opt-in — env-var-gated for A/B testing. Suppressed for
        // slur notes — a melisma never sits on a word boundary.
        let inject_sp = !is_slur
            && word_boundary_sp_sec > 0.0
            && is_word_end.get(syllable_idx).copied().unwrap_or(false)
            && syllable_idx + 1 < syllable_phonemes.len();
        let sp_sec = if inject_sp {
            word_boundary_sp_sec.min(sing_sec * 0.3)
        } else {
            0.0
        };
        let phon_sing_sec = (sing_sec - sp_sec).max(0.05);

        // Split `phon_sing_sec` across phonemes: each consonant gets
        // up to `cons_dur_target`, capped so consonants never eat
        // more than half the syllable. The vowel(s) absorb the
        // remainder evenly.
        let n_cons = phonemes.iter().filter(|p| g2p::is_consonant(p)).count();
        let n_vow = phonemes.len().saturating_sub(n_cons).max(1);
        let cons_total_cap = phon_sing_sec * 0.5;
        let cons_each = if n_cons > 0 {
            (cons_dur_target).min(cons_total_cap / n_cons as f64)
        } else {
            0.0
        };
        let vow_total = (phon_sing_sec - cons_each * n_cons as f64).max(0.05);
        let vow_each = vow_total / n_vow as f64;

        let note_name = &note_name_cache[i];
        // Track per-phoneme offset within this note for the metadata
        // arrays (consumed below by the dynamic tension curve and
        // vibrato gate).
        let mut offset_in_note: f64 = 0.0;
        for (ph_idx, ph) in phonemes.iter().enumerate() {
            // Optional stop-closure: prepend `cl` before a stop
            // consonant (B/P/T/D/K/G) to manufacture a brief closure
            // phase. Steals time from the stop's own slot to keep
            // the syllable's total duration unchanged. Skipped on
            // syllable-initial consonants — those have a natural
            // closure from the preceding silence/vowel.
            let is_stop = matches!(*ph, "b" | "p" | "t" | "d" | "k" | "g");
            if stop_closure_sec > 0.0 && is_stop && ph_idx > 0 {
                let cl_dur = stop_closure_sec.min(cons_each * 0.4);
                ph_seq.push(voicebank_phoneme_name(params.voicebank, "cl"));
                ph_dur.push(cl_dur);
                note_seq.push(note_name.clone());
                note_dur.push(cl_dur);
                note_seq_midi.push(n.note as i32);
                if let Some(id) = voicebank_language_id(params.voicebank, "cl") {
                    languages.push(id);
                }
            }
            let mut d = if g2p::is_consonant(ph) {
                cons_each
            } else {
                vow_each
            };
            // Subtract the borrowed closure time so total syllable
            // duration stays the same.
            if stop_closure_sec > 0.0 && is_stop && ph_idx > 0 {
                d = (d - stop_closure_sec.min(cons_each * 0.4)).max(0.005);
            }
            ph_seq.push(voicebank_phoneme_name(params.voicebank, ph));
            ph_dur.push(d);
            note_seq.push(note_name.clone());
            note_dur.push(d);
            note_seq_midi.push(n.note as i32);
            if let Some(id) = voicebank_language_id(params.voicebank, ph) {
                languages.push(id);
            }
            entry_note_velocity.push(n.velocity);
            entry_note_total_sec.push(sing_sec);
            entry_note_start_offset.push(offset_in_note);
            offset_in_note += d;
        }
        if sp_sec > 0.0 {
            // Insert SP within the same note's slot — the syllable's
            // pitch carries through the brief silence.
            ph_seq.push(voicebank_phoneme_name(params.voicebank, "SP"));
            ph_dur.push(sp_sec);
            note_seq.push(note_name.clone());
            note_dur.push(sp_sec);
            note_seq_midi.push(n.note as i32);
            if let Some(id) = voicebank_language_id(params.voicebank, "SP") {
                languages.push(id);
            }
            entry_note_velocity.push(n.velocity);
            entry_note_total_sec.push(sing_sec);
            entry_note_start_offset.push(offset_in_note);
            // No further entries belong to this note after the SP, so
            // we don't carry the cumulative offset forward.
        }

        if ap_sec > 0.0 {
            ph_seq.push(voicebank_phoneme_name(params.voicebank, "AP"));
            ph_dur.push(ap_sec);
            note_seq.push("rest".to_string());
            note_dur.push(ap_sec);
            note_seq_midi.push(0);
            if let Some(id) = voicebank_language_id(params.voicebank, "AP") {
                languages.push(id);
            }
            entry_note_velocity.push(0.0);
            entry_note_total_sec.push(0.0);
            entry_note_start_offset.push(0.0);
        }
    }

    // Trailing silence pad, mirroring the leading AP.
    ph_seq.push(voicebank_phoneme_name(params.voicebank, "AP"));
    ph_dur.push(SEGMENT_PAD_SEC);
    note_seq.push("rest".to_string());
    note_dur.push(SEGMENT_PAD_SEC);
    note_seq_midi.push(0);
    if let Some(id) = voicebank_language_id(params.voicebank, "AP") {
        languages.push(id);
    }
    entry_note_velocity.push(0.0);
    entry_note_total_sec.push(0.0);
    entry_note_start_offset.push(0.0);

    // f0_seq: piecewise constant pitch following the note sequence. The
    // pipeline resamples this to its internal frame rate; we just need a
    // grid dense enough to capture every note boundary.
    let f0_timestep = 0.005_f64;
    let total_sec: f64 = ph_dur.iter().sum();
    let n_samples = (total_sec / f0_timestep).ceil() as usize + 1;
    let mut f0_samples = Vec::with_capacity(n_samples);
    // Parallel per-frame metadata for the dynamic tension curve and
    // vibrato gate. Filled in lockstep with `f0_samples` so each
    // frame knows its parent note's velocity, total duration, and
    // how far we are into the note.
    let mut frame_velocity: Vec<f32> = Vec::with_capacity(n_samples);
    let mut frame_note_total_sec: Vec<f64> = Vec::with_capacity(n_samples);
    let mut frame_in_note_sec: Vec<f64> = Vec::with_capacity(n_samples);
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
        // Per-frame metadata: note velocity / duration / elapsed.
        let vel = entry_note_velocity.get(idx).copied().unwrap_or(0.0);
        let nts = entry_note_total_sec.get(idx).copied().unwrap_or(0.0);
        let entry_start_t = accum - note_dur[idx];
        let elapsed_in_entry = (t - entry_start_t).max(0.0);
        let offset = entry_note_start_offset.get(idx).copied().unwrap_or(0.0);
        frame_velocity.push(vel);
        frame_note_total_sec.push(nts);
        frame_in_note_sec.push(offset + elapsed_in_entry);
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
    // linear portamento. The reference fixtures train the model on
    // real human pitch curves that always slide between notes, so
    // hard pitch steps at every syllable boundary push the acoustic
    // model into a regime it doesn't render cleanly. The user controls
    // the slide duration (10..200 ms in the inspector); 0 disables
    // portamento entirely (hard step, only useful for stylistic
    // hard-attack effects). Skips frames that are exactly equal to
    // the previous (no slide needed).
    let portamento_sec = (params.portamento_ms.clamp(0.0, 250.0) as f64) / 1000.0;
    let portamento_frames = (portamento_sec / f0_timestep).round() as usize;
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

    // Vibrato: sinusoidal modulation of the f0 curve. Rate (4–7 Hz)
    // is user-controlled via `vibrato_rate`; depth scales peak
    // deviation up to ~20 cents at max. Real singers don't apply
    // vibrato to short syllables and let it ramp in after the
    // consonant attack, so we gate two ways:
    //   1. Skip notes whose total sing duration is below
    //      `VIBRATO_MIN_NOTE_SEC` — too short for vibrato to make
    //      musical sense (it'd just sound like a wobble on the
    //      consonant).
    //   2. Within longer notes, fade vibrato in over
    //      `VIBRATO_ONSET_SEC` after the note's start so the
    //      consonant attack stays clean.
    const VIBRATO_MIN_NOTE_SEC: f64 = 0.35;
    const VIBRATO_ONSET_SEC: f64 = 0.15;
    let vibrato_depth = params.vibrato.clamp(0.0, 1.0) as f64;
    if vibrato_depth > 0.001 {
        let max_cents = 20.0_f64;
        let rate_hz = params.vibrato_rate.clamp(2.0, 10.0) as f64;
        let two_pi = std::f64::consts::TAU;
        for (i, v) in f0_samples.iter_mut().enumerate() {
            if *v <= 0.0 {
                continue;
            }
            let note_dur_s = frame_note_total_sec.get(i).copied().unwrap_or(0.0);
            if note_dur_s < VIBRATO_MIN_NOTE_SEC {
                continue;
            }
            let elapsed = frame_in_note_sec.get(i).copied().unwrap_or(0.0);
            let onset_gain = (elapsed / VIBRATO_ONSET_SEC).clamp(0.0, 1.0);
            if onset_gain <= 0.0 {
                continue;
            }
            let t = i as f64 * f0_timestep;
            let cents =
                max_cents * vibrato_depth * onset_gain * (two_pi * rate_hz * t).sin();
            *v *= 2.0_f64.powf(cents / 1200.0);
        }
    }

    // Gender curve maps to the acoustic model's `gender` ONNX input,
    // which shifts formants brighter / darker (range [-1, +1], 0 =
    // neutral). The dsconfig's `use_key_shift_embed` flag is unrelated
    // — that's about training-time pitch-shift augmentation, not a
    // runtime input. Other per-frame curves (`energy`, `breathiness`,
    // `voicing`, `tension`) aren't accepted by the TIGER model and are
    // left as `SampleCurve::default()`. The `timbre` chip selects a
    // landmark on the brightness axis; the curve is constant across
    // the segment so the formant character stays consistent.
    //
    // Empirically-tuned band, characterised against TIGER (the
    // tightest of the three voicebanks): the negative side has a hard
    // ceiling around `-0.20`, and the positive side starts losing
    // intelligibility past about `+0.35` — whisper transcribes a
    // `+0.50` Bright TIGER as "my my my" instead of the test lyric.
    // Lilia and Meiji are robust across the band. If you widen, do
    // it positive-side only and re-run the sweep harness to confirm
    // intelligibility doesn't collapse.
    let curve_len = f0_samples.len();
    let gender_value = match params.timbre {
        VocalTimbre::Warm => -0.15,
        VocalTimbre::Edged => -0.05,
        VocalTimbre::Airy => 0.20,
        VocalTimbre::Bright => 0.30,
    };
    let gender = SampleCurve {
        samples: vec![gender_value; curve_len],
        timestep: f0_timestep,
    };

    // NOTE on `velocity`: TIGER does accept a per-frame `velocity`
    // input, but in DiffSinger semantics velocity is a *phoneme-
    // duration* multiplier (>1.0 shortens, <1.0 lengthens), not the
    // attack-strength knob it sounds like. Feeding non-1.0 values
    // smeared the rendered audio down to ~-60 dB during testing, so
    // we deliberately leave it as default (the pipeline fills with
    // 1.0 internally). The per-syllable velocities computed by
    // `derive_vocal` still drive MIDI clip dynamics; bridging them
    // into the SVS model needs a different parameter (and probably
    // training-set characterisation) than this knob provides.

    // Tension curve maps to the `tension` ONNX input on voicebanks
    // that expose it (Lilia, Meiji). Range [-1, +1]: -1 = relaxed /
    // breathy delivery, 0 = neutral, +1 = compressed / belted.
    // TIGER doesn't accept tension (the pipeline's `flags.tension`
    // will be false for that voicebank, so the curve is ignored).
    //
    // Two modulators add per-frame movement to the slider baseline:
    //   - Velocity: strong-beat syllables (higher per-note velocity
    //     from `derive_vocal`) push tension up; weak ones push down.
    //   - Contour: notes near the top of the section's pitch range
    //     push tension up (singers belt at the top of their range);
    //     notes near the bottom push down.
    // Each modulator's strength is its own slider in [0, 1] so the
    // user can dial in either, both, or neither.
    let tension = if voicebank_supports_tension(params.voicebank) {
        let base = params.tension.clamp(-1.0, 1.0) as f64;
        let vel_amount = params.tension_velocity_amount.clamp(0.0, 1.0) as f64;
        let contour_amount = params.tension_contour_amount.clamp(0.0, 1.0) as f64;
        // Section pitch range, used to normalise the contour
        // contribution. Use the f0 sample range (excluding silence
        // fill) so the modulation is per-section rather than global.
        let (mut min_hz, mut max_hz) = (f64::INFINITY, 0.0_f64);
        for (i, &v) in f0_samples.iter().enumerate() {
            if frame_note_total_sec.get(i).copied().unwrap_or(0.0) > 0.0 && v > 0.0 {
                if v < min_hz {
                    min_hz = v;
                }
                if v > max_hz {
                    max_hz = v;
                }
            }
        }
        let mid_hz = (min_hz + max_hz) * 0.5;
        let half_range_hz = ((max_hz - min_hz) * 0.5).max(1.0);
        let mut samples = Vec::with_capacity(curve_len);
        for i in 0..curve_len {
            // Velocity modulation: derive_vocal's neutral velocity is
            // ~0.78 with strong beats around 0.86. Map to roughly
            // [-1, +1] around neutral, then scale by amount and
            // contribute up to ±0.5.
            let vel = frame_velocity.get(i).copied().unwrap_or(0.0) as f64;
            let vel_mod = if vel > 0.0 {
                ((vel - 0.78) / 0.22).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            // Pitch contour modulation: position within section's f0
            // range, mapped to [-1, +1]. Silence frames contribute 0.
            let pitch = f0_samples[i];
            let in_voiced =
                frame_note_total_sec.get(i).copied().unwrap_or(0.0) > 0.0 && pitch > 0.0;
            let pitch_mod = if in_voiced {
                ((pitch - mid_hz) / half_range_hz).clamp(-1.0, 1.0)
            } else {
                0.0
            };
            let t = (base + vel_amount * vel_mod * 0.5 + contour_amount * pitch_mod * 0.5)
                .clamp(-1.0, 1.0);
            samples.push(t);
        }
        SampleCurve {
            samples,
            timestep: f0_timestep,
        }
    } else {
        SampleCurve::default()
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
        tension,
        languages,
    }
}

/// Whether the voicebank's acoustic model accepts a `tension` per-frame
/// curve. TIGER doesn't; Lilia and Meiji do. Cheaper than introspecting
/// the ONNX inputs at every render.
fn voicebank_supports_tension(voicebank: VocalVoicebank) -> bool {
    match voicebank {
        VocalVoicebank::Tiger => false,
        VocalVoicebank::Lilia | VocalVoicebank::Meiji => true,
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
