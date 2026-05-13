//! Pitch predictor (split-pipeline). Not exercised by the PoC smoke test. The split path
//! actually has three sub-graphs (preprocess / diffusion / postprocess) in openvpi's newer
//! exporters; this wrapper assumes a merged pitch ONNX as some exporters produce.

use anyhow::Result;
use ort::session::Session;
use std::path::Path;

use super::common::{build_session, input_names, output_names, ExecutionProvider};

pub struct PitchStage {
    pub session: Session,
}

impl PitchStage {
    pub fn load(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Self> {
        let session = build_session(model_path, ep, device_index)?;
        // Pitch ONNX format varies between exporter versions. Don't enforce a specific I/O
        // contract here; the caller is expected to introspect supported inputs.
        let _ = (input_names(&session), output_names(&session));
        Ok(Self { session })
    }
}
