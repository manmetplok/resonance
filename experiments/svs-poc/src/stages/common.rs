use anyhow::Result;
use ort::execution_providers::{
    CPUExecutionProvider, CUDAExecutionProvider, ExecutionProviderDispatch,
};
use ort::session::{builder::GraphOptimizationLevel, Session};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub enum ExecutionProvider {
    Cpu,
    Cuda,
    Rocm,
}

impl ExecutionProvider {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "cuda" => Self::Cuda,
            "rocm" | "hip" => Self::Rocm,
            _ => Self::Cpu,
        }
    }
}

/// Build a session with the requested provider, falling back to CPU if the provider isn't
/// registered (e.g. ROCm requested on a machine that only has CPU ORT binaries). Logs a
/// warning when falling back. The PoC tolerates this so users can develop on CPU machines.
pub fn build_session(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Session> {
    let providers: Vec<ExecutionProviderDispatch> = match ep {
        ExecutionProvider::Cpu => vec![CPUExecutionProvider::default().build()],
        ExecutionProvider::Cuda => vec![
            CUDAExecutionProvider::default()
                .with_device_id(device_index)
                .build(),
            CPUExecutionProvider::default().build(),
        ],
        ExecutionProvider::Rocm => {
            // ort 2.0.0-rc.10 gates the ROCm provider behind the `rocm` Cargo feature. Compile
            // with `--features rocm` to enable it. Without the feature flag we still attempt to
            // register, but it will silently fall back to CPU.
            #[cfg(feature = "rocm")]
            {
                vec![
                    ort::execution_providers::ROCmExecutionProvider::default()
                        .with_device_id(device_index)
                        .build(),
                    CPUExecutionProvider::default().build(),
                ]
            }
            #[cfg(not(feature = "rocm"))]
            {
                tracing::warn!(
                    "ROCm execution provider requested but svs-poc was built without `rocm` \
                     feature; falling back to CPU. Rebuild with `--features rocm` to enable."
                );
                vec![CPUExecutionProvider::default().build()]
            }
        }
    };

    let session = Session::builder()
        .map_err(|e| anyhow::anyhow!("creating ORT SessionBuilder: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow::anyhow!("setting graph optimization level: {e}"))?
        .with_execution_providers(providers)
        .map_err(|e| anyhow::anyhow!("registering execution providers: {e}"))?
        .commit_from_file(model_path)
        .map_err(|e| {
            anyhow::anyhow!("loading ONNX model at {}: {e}", model_path.display())
        })?;
    Ok(session)
}

/// Names of the inputs declared by an ONNX model.
pub fn input_names(session: &Session) -> HashSet<String> {
    session
        .inputs()
        .iter()
        .map(|i| i.name().to_string())
        .collect()
}

/// Names of the outputs declared by an ONNX model.
pub fn output_names(session: &Session) -> HashSet<String> {
    session
        .outputs()
        .iter()
        .map(|i| i.name().to_string())
        .collect()
}
