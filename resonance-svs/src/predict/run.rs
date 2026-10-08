//! The bank's own duration, pitch and variance models: load each one's
//! linguistic encoder and predictor once, then run them per render unit.
//! Input shapes follow OpenUtau's DiffSinger renderer (see [`super::score`]).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use ndarray::{Array0, Array2, Array3};
use ort::session::Session;
use ort::value::Value;

use super::config::{ModelConfig, ModelKind};
use super::score::{pitch_notes, BankScore};
use crate::stages::common::{build_session, input_names, ExecutionProvider, SeededSession};

/// Noise values per frame the pitch / variance models draw (64 repeat
/// bins on the pitch models; up to four variances × their bins).
const PITCH_NOISE_PER_FRAME: usize = 256;
const VARIANCE_NOISE_PER_FRAME: usize = 1024;

/// A model's linguistic encoder plus its speaker embeddings and phoneme
/// ids — what all three predictors share.
struct Encoder {
    cfg: ModelConfig,
    linguistic: Session,
    linguistic_inputs: std::collections::HashSet<String>,
    embeds: HashMap<String, Vec<f32>>,
}

/// What the linguistic encoder returns.
struct Encoded {
    /// `[1, n_tokens, hidden]`.
    encoder_out: Array3<f32>,
    /// `[1, n_tokens]`.
    x_masks: Array2<bool>,
}

impl Encoder {
    fn load(config: &Path, kind: ModelKind, ep: ExecutionProvider) -> Result<Self> {
        let cfg = ModelConfig::load(config, kind)?;
        let linguistic = build_session(&cfg.linguistic, ep, 0)?;
        let linguistic_inputs = input_names(&linguistic);
        let mut embeds = HashMap::new();
        for spk in &cfg.speakers {
            let path = cfg.dir.join(format!("{spk}.emb"));
            let bytes = std::fs::read(&path)
                .with_context(|| format!("reading speaker embedding {}", path.display()))?;
            let floats: Vec<f32> = bytes
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            if floats.len() != cfg.hidden_size {
                bail!(
                    "speaker embedding {} holds {} values, not {}",
                    path.display(),
                    floats.len(),
                    cfg.hidden_size
                );
            }
            embeds.insert(spk.clone(), floats);
        }
        Ok(Self {
            cfg,
            linguistic,
            linguistic_inputs,
            embeds,
        })
    }

    /// The score's phonemes as this model's token ids.
    fn tokens(&self, score: &BankScore) -> Result<Vec<i64>> {
        score
            .phones
            .iter()
            .map(|p| {
                self.cfg.phonemes.get(&p.symbol).copied().ok_or_else(|| {
                    anyhow!(
                        "the bank's {} model has no phoneme {:?}",
                        self.cfg.kind.as_str(),
                        p.symbol
                    )
                })
            })
            .collect()
    }

    /// Run the encoder: word mode (`word_div` / `word_dur`) when it takes
    /// them, else phoneme mode (`ph_dur` frames).
    fn encode(&mut self, score: &BankScore, ph_dur: Option<&[i64]>) -> Result<Encoded> {
        let tokens = self.tokens(score)?;
        let n = tokens.len();
        let mut inputs: Vec<(String, Value)> = vec![("tokens".into(), row_i64(tokens)?)];
        if self.linguistic_inputs.contains("languages") {
            let langs = score.phones.iter().map(|p| p.language).collect();
            inputs.push(("languages".into(), row_i64(langs)?));
        }
        if self.linguistic_inputs.contains("word_div") {
            inputs.push(("word_div".into(), row_i64(score.word_div())?));
            inputs.push((
                "word_dur".into(),
                row_i64(score.word_dur(self.cfg.frame_sec()))?,
            ));
        }
        if self.linguistic_inputs.contains("ph_dur") {
            let d = ph_dur.ok_or_else(|| anyhow!("phoneme-mode encoder needs durations"))?;
            inputs.push(("ph_dur".into(), row_i64(d.to_vec())?));
        }
        let outputs = self.linguistic.run(inputs).context("linguistic encoder")?;
        let (shape, enc) = outputs
            .get("encoder_out")
            .ok_or_else(|| anyhow!("linguistic encoder: no encoder_out"))?
            .try_extract_tensor::<f32>()?;
        let hidden = *shape.last().ok_or_else(|| anyhow!("encoder_out rank"))? as usize;
        let encoder_out = Array3::from_shape_vec((1, n, hidden), enc.to_vec())?;
        let (_, masks) = outputs
            .get("x_masks")
            .ok_or_else(|| anyhow!("linguistic encoder: no x_masks"))?
            .try_extract_tensor::<bool>()?;
        let x_masks = Array2::from_shape_vec((1, n), masks.to_vec())?;
        Ok(Encoded {
            encoder_out,
            x_masks,
        })
    }

    /// The model's embedding for acoustic speaker `speaker`, repeated
    /// `rows` times: `[1, rows, hidden]`.
    fn speaker_rows(&self, speaker: Option<&str>, rows: usize) -> Result<Value> {
        let key = self
            .cfg
            .speaker_for(speaker)
            .ok_or_else(|| anyhow!("the {} model takes a speaker but lists none", self.cfg.kind.as_str()))?;
        let emb = &self.embeds[key];
        let mut data = Vec::with_capacity(rows * emb.len());
        for _ in 0..rows {
            data.extend_from_slice(emb);
        }
        Ok(Value::from_array(Array3::from_shape_vec((1, rows, emb.len()), data)?)?.into())
    }
}

fn row_i64(v: Vec<i64>) -> Result<Value> {
    let n = v.len();
    Ok(Value::from_array(Array2::from_shape_vec((1, n), v)?)?.into())
}

fn row_f32(v: Vec<f32>) -> Result<Value> {
    let n = v.len();
    Ok(Value::from_array(Array2::from_shape_vec((1, n), v)?)?.into())
}

fn row_bool(v: Vec<bool>) -> Result<Value> {
    let n = v.len();
    Ok(Value::from_array(Array2::from_shape_vec((1, n), v)?)?.into())
}

fn scalar_i64(v: i64) -> Result<Value> {
    Ok(Value::from_array(Array0::from_elem((), v))?.into())
}

/// The bank's duration model (`dsdur`).
pub struct DurationModel {
    enc: Encoder,
    dur: Session,
    dur_inputs: std::collections::HashSet<String>,
}

impl DurationModel {
    pub fn load(config: &Path, ep: ExecutionProvider) -> Result<Self> {
        let enc = Encoder::load(config, ModelKind::Duration, ep)?;
        let dur = build_session(&enc.cfg.model, ep, 0)?;
        let dur_inputs = input_names(&dur);
        Ok(Self {
            enc,
            dur,
            dur_inputs,
        })
    }

    pub fn frame_sec(&self) -> f64 {
        self.enc.cfg.frame_sec()
    }

    /// Predicted frames per phoneme of `score`, sung by acoustic speaker
    /// `speaker`.
    pub fn predict(&mut self, score: &BankScore, speaker: Option<&str>) -> Result<Vec<f32>> {
        score.validate().map_err(|e| anyhow!("duration model: {e}"))?;
        let n = score.phones.len();
        let encoded = self.enc.encode(score, None)?;
        let mut inputs: Vec<(String, Value)> = vec![
            ("encoder_out".into(), Value::from_array(encoded.encoder_out)?.into()),
            ("x_masks".into(), Value::from_array(encoded.x_masks)?.into()),
            ("ph_midi".into(), row_i64(score.ph_midi())?),
        ];
        if self.dur_inputs.contains("spk_embed") {
            inputs.push(("spk_embed".into(), self.enc.speaker_rows(speaker, n)?));
        }
        let outputs = self.dur.run(inputs).context("duration model")?;
        let (_, pred) = outputs
            .get("ph_dur_pred")
            .ok_or_else(|| anyhow!("duration model: no ph_dur_pred"))?
            .try_extract_tensor::<f32>()?;
        if pred.len() != n {
            bail!("duration model returned {} durations for {n} phonemes", pred.len());
        }
        Ok(pred.to_vec())
    }
}

/// The bank's pitch model (`dspitch`).
pub struct PitchModel {
    enc: Encoder,
    pitch: SeededSession,
    pitch_inputs: std::collections::HashSet<String>,
}

impl PitchModel {
    pub fn load(config: &Path, ep: ExecutionProvider) -> Result<Self> {
        let enc = Encoder::load(config, ModelKind::Pitch, ep)?;
        let pitch = SeededSession::load(&enc.cfg.model, ep, 0)?;
        let pitch_inputs = input_names(&pitch.session);
        Ok(Self {
            enc,
            pitch,
            pitch_inputs,
        })
    }

    pub fn frame_sec(&self) -> f64 {
        self.enc.cfg.frame_sec()
    }

    /// The predicted pitch (fractional MIDI) per frame of a unit whose
    /// phonemes last `ph_dur` frames. `expr` is the model's
    /// expressiveness (1 = as trained), `steps` its sampling steps, `seed`
    /// its noise.
    pub fn predict(
        &mut self,
        score: &BankScore,
        ph_dur: &[i64],
        speaker: Option<&str>,
        expr: f32,
        steps: i64,
        seed: u64,
    ) -> Result<Vec<f32>> {
        let n_frames: i64 = ph_dur.iter().sum();
        let frames = n_frames as usize;
        let encoded = self.enc.encode(score, Some(ph_dur))?;
        let notes = pitch_notes(score, self.frame_sec(), n_frames);
        let mut inputs: Vec<(String, Value)> = vec![
            ("encoder_out".into(), Value::from_array(encoded.encoder_out)?.into()),
            ("ph_dur".into(), row_i64(ph_dur.to_vec())?),
            ("note_midi".into(), row_f32(notes.iter().map(|n| n.0).collect())?),
            ("note_dur".into(), row_i64(notes.iter().map(|n| n.2).collect())?),
            ("pitch".into(), row_f32(vec![60.0; frames])?),
            ("retake".into(), row_bool(vec![true; frames])?),
            ("steps".into(), scalar_i64(steps)?),
        ];
        if self.pitch_inputs.contains("note_rest") {
            inputs.push(("note_rest".into(), row_bool(notes.iter().map(|n| n.1).collect())?));
        }
        if self.pitch_inputs.contains("expr") {
            inputs.push(("expr".into(), row_f32(vec![expr.clamp(0.0, 1.0); frames])?));
        }
        if self.pitch_inputs.contains("spk_embed") {
            inputs.push(("spk_embed".into(), self.enc.speaker_rows(speaker, frames)?));
        }
        inputs.extend(SeededSession::noise_values(
            &self.pitch.noise,
            seed,
            frames,
            PITCH_NOISE_PER_FRAME,
        )?);
        let outputs = self.pitch.session.run(inputs).context("pitch model")?;
        let (_, pred) = outputs
            .get("pitch_pred")
            .ok_or_else(|| anyhow!("pitch model: no pitch_pred"))?
            .try_extract_tensor::<f32>()?;
        if pred.len() != frames {
            bail!("pitch model returned {} frames for {frames}", pred.len());
        }
        Ok(pred.to_vec())
    }
}

/// What the variance model predicted, per frame; `None` for a curve it
/// does not predict.
#[derive(Debug, Clone, Default)]
pub struct VarianceCurves {
    pub energy: Option<Vec<f32>>,
    pub breathiness: Option<Vec<f32>>,
    pub voicing: Option<Vec<f32>>,
    pub tension: Option<Vec<f32>>,
}

/// The bank's variance model (`dsvariance`).
pub struct VarianceModel {
    enc: Encoder,
    variance: SeededSession,
    variance_inputs: std::collections::HashSet<String>,
}

const VARIANCES: [&str; 4] = ["energy", "breathiness", "voicing", "tension"];

impl VarianceModel {
    pub fn load(config: &Path, ep: ExecutionProvider) -> Result<Self> {
        let enc = Encoder::load(config, ModelKind::Variance, ep)?;
        let variance = SeededSession::load(&enc.cfg.model, ep, 0)?;
        let variance_inputs = input_names(&variance.session);
        Ok(Self {
            enc,
            variance,
            variance_inputs,
        })
    }

    pub fn frame_sec(&self) -> f64 {
        self.enc.cfg.frame_sec()
    }

    /// Predict the variance curves of a unit whose phonemes last `ph_dur`
    /// frames, sung at `pitch` (fractional MIDI per frame).
    pub fn predict(
        &mut self,
        score: &BankScore,
        ph_dur: &[i64],
        pitch: &[f32],
        speaker: Option<&str>,
        steps: i64,
        seed: u64,
    ) -> Result<VarianceCurves> {
        let frames = ph_dur.iter().sum::<i64>() as usize;
        if pitch.len() != frames {
            bail!("variance model: {} pitch frames for {frames}", pitch.len());
        }
        let encoded = self.enc.encode(score, Some(ph_dur))?;
        let mut inputs: Vec<(String, Value)> = vec![
            ("encoder_out".into(), Value::from_array(encoded.encoder_out)?.into()),
            ("ph_dur".into(), row_i64(ph_dur.to_vec())?),
            ("pitch".into(), row_f32(pitch.to_vec())?),
            ("steps".into(), scalar_i64(steps)?),
        ];
        let predicted: Vec<&str> = VARIANCES
            .iter()
            .copied()
            .filter(|v| self.variance_inputs.contains(*v))
            .collect();
        for v in &predicted {
            inputs.push(((*v).into(), row_f32(vec![0.0; frames])?));
        }
        let retake = vec![true; frames * predicted.len().max(1)];
        inputs.push((
            "retake".into(),
            Value::from_array(Array3::from_shape_vec(
                (1, frames, predicted.len().max(1)),
                retake,
            )?)?
            .into(),
        ));
        if self.variance_inputs.contains("spk_embed") {
            inputs.push(("spk_embed".into(), self.enc.speaker_rows(speaker, frames)?));
        }
        inputs.extend(SeededSession::noise_values(
            &self.variance.noise,
            seed,
            frames,
            VARIANCE_NOISE_PER_FRAME,
        )?);
        let outputs = self.variance.session.run(inputs).context("variance model")?;
        let get = |name: &str| -> Result<Option<Vec<f32>>> {
            match outputs.get(format!("{name}_pred").as_str()) {
                Some(v) => {
                    let (_, data) = v.try_extract_tensor::<f32>()?;
                    Ok(Some(data.to_vec()))
                }
                None => Ok(None),
            }
        };
        Ok(VarianceCurves {
            energy: get("energy")?,
            breathiness: get("breathiness")?,
            voicing: get("voicing")?,
            tension: get("tension")?,
        })
    }
}
