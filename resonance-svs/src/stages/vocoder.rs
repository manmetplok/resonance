//! Vocoder ONNX: mel + f0 → float waveform at the vocoder's native sample rate.

use anyhow::{anyhow, Context, Result};
use ndarray::{Array2, Array3};
use ort::session::Session;
use ort::value::Value;
use std::path::Path;

use super::common::{build_session, input_names, output_names, ExecutionProvider};
use super::acoustic::MelOutput;

pub struct VocoderStage {
    pub session: Session,
    pub n_mel_bins: usize,
}

impl VocoderStage {
    pub fn load(model_path: &Path, ep: ExecutionProvider, device_index: i32, n_mel_bins: usize) -> Result<Self> {
        let session = build_session(model_path, ep, device_index)?;
        let ins = input_names(&session);
        let outs = output_names(&session);
        for required in &["mel", "f0"] {
            if !ins.contains(*required) {
                return Err(anyhow!(
                    "vocoder model at {} missing required input `{required}`",
                    model_path.display()
                ));
            }
        }
        if !outs.contains("waveform") {
            return Err(anyhow!(
                "vocoder model at {} missing required output `waveform`",
                model_path.display()
            ));
        }
        Ok(Self { session, n_mel_bins })
    }

    /// Run the vocoder, returning the rendered waveform.
    ///
    /// Takes `mel` by `&mut` so the spectrogram data can be moved out via
    /// `std::mem::take` and handed to `Array3::from_shape_vec` without
    /// cloning — the same pattern as `AcousticStage::infer`. A mel
    /// spectrogram is the largest per-segment tensor (n_frames ×
    /// n_mel_bins floats), so the previous `&MelOutput` signature cloned megabytes
    /// per segment. `mel.shape` is left intact; `mel.data` is empty after
    /// this call. `f0` is converted f64 → f32, which allocates the small
    /// per-frame vector regardless of ownership.
    pub fn infer(&mut self, mel: &mut MelOutput, f0: &[f64]) -> Result<Vec<f32>> {
        let n_frames = if mel.shape.len() >= 2 {
            mel.shape[mel.shape.len() - 2] as usize
        } else {
            return Err(anyhow!("mel tensor has unexpected rank {}", mel.shape.len()));
        };
        if mel.data.len() != n_frames * self.n_mel_bins {
            return Err(anyhow!(
                "mel data length {} != n_frames {} * n_mel_bins {}",
                mel.data.len(),
                n_frames,
                self.n_mel_bins
            ));
        }
        if f0.len() != n_frames {
            return Err(anyhow!(
                "f0 length {} != mel n_frames {}",
                f0.len(),
                n_frames
            ));
        }

        let mel_arr = Array3::<f32>::from_shape_vec(
            (1, n_frames, self.n_mel_bins),
            std::mem::take(&mut mel.data),
        )
        .context("packing mel for vocoder")?;
        let f0_arr = Array2::<f32>::from_shape_vec((1, n_frames), f0.iter().map(|x| *x as f32).collect())
            .context("packing f0 for vocoder")?;

        let inputs: Vec<(String, Value)> = vec![
            ("mel".into(), Value::from_array(mel_arr)?.into()),
            ("f0".into(), Value::from_array(f0_arr)?.into()),
        ];

        let outputs = self
            .session
            .run(inputs)
            .context("running vocoder ONNX session")?;
        let waveform_value = outputs
            .get("waveform")
            .ok_or_else(|| anyhow!("vocoder output `waveform` not present"))?;
        let (_shape, data) = waveform_value
            .try_extract_tensor::<f32>()
            .context("extracting waveform tensor")?;
        Ok(data.to_vec())
    }
}
