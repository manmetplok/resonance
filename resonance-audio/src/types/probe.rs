//! Insert-chain probe descriptors (warmth-width-depth.md §7.3).
//!
//! [`AudioCommand::ProbeChain`] names the stages of one insert chain and
//! a stimulus; the engine answers with exactly one
//! [`AudioEvent::ChainProbed`] or [`AudioEvent::ChainProbeError`],
//! echoing `probe_id`. See `engine::probe` for how the chain is cloned.
//!
//! [`AudioCommand::ProbeChain`]: super::AudioCommand::ProbeChain
//! [`AudioEvent::ChainProbed`]: super::AudioEvent::ChainProbed
//! [`AudioEvent::ChainProbeError`]: super::AudioEvent::ChainProbeError

use resonance_metering::probe::HarmonicReport;

use super::PluginInstanceId;

/// One stage of the chain to probe: the LIVE instance whose state is
/// copied, and the bundle to build its offline clone from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeStage {
    /// The live instance, read once for its state and never processed.
    pub instance_id: PluginInstanceId,
    /// The `.clap` bundle the live instance came from.
    pub clap_file_path: String,
    /// Its CLAP plugin id.
    pub clap_plugin_id: String,
}

/// The stimulus.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProbeSpec {
    /// Probe tone, Hz; snapped to an analysis bin
    /// ([`resonance_metering::probe::bin_exact_hz`]).
    pub freq_hz: f64,
    /// Peak level of the tone (and of the IMD pair's sum), dBFS.
    pub level_dbfs: f64,
    /// Also run the SMPTE 60 Hz + 7 kHz 4:1 pair for IMD.
    pub imd: bool,
}

/// How one stage's clone came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbedStage {
    /// The live instance it was cloned from.
    pub instance_id: PluginInstanceId,
    /// Whether the live instance's saved state loaded into the clone. A
    /// plugin without the CLAP state extension probes at its defaults.
    pub state_copied: bool,
}

/// The engine's answer to one probe.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainProbeReport {
    /// One entry per stage, in chain order.
    pub stages: Vec<ProbedStage>,
    /// The harmonic analysis of the steady-state sine output (left
    /// channel; both inputs carry the same tone).
    pub harmonics: HarmonicReport,
    /// SMPTE IMD, %, when asked for and the 7 kHz tone came through.
    pub imd_pct: Option<f64>,
    /// Summed latency the clones report, samples. The analysis starts
    /// after it has passed.
    pub latency_samples: u32,
}
