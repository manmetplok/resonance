//! Duration predictor (split-pipeline). Not exercised by the PoC smoke test.

use anyhow::{anyhow, Context, Result};
use ndarray::{Array2, Array3};
use ort::session::Session;
use ort::value::Value;
use std::path::Path;

use super::common::{build_session, input_names, output_names, ExecutionProvider};
use super::linguistic::LinguisticOutput;

pub struct DurationStage {
    pub session: Session,
}

impl DurationStage {
    pub fn load(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Self> {
        let session = build_session(model_path, ep, device_index)?;
        let ins = input_names(&session);
        let outs = output_names(&session);
        if !ins.contains("encoder_out") || !ins.contains("x_masks") || !ins.contains("ph_midi") {
            return Err(anyhow!("duration model missing inputs (encoder_out, x_masks, ph_midi)"));
        }
        if !outs.contains("ph_dur_pred") {
            return Err(anyhow!("duration model missing output ph_dur_pred"));
        }
        Ok(Self { session })
    }

    pub fn infer(&mut self, ling: &LinguisticOutput, ph_midi: &[i64]) -> Result<Vec<f32>> {
        let n_tokens = ling.encoder_out.len() / ling.hidden_size;
        let enc = Array3::<f32>::from_shape_vec(
            (1, n_tokens, ling.hidden_size),
            ling.encoder_out.clone(),
        )
        .context("encoder_out")?;
        let masks = Array2::<bool>::from_shape_vec((1, n_tokens), ling.x_masks.clone())
            .context("x_masks")?;
        let midi = Array2::<i64>::from_shape_vec((1, ph_midi.len()), ph_midi.to_vec())
            .context("ph_midi")?;

        let inputs: Vec<(String, Value)> = vec![
            ("encoder_out".into(), Value::from_array(enc)?.into()),
            ("x_masks".into(), Value::from_array(masks)?.into()),
            ("ph_midi".into(), Value::from_array(midi)?.into()),
        ];
        let outputs = self.session.run(inputs).context("duration.run")?;
        let pred = outputs
            .get("ph_dur_pred")
            .ok_or_else(|| anyhow!("missing ph_dur_pred"))?;
        let (_s, data) = pred.try_extract_tensor::<f32>()?;
        Ok(data.to_vec())
    }
}
