//! Linguistic encoder stage (split-pipeline). Loads a `linguistic.onnx` exported by recent
//! openvpi variance exporters. Inputs: `tokens`, `word_div`, `word_dur`. Outputs:
//! `encoder_out`, `x_masks`. Not exercised by the PoC smoke test — included so the
//! architecture matches the split-pipeline layout the spec describes.

use anyhow::{anyhow, Context, Result};
use ndarray::Array2;
use ort::session::Session;
use ort::value::Value;
use std::path::Path;

use super::common::{build_session, input_names, output_names, ExecutionProvider};

pub struct LinguisticStage {
    pub session: Session,
}

pub struct LinguisticOutput {
    /// `[n_tokens, hidden_size]` row-major.
    pub encoder_out: Vec<f32>,
    pub hidden_size: usize,
    pub x_masks: Vec<bool>,
}

impl LinguisticStage {
    pub fn load(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Self> {
        let session = build_session(model_path, ep, device_index)?;
        let ins = input_names(&session);
        let outs = output_names(&session);
        for required in &["tokens"] {
            if !ins.contains(*required) {
                return Err(anyhow!("linguistic model missing input `{required}`"));
            }
        }
        for required in &["encoder_out", "x_masks"] {
            if !outs.contains(*required) {
                return Err(anyhow!("linguistic model missing output `{required}`"));
            }
        }
        Ok(Self { session })
    }

    pub fn infer(
        &mut self,
        tokens: &[i64],
        word_div: Option<&[i64]>,
        word_dur: Option<&[i64]>,
        ph_dur: Option<&[i64]>,
    ) -> Result<LinguisticOutput> {
        let n = tokens.len();
        let tokens_arr =
            Array2::<i64>::from_shape_vec((1, n), tokens.to_vec()).context("packing tokens")?;
        let mut inputs: Vec<(String, Value)> = vec![
            ("tokens".into(), Value::from_array(tokens_arr)?.into()),
        ];
        let ins = input_names(&self.session);
        if ins.contains("word_div") {
            let wd = word_div.ok_or_else(|| anyhow!("linguistic needs word_div"))?;
            let arr =
                Array2::<i64>::from_shape_vec((1, wd.len()), wd.to_vec()).context("word_div")?;
            inputs.push(("word_div".into(), Value::from_array(arr)?.into()));
        }
        if ins.contains("word_dur") {
            let wd = word_dur.ok_or_else(|| anyhow!("linguistic needs word_dur"))?;
            let arr =
                Array2::<i64>::from_shape_vec((1, wd.len()), wd.to_vec()).context("word_dur")?;
            inputs.push(("word_dur".into(), Value::from_array(arr)?.into()));
        }
        if ins.contains("ph_dur") {
            let pd = ph_dur.ok_or_else(|| anyhow!("linguistic needs ph_dur"))?;
            let arr = Array2::<i64>::from_shape_vec((1, pd.len()), pd.to_vec()).context("ph_dur")?;
            inputs.push(("ph_dur".into(), Value::from_array(arr)?.into()));
        }

        let outputs = self.session.run(inputs).context("linguistic.run")?;
        let enc = outputs
            .get("encoder_out")
            .ok_or_else(|| anyhow!("missing encoder_out"))?;
        let (enc_shape, enc_data) = enc.try_extract_tensor::<f32>()?;
        let hidden_size = *enc_shape.last().ok_or_else(|| anyhow!("encoder_out rank"))? as usize;

        let masks = outputs
            .get("x_masks")
            .ok_or_else(|| anyhow!("missing x_masks"))?;
        let (_ms, masks_data) = masks.try_extract_tensor::<bool>()?;

        Ok(LinguisticOutput {
            encoder_out: enc_data.to_vec(),
            hidden_size,
            x_masks: masks_data.to_vec(),
        })
    }
}
