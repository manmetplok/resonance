//! Resonance Mastering — automatic mastering plugin for band music.
//!
//! **Phase 2:** audio passes through unchanged. The plugin is effectively
//! a high-quality analyzer / meter built on top of the `resonance-metering`
//! crate. Later phases add the mastering chain (EQ, compressor, saturator,
//! multiband, M/S imager, true-peak limiter) and the one-shot master
//! assistant that drives it from stored genre target curves or a user-
//! loaded reference track.

use std::sync::Arc;

use resonance_plugin::*;

pub mod assistant;
pub mod chain;
pub mod dsp;
pub mod params;
pub mod stages;
pub mod viz;

#[cfg(feature = "editor")]
mod editor;

use chain::Chain;
use params::MasteringParams;
use viz::MasteringViz;

pub use params::PARAM_COUNT;

pub struct ResonanceMastering {
    params: Arc<MasteringParams>,
    viz: Arc<MasteringViz>,
    chain: Option<Chain>,
}

impl ResonanceMastering {
    /// Test helper: direct access to the parameter struct.
    pub fn params(&self) -> &MasteringParams {
        &self.params
    }

    /// Test helper: direct access to the shared viz state (and its
    /// embedded assistant).
    pub fn viz(&self) -> &MasteringViz {
        &self.viz
    }
}

impl ResonancePlugin for ResonanceMastering {
    const CLAP_ID: &'static str = "com.resonance.mastering";
    const NAME: &'static str = "Resonance Mastering";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Automatic mastering plugin — analyzer + linear-phase mastering chain";
    const FEATURES: &'static [&'static str] = &["audio-effect", "mastering", "analyzer", "stereo"];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(MasteringParams::default()),
            viz: MasteringViz::new(),
            chain: None,
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool {
        self.viz.assistant.set_sample_rate(sample_rate);
        self.chain = Some(Chain::new(sample_rate, max_buffer_size as usize, &self.viz));
        true
    }

    fn reset(&mut self) {
        if let Some(chain) = &mut self.chain {
            chain.reset();
        }
    }

    fn latency_samples(&self) -> u32 {
        self.chain.as_ref().map(|c| c.latency()).unwrap_or(0)
    }

    fn process(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut main.left[..frames];
        let right = &mut main.right[..frames];
        resonance_common::flush_denormals();

        if self.params.bypass.value() {
            return;
        }
        if let Some(chain) = &mut self.chain {
            chain.process(left, right, &self.params, &self.viz);
        }
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::MasteringEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceMastering);
