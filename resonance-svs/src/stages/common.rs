use anyhow::Result;
use ort::execution_providers::{
    CPUExecutionProvider, CUDAExecutionProvider, ExecutionProviderDispatch,
    MIGraphXExecutionProvider,
};
use ort::session::{builder::GraphOptimizationLevel, Session};
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

/// ONNX Runtime execution provider an SVS session runs on.
///
/// `Cpu` works out of the box. `Cuda` and `MiGraphX` need the matching
/// cargo feature (`cuda` / `migraphx`) *and* an ONNX Runtime build with
/// that provider on the host; when either is missing, [`build_session`]
/// falls back to CPU with a warning and reports `Cpu` as the backend in
/// use. (ONNX Runtime 1.23 removed the ROCm provider; MIGraphX replaces
/// it for AMD GPUs.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExecutionProvider {
    Cpu,
    Cuda,
    MiGraphX,
}

impl ExecutionProvider {
    /// Parse a CLI/config name. Unknown names mean CPU. `rocm` and `hip`
    /// are accepted as aliases for MIGraphX, the AMD path now that ONNX
    /// Runtime has dropped its ROCm provider.
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "cuda" => Self::Cuda,
            "migraphx" | "rocm" | "hip" => Self::MiGraphX,
            _ => Self::Cpu,
        }
    }

    /// Stable lowercase name (`cpu`, `cuda`, `migraphx`): what
    /// [`active_backend`] reports over the control API.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
            Self::MiGraphX => "migraphx",
        }
    }
}

/// The backend the most recent acoustic session really runs on; `None`
/// until one was created in this process.
static ACTIVE_BACKEND: Mutex<Option<ExecutionProvider>> = Mutex::new(None);

/// The execution provider the most recently created acoustic session is
/// really running on (after any CPU fallback), or `None` before the first
/// render in this process. The vocoder always runs on CPU and is not
/// reported here.
pub fn active_backend() -> Option<ExecutionProvider> {
    *ACTIVE_BACKEND.lock().unwrap_or_else(|e| e.into_inner())
}

/// Record `backend` as the one in use, logging it at info level the first
/// time and whenever it changes. Sessions are rebuilt per render, so this
/// keeps the log to one line per change rather than one per render.
pub(crate) fn record_active_backend(backend: ExecutionProvider) {
    let mut slot = ACTIVE_BACKEND.lock().unwrap_or_else(|e| e.into_inner());
    if *slot != Some(backend) {
        tracing::info!("SVS acoustic model running on the {} backend", backend.as_str());
        *slot = Some(backend);
    }
}

/// Build a session on the requested provider, discarding which backend it
/// ended up on. See [`build_session_reporting`].
pub fn build_session(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Session> {
    build_session_reporting(model_path, ep, device_index).map(|(session, _)| session)
}

/// Build a session on the requested provider and return the backend it
/// really runs on.
///
/// A GPU provider is registered with `error_on_failure`, so ort cannot
/// quietly drop it and run on CPU behind our back. If registration, or
/// session creation on that provider, fails (feature not compiled in, ONNX
/// Runtime build without the provider, driver missing), the session is
/// rebuilt on CPU, a warning names the provider and the reason, and the
/// returned backend is `Cpu`. CPU stays the fail-soft path so a render
/// still happens on a machine without the GPU stack.
pub fn build_session_reporting(
    model_path: &Path,
    ep: ExecutionProvider,
    device_index: i32,
) -> Result<(Session, ExecutionProvider)> {
    let gpu: Option<ExecutionProviderDispatch> = match ep {
        ExecutionProvider::Cpu => None,
        ExecutionProvider::Cuda => Some(
            CUDAExecutionProvider::default()
                .with_device_id(device_index)
                .build()
                .error_on_failure(),
        ),
        ExecutionProvider::MiGraphX => Some(
            MIGraphXExecutionProvider::default()
                .with_device_id(device_index)
                .build()
                .error_on_failure(),
        ),
    };

    if let Some(gpu) = gpu {
        match commit_session(model_path, vec![gpu]) {
            Ok(session) => return Ok((session, ep)),
            Err(e) => tracing::warn!(
                "{} execution provider unavailable for {} ({e:#}); running on CPU instead",
                ep.as_str(),
                model_path.display()
            ),
        }
    }
    let session = commit_session(model_path, vec![CPUExecutionProvider::default().build()])?;
    Ok((session, ExecutionProvider::Cpu))
}

fn commit_session(
    model_path: &Path,
    providers: Vec<ExecutionProviderDispatch>,
) -> Result<Session> {
    Session::builder()
        .map_err(|e| anyhow::anyhow!("creating ORT SessionBuilder: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| anyhow::anyhow!("setting graph optimization level: {e}"))?
        .with_execution_providers(providers)
        .map_err(|e| anyhow::anyhow!("registering execution providers: {e}"))?
        .commit_from_file(model_path)
        .map_err(|e| anyhow::anyhow!("loading ONNX model at {}: {e}", model_path.display()))
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
