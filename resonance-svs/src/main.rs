use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;

use resonance_svs::pipeline;
use resonance_svs::stages::common::ExecutionProvider;

#[derive(Parser, Debug)]
#[command(
    name = "resonance-svs",
    about = "Render a DiffSinger .ds file to WAV via ONNX.",
    version
)]
struct Cli {
    /// Path to the input .ds JSON score.
    #[arg(long = "ds-file")]
    ds_file: PathBuf,

    /// Path to the acoustic `dsconfig.yaml` shipped with the voicebank.
    #[arg(long = "acoustic-config")]
    acoustic_config: PathBuf,

    /// Path to the vocoder `vocoder.yaml` shipped with the vocoder package.
    #[arg(long = "vocoder-config")]
    vocoder_config: PathBuf,

    /// Output WAV path.
    #[arg(long = "out")]
    out: PathBuf,

    /// ONNX Runtime execution provider: cpu, cuda, or rocm.
    #[arg(long = "execution-provider", default_value = "cpu")]
    execution_provider: String,

    /// GPU device index for cuda/rocm providers.
    #[arg(long = "device-index", default_value_t = 0)]
    device_index: i32,

    /// Optional speaker name for multi-speaker voicebanks. Defaults to the first speaker
    /// declared in the acoustic config.
    #[arg(long = "speaker")]
    speaker: Option<String>,

    /// PNDM diffusion speedup ratio (`1` = full sampling, larger = faster but less accurate).
    #[arg(long = "speedup", default_value_t = 10)]
    speedup: i32,

    /// Shallow diffusion depth. Ignored if the acoustic model doesn't declare a `depth`
    /// input. Default 1000 follows Jobsecond's reference; the pipeline clamps to
    /// `acoustic_config.max_depth` and rounds down to a multiple of `speedup`.
    #[arg(long = "depth", default_value_t = 1000)]
    depth: i32,
}

fn main() -> Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();

    let cli = Cli::parse();

    let args = pipeline::PipelineArgs {
        ds_file: cli.ds_file,
        acoustic_config: cli.acoustic_config,
        vocoder_config: cli.vocoder_config,
        out: cli.out,
        execution_provider: ExecutionProvider::parse(&cli.execution_provider),
        device_index: cli.device_index,
        speaker: cli.speaker,
        speedup: cli.speedup,
        depth: cli.depth,
    };

    let summary = pipeline::run(&args).context("running resonance-svs pipeline")?;
    tracing::info!(
        "wrote {} samples ({:.2}s @ {} Hz) to {}",
        summary.total_samples,
        summary.total_samples as f64 / summary.sample_rate as f64,
        summary.sample_rate,
        args.out.display()
    );
    Ok(())
}
