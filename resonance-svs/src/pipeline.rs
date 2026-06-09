//! Orchestrates the .ds → WAV pipeline. Stages run in order:
//!   linguistic → (dur) → (pitch) → (variance) → acoustic → vocoder
//! gated on dsconfig flags + CLI overrides. The smoke-test path only exercises
//! acoustic + vocoder (combined acoustic ONNX, .ds supplies ph_dur and f0_seq directly).

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::time::Instant;

use crate::audio::{mix_into_timeline, write_mono_f32_wav};
use crate::config::{load_acoustic, load_phoneme_dict, load_vocoder, AcousticConfig};
use crate::ds::{load_ds_file, DsSegment};
use crate::stages::acoustic::{AcousticStage, PreprocessedAcoustic, SPK_EMBED_SIZE};
use crate::stages::common::ExecutionProvider;
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
/// mixed mono waveform in memory.
pub fn render_segments(segments: &[DsSegment], args: &PipelineArgs) -> Result<RenderedAudio> {
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

    let mut acoustic = AcousticStage::load(
        &acoustic_cfg.acoustic_model,
        args.execution_provider,
        args.device_index,
    )?;
    tracing::info!("acoustic features: {:?}", acoustic.supported_features());

    // Vocoder runs on CPU regardless of execution provider for the acoustic model. Mirrors
    // Jobsecond's choice: most vocoders are CPU-bound matmul + 1D conv, GPU dispatch overhead
    // dominates. Users can change this trivially if they want.
    let mut vocoder = VocoderStage::load(
        &vocoder_cfg.model_path,
        ExecutionProvider::Cpu,
        args.device_index,
        vocoder_cfg.num_mel_bins as usize,
    )?;

    let speaker_embeddings = load_speaker_embeddings(&acoustic_cfg)?;
    let selected_speaker = args
        .speaker
        .clone()
        .or_else(|| acoustic_cfg.speakers.first().cloned());

    let mut rendered: Vec<(i64, Vec<f32>)> = Vec::with_capacity(segments.len());
    let mut perf = Vec::with_capacity(segments.len());

    for (i, seg) in segments.iter().enumerate() {
        tracing::info!("segment {}/{}", i + 1, segments.len());
        let t0 = Instant::now();
        let offset_samples = (seg.offset * vocoder_cfg.sample_rate as f64).ceil() as i64;

        let mut pd = preprocess_acoustic(
            seg,
            &phoneme_map,
            frame_length,
            &acoustic_cfg,
            &acoustic,
            &speaker_embeddings,
            selected_speaker.as_deref(),
        )?;
        let n_frames = pd.f0.len();
        let mut mel = acoustic.infer(&mut pd, speedup as i64, depth)?;
        let mut waveform = vocoder.infer(&mut mel, &pd.f0)?;
        // Vocoder waveform length is typically n_frames * hop_size, but some vocoders return
        // shorter/longer. Trim very long results to a sensible cap.
        let expected = n_frames * vocoder_cfg.hop_size as usize;
        if waveform.len() > expected + vocoder_cfg.hop_size as usize {
            waveform.truncate(expected);
        }
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
        sample_rate: vocoder_cfg.sample_rate as u32,
        per_segment_seconds: perf
            .into_iter()
            .map(|(i, d)| (i, d.as_secs_f64()))
            .collect(),
    })
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
    acoustic: &AcousticStage,
    speaker_embeddings: &SpeakerEmbeddings,
    selected_speaker: Option<&str>,
) -> Result<PreprocessedAcoustic> {
    let tokens = phonemes_to_tokens(phoneme_map, &seg.ph_seq);
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

    let spk_embed = if acoustic.flags.multi_speakers {
        let speaker = selected_speaker
            .ok_or_else(|| anyhow!("acoustic model is multi-speaker but no speaker selected"))?;
        let emb = speaker_embeddings
            .get(speaker)
            .ok_or_else(|| anyhow!("speaker `{speaker}` not found in voicebank"))?;
        let mut out = Vec::with_capacity(n_frames * SPK_EMBED_SIZE);
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
    if acoustic.flags.energy && energy.is_none() {
        return Err(anyhow!(
            "acoustic model requires energy input but the .ds supplied none and the variance \
             predictor is not wired up in this PoC"
        ));
    }
    if acoustic.flags.breathiness && breathiness.is_none() {
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

fn phonemes_to_tokens(map: &HashMap<String, i64>, phonemes: &[String]) -> Vec<i64> {
    // Unknown phonemes fall back to token 0 (typically `<PAD>` / `AP`),
    // which silently corrupts the rendered segment if the caller's g2p
    // emits a symbol the voicebank doesn't have. Log the first few
    // unknowns per call so the issue isn't completely silent — once
    // per phoneme is enough to identify a missing mapping without
    // flooding the logs.
    let mut warned: std::collections::HashSet<&str> = std::collections::HashSet::new();
    phonemes
        .iter()
        .map(|ph| {
            if let Some(&tok) = map.get(ph) {
                tok
            } else {
                if warned.insert(ph.as_str()) {
                    eprintln!(
                        "resonance-svs: phoneme {:?} not in voicebank dictionary — substituting token 0",
                        ph
                    );
                }
                0
            }
        })
        .collect()
}

/// Convert phoneme durations (seconds) into frame counts, using the same accumulate-then-diff
/// approach as Jobsecond's reference. This makes the cumulative position match a naive
/// round-each-duration scheme, but distributes rounding error correctly across the sequence.
fn phoneme_durations_to_frames(seconds: &[f64], frame_length: f64) -> Vec<i64> {
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
        if bytes.len() != SPK_EMBED_SIZE * std::mem::size_of::<f32>() {
            return Err(anyhow!(
                "speaker embedding `{}` has unexpected size {} (expected {})",
                speaker,
                bytes.len(),
                SPK_EMBED_SIZE * std::mem::size_of::<f32>()
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

