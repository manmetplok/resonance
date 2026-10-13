//! Orchestrates the .ds → WAV pipeline. Stages run in order:
//!   linguistic → (dur) → (pitch) → (variance) → acoustic → vocoder
//! gated on dsconfig flags + CLI overrides. The smoke-test path only exercises
//! acoustic + vocoder (combined acoustic ONNX, .ds supplies ph_dur and f0_seq directly).

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::audio::{mix_into_timeline, write_mono_f32_wav};
use crate::config::{load_acoustic, load_phoneme_dict, load_vocoder, AcousticConfig};
use crate::ds::{load_ds_file, DsSegment};
use crate::stages::acoustic::{AcousticFlags, AcousticStage, PreprocessedAcoustic};
use crate::stages::common::ExecutionProvider;
use crate::stages::session_cache;
use crate::stages::vocoder::VocoderStage;

#[derive(Debug, Clone)]
pub struct PipelineArgs {
    pub ds_file: std::path::PathBuf,
    pub acoustic_config: std::path::PathBuf,
    pub vocoder_config: std::path::PathBuf,
    pub out: std::path::PathBuf,
    pub execution_provider: ExecutionProvider,
    pub device_index: i32,
    pub speaker: Option<String>,
    pub speedup: i32,
    pub depth: i32,
    /// Seed of the acoustic model's noise (segment `i` of a
    /// [`render_segments`] call uses `seed + i`).
    pub seed: u64,
}

pub fn run(args: &PipelineArgs) -> Result<RunSummary> {
    let segments = load_ds_file(&args.ds_file)?;
    tracing::info!(
        "loaded {} segment(s) from {}",
        segments.len(),
        args.ds_file.display()
    );

    let rendered = render_segments(&segments, args)?;

    write_mono_f32_wav(&args.out, &rendered.samples, rendered.sample_rate)?;

    Ok(RunSummary {
        segments_rendered: rendered.segments_rendered,
        total_samples: rendered.samples.len(),
        sample_rate: rendered.sample_rate,
        per_segment_seconds: rendered.per_segment_seconds,
    })
}

/// In-memory output of [`render_segments`]: a mono float waveform plus
/// timing metadata. Callers can write the samples to disk themselves
/// (see [`write_mono_f32_wav`]) or push them straight into a host audio
/// engine.
#[derive(Debug, Clone)]
pub struct RenderedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub segments_rendered: usize,
    pub per_segment_seconds: Vec<(usize, f64)>,
}

/// Run the acoustic + vocoder pipeline on a programmatically-built list
/// of [`DsSegment`]s. Mirrors [`run`] but skips both ends of the
/// filesystem dance: callers supply the score directly and receive the
/// mixed mono waveform in memory. Segment `i` is rendered with seed
/// `args.seed + i` (see [`Renderer::render`]).
pub fn render_segments(segments: &[DsSegment], args: &PipelineArgs) -> Result<RenderedAudio> {
    let mut renderer = Renderer::load(args)?;
    let sample_rate = renderer.sample_rate();
    let mut rendered: Vec<(i64, Vec<f32>)> = Vec::with_capacity(segments.len());
    let mut perf = Vec::with_capacity(segments.len());

    for (i, seg) in segments.iter().enumerate() {
        tracing::info!("segment {}/{}", i + 1, segments.len());
        let t0 = Instant::now();
        let offset_samples = (seg.offset * sample_rate as f64).ceil() as i64;
        let waveform = renderer.render(seg, args.seed.wrapping_add(i as u64))?;
        let elapsed = t0.elapsed();
        perf.push((i, elapsed));
        tracing::info!(
            "segment {} rendered in {:.2}s ({} samples)",
            i + 1,
            elapsed.as_secs_f64(),
            waveform.len()
        );
        rendered.push((offset_samples, waveform));
    }

    let timeline = mix_into_timeline(&rendered);

    Ok(RenderedAudio {
        segments_rendered: segments.len(),
        samples: timeline,
        sample_rate,
        per_segment_seconds: perf
            .into_iter()
            .map(|(i, d)| (i, d.as_secs_f64()))
            .collect(),
    })
}

/// The acoustic + vocoder models of one bank, loaded once and reused for
/// any number of segments. The sessions come from the process-wide
/// [`session_cache`], so a later render of the same bank (a re-render of
/// one edited unit) does not load them again.
pub struct Renderer {
    acoustic_cfg: AcousticConfig,
    phoneme_map: HashMap<String, i64>,
    frame_length: f64,
    hop_size: usize,
    sample_rate: u32,
    speedup: i32,
    depth: f32,
    acoustic: Arc<Mutex<AcousticStage>>,
    acoustic_flags: AcousticFlags,
    vocoder: Arc<Mutex<VocoderStage>>,
    speaker_embeddings: SpeakerEmbeddings,
    selected_speaker: Option<String>,
}

impl Renderer {
    /// Load the acoustic and vocoder sessions `args` names.
    pub fn load(args: &PipelineArgs) -> Result<Self> {
        let acoustic_cfg = load_acoustic(&args.acoustic_config)?;
        let vocoder_cfg = load_vocoder(&args.vocoder_config)?;
        let phoneme_map = load_phoneme_dict(&acoustic_cfg.phonemes_path)?;

        // Frame interval the acoustic / vocoder stages expect.
        let frame_length = vocoder_cfg.hop_size as f64 / vocoder_cfg.sample_rate as f64;

        let mut speedup = args.speedup;
        if !(1..=1000).contains(&speedup) {
            tracing::warn!("speedup {} out of [1,1000]; clamping to 10", speedup);
            speedup = 10;
        }
        // Shallow-diffusion depth handling. Two formats exist:
        //   - Old: integer step count (max_depth like 1000), CLI --depth is integer steps,
        //     constraint: depth ≤ max_depth and depth % speedup == 0
        //   - New (variable depth): fractional 0–1 (max_depth like 0.6). Depth is passed to
        //     the model as a float; speedup/step constraints don't apply.
        // We pick the format heuristically: if max_depth < 5 we treat it as fractional.
        let mut depth = args.depth as f32;
        if acoustic_cfg.use_shallow_diffusion && acoustic_cfg.max_depth >= 0.0 {
            let is_fractional = acoustic_cfg.max_depth > 0.0 && acoustic_cfg.max_depth < 5.0;
            if is_fractional {
                // CLI --depth defaults to 1000 which is meaningless here; cap at max_depth.
                depth = depth.min(acoustic_cfg.max_depth).max(0.0);
                if depth >= 1.0 {
                    depth = acoustic_cfg.max_depth;
                }
            } else {
                // Cap at max_depth, then quantise to a multiple of `speedup`
                // so the PNDM step count comes out as an integer. The
                // quantisation MUST run regardless of whether `depth >
                // max_depth` — otherwise a request like `--depth 350
                // --speedup 100` with `max_depth = 1000` leaves depth at
                // 350, violating the documented `depth % speedup == 0`
                // invariant and producing the wrong PNDM step count for
                // older voicebanks.
                depth = depth.min(acoustic_cfg.max_depth);
                depth = (depth / speedup as f32).floor() * speedup as f32;
            }
        }

        let group = args
            .acoustic_config
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .to_path_buf();
        let acoustic = session_cache::get_or_load(
            &group,
            &acoustic_cfg.acoustic_model,
            args.execution_provider,
            || {
                let stage = AcousticStage::load(
                    &acoustic_cfg.acoustic_model,
                    args.execution_provider,
                    args.device_index,
                )?;
                tracing::info!("acoustic features: {:?}", stage.supported_features());
                Ok(stage)
            },
        )?;
        let acoustic_flags = *lock(&acoustic).supported_features();

        // Vocoder runs on CPU regardless of execution provider for the acoustic model. Mirrors
        // Jobsecond's choice: most vocoders are CPU-bound matmul + 1D conv, GPU dispatch overhead
        // dominates. Users can change this trivially if they want.
        let vocoder = session_cache::get_or_load(
            &group,
            &vocoder_cfg.model_path,
            ExecutionProvider::Cpu,
            || {
                VocoderStage::load(
                    &vocoder_cfg.model_path,
                    ExecutionProvider::Cpu,
                    args.device_index,
                    vocoder_cfg.num_mel_bins as usize,
                    vocoder_cfg.hop_size as usize,
                )
            },
        )?;

        let speaker_embeddings = load_speaker_embeddings(&acoustic_cfg)?;
        let selected_speaker = args
            .speaker
            .clone()
            .or_else(|| acoustic_cfg.speakers.first().cloned());
        Ok(Self {
            acoustic_cfg,
            phoneme_map,
            frame_length,
            hop_size: vocoder_cfg.hop_size as usize,
            sample_rate: vocoder_cfg.sample_rate as u32,
            speedup,
            depth,
            acoustic,
            acoustic_flags,
            vocoder,
            speaker_embeddings,
            selected_speaker,
        })
    }

    /// Whether the acoustic model and the vocoder render repeatably: every
    /// random op of both reads seeded noise (see [`crate::stages::noise`]).
    pub fn repeatable(&self) -> bool {
        lock(&self.acoustic).model.repeatable && lock(&self.vocoder).model.repeatable
    }

    /// The vocoder's output rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Seconds per acoustic frame (`hop_size / sample_rate`).
    pub fn frame_sec(&self) -> f64 {
        self.frame_length
    }

    /// Render one segment to mono audio. `seed` fills the acoustic
    /// model's and the vocoder's noise ([`crate::stages::noise`]): the
    /// same segment and seed render the same samples.
    pub fn render(&mut self, seg: &DsSegment, seed: u64) -> Result<Vec<f32>> {
        let mut pd = preprocess_acoustic(
            seg,
            &self.phoneme_map,
            self.frame_length,
            &self.acoustic_cfg,
            &self.acoustic_flags,
            &self.speaker_embeddings,
            self.selected_speaker.as_deref(),
        )?;
        let n_frames = pd.f0.len();
        let mut mel = lock(&self.acoustic).infer(&mut pd, self.speedup as i64, self.depth, seed)?;
        let mut waveform =
            lock(&self.vocoder).infer(&mut mel, &pd.f0, seed ^ 0x564f_434f_4445_5221)?;
        // Vocoder waveform length is typically n_frames * hop_size, but some vocoders return
        // shorter/longer. Trim very long results to a sensible cap.
        let expected = n_frames * self.hop_size;
        if waveform.len() > expected + self.hop_size {
            waveform.truncate(expected);
        }
        Ok(waveform)
    }
}

/// The files a render of the bank whose acoustic config is
/// `acoustic_config` (with vocoder config `vocoder_config`) reads: the
/// configs, the phoneme dictionary and both models — what a render key
/// digests ([`crate::stages::common::files_digest`]) so that replacing a
/// model re-renders. A config that does not parse contributes itself only.
pub fn render_files(
    acoustic_config: &std::path::Path,
    vocoder_config: Option<&std::path::Path>,
) -> Vec<std::path::PathBuf> {
    let mut files = vec![acoustic_config.to_path_buf()];
    if let Ok(cfg) = load_acoustic(acoustic_config) {
        files.push(cfg.phonemes_path);
        files.push(cfg.acoustic_model);
    }
    if let Some(vocoder) = vocoder_config {
        files.push(vocoder.to_path_buf());
        if let Ok(cfg) = load_vocoder(vocoder) {
            files.push(cfg.model_path);
        }
    }
    files
}

/// Bumped whenever a change to this crate makes the same score, models and
/// seed render different samples (the app hashes it into every render
/// unit's key, so old takes read stale instead of being reused). 2: the
/// bank models' per-unit coverage, the one-frame floor in the alignment
/// and the pitch model's rest flags (2026-10-08).
pub const SVS_RENDER_REV: u32 = 2;

/// Lock a cached session, recovering it from a panicked holder (a run
/// that panicked leaves the session as usable as before it).
pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone)]
pub struct RunSummary {
    pub segments_rendered: usize,
    pub total_samples: usize,
    pub sample_rate: u32,
    pub per_segment_seconds: Vec<(usize, f64)>,
}

fn preprocess_acoustic(
    seg: &DsSegment,
    phoneme_map: &HashMap<String, i64>,
    frame_length: f64,
    cfg: &AcousticConfig,
    flags: &AcousticFlags,
    speaker_embeddings: &SpeakerEmbeddings,
    selected_speaker: Option<&str>,
) -> Result<PreprocessedAcoustic> {
    let tokens = phonemes_to_tokens(phoneme_map, &seg.ph_seq)?;
    let durations = phoneme_durations_to_frames(&seg.ph_dur, frame_length);
    let target_frames: i64 = durations.iter().sum();
    let n_frames = target_frames.max(0) as usize;
    if n_frames == 0 {
        return Err(anyhow!("segment yielded zero frames"));
    }

    let f0 = seg.f0.resample(frame_length, n_frames);
    if f0.is_empty() {
        return Err(anyhow!("segment has no f0 curve"));
    }

    let velocity = if seg.velocity.is_empty() {
        None
    } else {
        Some(
            seg.velocity
                .resample(frame_length, n_frames)
                .into_iter()
                .map(|v| v as f32)
                .collect(),
        )
    };
    let gender = if seg.gender.is_empty() {
        None
    } else {
        Some(
            seg.gender
                .resample(frame_length, n_frames)
                .into_iter()
                .map(|v| v as f32)
                .collect(),
        )
    };
    let energy = if seg.energy.is_empty() {
        None
    } else {
        Some(
            seg.energy
                .resample(frame_length, n_frames)
                .into_iter()
                .map(|v| v as f32)
                .collect(),
        )
    };
    let breathiness = if seg.breathiness.is_empty() {
        None
    } else {
        Some(
            seg.breathiness
                .resample(frame_length, n_frames)
                .into_iter()
                .map(|v| v as f32)
                .collect(),
        )
    };
    let tension = if seg.tension.is_empty() {
        None
    } else {
        Some(
            seg.tension
                .resample(frame_length, n_frames)
                .into_iter()
                .map(|v| v as f32)
                .collect(),
        )
    };

    let spk_embed = if flags.multi_speakers {
        let speaker = selected_speaker
            .ok_or_else(|| anyhow!("acoustic model is multi-speaker but no speaker selected"))?;
        let emb = speaker_embeddings
            .get(speaker)
            .ok_or_else(|| anyhow!("speaker `{speaker}` not found in voicebank"))?;
        let mut out = Vec::with_capacity(n_frames * emb.len());
        for _ in 0..n_frames {
            out.extend_from_slice(emb);
        }
        Some(out)
    } else {
        None
    };

    // Energy / breathiness are required when the acoustic model declares them but the .ds
    // doesn't supply them. In a real pipeline we'd run the variance model here; for the PoC
    // we return an error so the user knows to supply them or pick a simpler voicebank.
    if flags.energy && energy.is_none() {
        return Err(anyhow!(
            "acoustic model requires energy input but the .ds supplied none and the variance \
             predictor is not wired up in this PoC"
        ));
    }
    if flags.breathiness && breathiness.is_none() {
        return Err(anyhow!(
            "acoustic model requires breathiness input but the .ds supplied none and the \
             variance predictor is not wired up in this PoC"
        ));
    }

    let _ = cfg;

    let languages = if seg.languages.is_empty() {
        None
    } else {
        if seg.languages.len() != tokens.len() {
            return Err(anyhow!(
                "segment.languages length {} != tokens length {}",
                seg.languages.len(),
                tokens.len()
            ));
        }
        Some(seg.languages.clone())
    };

    Ok(PreprocessedAcoustic {
        tokens,
        durations,
        languages,
        f0,
        velocity,
        gender,
        tension,
        energy,
        breathiness,
        spk_embed,
    })
}

/// Map phoneme symbols to the voicebank's token ids.
///
/// A symbol the dictionary lacks is sung as its nearest substitute the
/// dictionary has ([`nearest_substitutes`], e.g. Lilia `v` → `f`) — the
/// same rule the app applies before building segments, repeated here so
/// a phoneme stream that skipped it still renders (code review FU-M6b).
/// A symbol with no substitute either is an error listing every such
/// symbol (each once, in order of first appearance). It used to fall
/// back to token 0 (`<PAD>` / `AP`), which rendered breath or silence
/// where the phoneme should be while the render reported success.
///
/// [`nearest_substitutes`]: crate::voicebank::phonetics::nearest_substitutes
pub fn phonemes_to_tokens(map: &HashMap<String, i64>, phonemes: &[String]) -> Result<Vec<i64>> {
    let mut missing: Vec<&str> = Vec::new();
    let tokens: Vec<i64> = phonemes
        .iter()
        .map(|ph| {
            if let Some(&token) = map.get(ph) {
                return token;
            }
            let substitute = crate::voicebank::phonetics::nearest_substitutes(ph)
                .iter()
                .find_map(|candidate| map.get(*candidate).copied());
            substitute.unwrap_or_else(|| {
                if !missing.contains(&ph.as_str()) {
                    missing.push(ph.as_str());
                }
                0
            })
        })
        .collect();
    if missing.is_empty() {
        Ok(tokens)
    } else {
        Err(anyhow!(
            "phoneme(s) not in the voicebank dictionary, with no substitute it has: {} — \
             re-spell the lyric or choose a voicebank that sings them",
            missing.join(", ")
        ))
    }
}

/// Convert phoneme durations (seconds) into frame counts, using the same accumulate-then-diff
/// approach as Jobsecond's reference. This makes the cumulative position match a naive
/// round-each-duration scheme, but distributes rounding error correctly across the sequence.
pub fn phoneme_durations_to_frames(seconds: &[f64], frame_length: f64) -> Vec<i64> {
    let mut accum = 0.0;
    let mut prev_frames: i64 = 0;
    let mut out = Vec::with_capacity(seconds.len());
    for &s in seconds {
        accum += s;
        let cum_frames = (accum / frame_length).round() as i64;
        out.push(cum_frames - prev_frames);
        prev_frames = cum_frames;
    }
    out
}

type SpeakerEmbeddings = HashMap<String, Vec<f32>>;

/// The speaker embeddings the acoustic config at `acoustic_config` lists,
/// by speaker key: what a render loads. Each `.emb` file must hold
/// `hidden_size` little-endian floats (256 on the shipped banks, 384 on
/// the LUNAI v170 singers); a missing file is skipped with a warning.
pub fn speaker_embeddings(acoustic_config: &Path) -> Result<HashMap<String, Vec<f32>>> {
    load_speaker_embeddings(&crate::config::load_acoustic(acoustic_config)?)
}

fn load_speaker_embeddings(cfg: &AcousticConfig) -> Result<SpeakerEmbeddings> {
    let mut map = SpeakerEmbeddings::new();
    if cfg.speakers.is_empty() {
        return Ok(map);
    }
    for speaker in &cfg.speakers {
        let path = cfg.speaker_dir.join(format!("{speaker}.emb"));
        if !path.exists() {
            tracing::warn!(
                "speaker embedding file missing: {} (speaker `{}`)",
                path.display(),
                speaker
            );
            continue;
        }
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        // An embedding is `hidden_size` floats: 256 on the shipped banks,
        // 384 on the LUNAI v170 singers.
        let expected = cfg.hidden_size.max(0) as usize * std::mem::size_of::<f32>();
        if bytes.len() != expected {
            return Err(anyhow!(
                "speaker embedding `{}` has unexpected size {} (expected {}: hidden_size {})",
                speaker,
                bytes.len(),
                expected,
                cfg.hidden_size
            ));
        }
        let floats: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        map.insert(speaker.clone(), floats);
    }
    Ok(map)
}

