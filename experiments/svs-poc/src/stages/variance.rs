//! Variance predictor (split-pipeline). Predicts energy / breathiness / voicing / tension
//! curves when the .ds doesn't supply them. Not exercised by the PoC smoke test.

use anyhow::Result;
use ort::session::Session;
use std::path::Path;

use super::common::{build_session, ExecutionProvider};

pub struct VarianceStage {
    pub session: Session,
}

impl VarianceStage {
    pub fn load(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Self> {
        let session = build_session(model_path, ep, device_index)?;
        Ok(Self { session })
    }
}
