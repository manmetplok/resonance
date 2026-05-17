//! Combined acoustic ONNX (encoder + diffusion + post-processing → mel). Mirrors the I/O
//! contract Jobsecond's reference C++ implementation uses: required inputs are `tokens`,
//! `durations`, `f0`, `speedup`; optional inputs gated on whether the model declares them
//! are `velocity`, `gender`, `spk_embed`, `energy`, `breathiness`, `depth`. Output: `mel`.

use anyhow::{anyhow, Context, Result};
use ndarray::{Array0, Array2, Array3};
use ort::session::Session;
use ort::value::Value;
use std::path::Path;

use super::common::{build_session, input_names, output_names, ExecutionProvider};

#[derive(Debug, Clone, Copy, Default)]
pub struct AcousticFlags {
    pub speedup: bool,
    pub steps: bool,
    pub velocity: bool,
    pub gender: bool,
    pub multi_speakers: bool,
    pub energy: bool,
    pub breathiness: bool,
    pub shallow_diffusion: bool,
    pub voicing: bool,
    pub tension: bool,
    pub key_shift: bool,
    pub speed: bool,
    /// Multi-language voicebanks accept a per-token `languages` input
    /// alongside `tokens` / `durations`. When false, callers should
    /// leave `PreprocessedAcoustic::languages` as `None`.
    pub languages: bool,
}

pub struct AcousticStage {
    pub session: Session,
    pub flags: AcousticFlags,
}

impl AcousticStage {
    pub fn load(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Self> {
        let session = build_session(model_path, ep, device_index)?;
        let ins = input_names(&session);
        let outs = output_names(&session);

        for required in &["tokens", "durations", "f0"] {
            if !ins.contains(*required) {
                return Err(anyhow!(
                    "acoustic model at {} missing required input `{required}`",
                    model_path.display()
                ));
            }
        }
        if !outs.contains("mel") {
            return Err(anyhow!(
                "acoustic model at {} missing required output `mel`",
                model_path.display()
            ));
        }

        let flags = AcousticFlags {
            speedup: ins.contains("speedup"),
            steps: ins.contains("steps"),
            velocity: ins.contains("velocity"),
            gender: ins.contains("gender"),
            multi_speakers: ins.contains("spk_embed"),
            energy: ins.contains("energy"),
            breathiness: ins.contains("breathiness"),
            shallow_diffusion: ins.contains("depth"),
            voicing: ins.contains("voicing"),
            tension: ins.contains("tension"),
            key_shift: ins.contains("key_shift"),
            speed: ins.contains("speed"),
            languages: ins.contains("languages"),
        };
        Ok(Self { session, flags })
    }

    /// Run the acoustic model, returning the mel spectrogram as `[n_frames, n_mel_bins]` in
    /// row-major order (n_frames-first). `depth` may be either an integer step count (older
    /// voicebanks) or a fractional 0–1 value (newer `use_variable_depth` voicebanks); the
    /// model itself decides which it expects via the dtype of its `depth` input.
    pub fn infer(&mut self, pd: &PreprocessedAcoustic, speedup: i64, depth: f32) -> Result<MelOutput> {
        let n_tokens = pd.tokens.len();
        if pd.durations.len() != n_tokens {
            return Err(anyhow!(
                "tokens / durations length mismatch: {} vs {}",
                n_tokens,
                pd.durations.len()
            ));
        }
        let n_frames = pd.f0.len();

        let mut inputs: Vec<(String, Value)> = Vec::with_capacity(10);

        let tokens = Array2::<i64>::from_shape_vec((1, n_tokens), pd.tokens.clone())
            .context("packing tokens tensor")?;
        let durations = Array2::<i64>::from_shape_vec((1, n_tokens), pd.durations.clone())
            .context("packing durations tensor")?;
        let f0 = Array2::<f32>::from_shape_vec((1, n_frames), pd.f0.iter().map(|x| *x as f32).collect())
            .context("packing f0 tensor")?;
        let speedup_scalar = Array0::<i64>::from_elem((), speedup);

        inputs.push(("tokens".into(), Value::from_array(tokens)?.into()));
        inputs.push(("durations".into(), Value::from_array(durations)?.into()));
        if self.flags.languages {
            let langs = pd.languages.as_ref().ok_or_else(|| {
                anyhow!("acoustic model expects `languages` input but none supplied in .ds")
            })?;
            if langs.len() != n_tokens {
                return Err(anyhow!(
                    "languages length {} != n_tokens {}",
                    langs.len(),
                    n_tokens
                ));
            }
            let arr = Array2::<i64>::from_shape_vec((1, n_tokens), langs.clone())
                .context("packing languages tensor")?;
            inputs.push(("languages".into(), Value::from_array(arr)?.into()));
        }
        inputs.push(("f0".into(), Value::from_array(f0)?.into()));
        if self.flags.speedup {
            inputs.push(("speedup".into(), Value::from_array(speedup_scalar)?.into()));
        }
        if self.flags.steps {
            // Newer "continuous-acceleration" exports take `steps` directly (number of
            // diffusion sampling steps). We reuse the CLI --speedup value as the step count.
            let steps_scalar = Array0::<i64>::from_elem((), speedup.max(1));
            inputs.push(("steps".into(), Value::from_array(steps_scalar)?.into()));
        }
        if self.flags.key_shift {
            let arr = Array2::<f32>::from_shape_vec((1, n_frames), vec![0.0; n_frames])
                .context("packing key_shift")?;
            inputs.push(("key_shift".into(), Value::from_array(arr)?.into()));
        }
        if self.flags.speed {
            let arr = Array2::<f32>::from_shape_vec((1, n_frames), vec![1.0; n_frames])
                .context("packing speed")?;
            inputs.push(("speed".into(), Value::from_array(arr)?.into()));
        }
        if self.flags.voicing {
            let arr = Array2::<f32>::from_shape_vec((1, n_frames), vec![0.0; n_frames])
                .context("packing voicing")?;
            inputs.push(("voicing".into(), Value::from_array(arr)?.into()));
        }
        if self.flags.tension {
            let v = pd.tension_or_default(n_frames);
            let arr = Array2::<f32>::from_shape_vec((1, n_frames), v).context("packing tension")?;
            inputs.push(("tension".into(), Value::from_array(arr)?.into()));
        }

        if self.flags.velocity {
            let v = pd.velocity_or_default(n_frames);
            let arr = Array2::<f32>::from_shape_vec((1, n_frames), v).context("packing velocity")?;
            inputs.push(("velocity".into(), Value::from_array(arr)?.into()));
        }
        if self.flags.gender {
            let v = pd.gender_or_default(n_frames);
            let arr = Array2::<f32>::from_shape_vec((1, n_frames), v).context("packing gender")?;
            inputs.push(("gender".into(), Value::from_array(arr)?.into()));
        }
        if self.flags.multi_speakers {
            let emb = pd
                .spk_embed
                .as_ref()
                .ok_or_else(|| anyhow!("acoustic model expects spk_embed but none was prepared"))?;
            let frames_check = emb.len() / SPK_EMBED_SIZE;
            if frames_check != n_frames {
                return Err(anyhow!(
                    "spk_embed frame count {} != f0 frame count {}",
                    frames_check,
                    n_frames
                ));
            }
            let arr = Array3::<f32>::from_shape_vec((1, n_frames, SPK_EMBED_SIZE), emb.clone())
                .context("packing spk_embed")?;
            inputs.push(("spk_embed".into(), Value::from_array(arr)?.into()));
        }
        if self.flags.energy {
            let v = pd
                .energy
                .as_ref()
                .ok_or_else(|| anyhow!("acoustic model expects energy but none supplied in .ds"))?;
            let arr = Array2::<f32>::from_shape_vec((1, n_frames), v.clone()).context("packing energy")?;
            inputs.push(("energy".into(), Value::from_array(arr)?.into()));
        }
        if self.flags.breathiness {
            let v = pd
                .breathiness
                .as_ref()
                .ok_or_else(|| anyhow!("acoustic model expects breathiness but none supplied in .ds"))?;
            let arr = Array2::<f32>::from_shape_vec((1, n_frames), v.clone())
                .context("packing breathiness")?;
            inputs.push(("breathiness".into(), Value::from_array(arr)?.into()));
        }
        if self.flags.shallow_diffusion {
            if depth < 0.0 {
                return Err(anyhow!("acoustic model supports shallow diffusion but depth is < 0"));
            }
            // Inspect the model's actual dtype for `depth`. Newer "variable-depth" exports use
            // float32; older exports use int64 step counts.
            let depth_is_float = self
                .session
                .inputs()
                .iter()
                .find(|i| i.name() == "depth")
                .map(|i| matches!(
                    i.dtype(),
                    ort::value::ValueType::Tensor { ty: ort::value::TensorElementType::Float32, .. }
                ))
                .unwrap_or(false);
            if depth_is_float {
                let depth_scalar = Array0::<f32>::from_elem((), depth);
                inputs.push(("depth".into(), Value::from_array(depth_scalar)?.into()));
            } else {
                let depth_scalar = Array0::<i64>::from_elem((), depth as i64);
                inputs.push(("depth".into(), Value::from_array(depth_scalar)?.into()));
            }
        }

        let outputs = self
            .session
            .run(inputs)
            .context("running acoustic ONNX session")?;
        let mel_value = outputs
            .get("mel")
            .ok_or_else(|| anyhow!("acoustic output `mel` not present"))?;
        let (shape, data) = mel_value
            .try_extract_tensor::<f32>()
            .context("extracting mel tensor")?;

        Ok(MelOutput {
            shape: shape.iter().copied().collect(),
            data: data.to_vec(),
        })
    }

    pub fn supported_features(&self) -> &AcousticFlags {
        &self.flags
    }
}

pub const SPK_EMBED_SIZE: usize = 256;

#[derive(Debug, Clone, Default)]
pub struct PreprocessedAcoustic {
    pub tokens: Vec<i64>,
    pub durations: Vec<i64>,
    /// Per-token language ids (parallel to `tokens`). `Some` only for
    /// multi-language voicebanks whose acoustic ONNX accepts a
    /// `languages` input.
    pub languages: Option<Vec<i64>>,
    pub f0: Vec<f64>,
    pub velocity: Option<Vec<f32>>,
    pub gender: Option<Vec<f32>>,
    pub tension: Option<Vec<f32>>,
    pub energy: Option<Vec<f32>>,
    pub breathiness: Option<Vec<f32>>,
    /// Length `n_frames * SPK_EMBED_SIZE`, row-major over frames.
    pub spk_embed: Option<Vec<f32>>,
}

impl PreprocessedAcoustic {
    pub fn velocity_or_default(&self, n_frames: usize) -> Vec<f32> {
        self.velocity.clone().unwrap_or_else(|| vec![1.0; n_frames])
    }

    pub fn gender_or_default(&self, n_frames: usize) -> Vec<f32> {
        self.gender.clone().unwrap_or_else(|| vec![0.0; n_frames])
    }

    pub fn tension_or_default(&self, n_frames: usize) -> Vec<f32> {
        self.tension.clone().unwrap_or_else(|| vec![0.0; n_frames])
    }
}

#[derive(Debug, Clone)]
pub struct MelOutput {
    /// `[1, n_frames, n_mel_bins]`.
    pub shape: Vec<i64>,
    /// Row-major `f32` mel-spectrogram.
    pub data: Vec<f32>,
}

