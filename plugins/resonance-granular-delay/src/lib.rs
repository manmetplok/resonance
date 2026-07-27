//! Resonance Granular Delay — a circular delay buffer granulated by
//! many enveloped, independently transposed read heads behind the write
//! head (doc #252, doc #253; epic #196).
//!
//! This crate is the plugin skeleton (ba todo #1073): ring buffer,
//! write head, grain engines and the full §9 parameter surface.
//! Feedback (#1074), freeze (#1075), time modes (#1076), stereo width
//! (#1077), shimmer/quantize (#1078), the editor (#1079), pitch-sync
//! scheduling (#1082) and quality tiers (#1083) land on top of the
//! seams marked `TODO(epic-196 #...)`.

use std::sync::Arc;

use resonance_dsp::SchedulerMode;
use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod sync;

#[cfg(feature = "editor")]
mod editor;

use dsp::GranularDsp;
use params::{GranularDelayParams, GranularSmoothers, PARAM_COUNT};

pub struct ResonanceGranularDelay {
    pub params: Arc<GranularDelayParams>,
    smoothers: GranularSmoothers,
    dsp: Option<GranularDsp>,
    sample_rate: f32,
}

impl ResonanceGranularDelay {
    /// Total grains spawned since activation (metering aid; exposed for
    /// tests and the future editor, ba todo #1079).
    pub fn grains_spawned(&self) -> u64 {
        self.dsp.as_ref().map_or(0, GranularDsp::grains_spawned)
    }

    /// Currently sounding grains (metering aid).
    pub fn active_grains(&self) -> usize {
        self.dsp.as_ref().map_or(0, GranularDsp::active_grains)
    }
}

impl ResonancePlugin for ResonanceGranularDelay {
    const CLAP_ID: &'static str = "com.resonance.granular-delay";
    const NAME: &'static str = "Resonance Granular Delay";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Granular delay with per-grain pitch, texture and jitter";
    const FEATURES: &'static [&'static str] = &["audio-effect", "stereo", "delay", "granular"];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(GranularDelayParams::default()),
            smoothers: GranularSmoothers::new(),
            dsp: None,
            sample_rate: 48_000.0,
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, max_buffer_size: u32) -> bool {
        self.sample_rate = sample_rate;
        self.smoothers.prepare(sample_rate, &self.params);
        self.dsp = Some(GranularDsp::new(sample_rate, max_buffer_size as usize));
        true
    }

    fn reset(&mut self) {
        if let Some(dsp) = &mut self.dsp {
            dsp.clear();
        }
    }

    fn process(
        &mut self,
        outputs: &mut [OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
    ) {
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut *main.left;
        let right = &mut *main.right;
        // FTZ/DAZ guard (doc #252 §5/§8): grain tails and future
        // feedback recursions decay into denormal range otherwise.
        resonance_common::flush_denormals();

        let Some(dsp) = &mut self.dsp else {
            return;
        };

        self.smoothers.retarget_from(&self.params);

        // Delay time is *grain-latched*: each grain reads the position
        // current at its spawn, so time changes granulate over per the
        // default Per-Grain time mode. TODO(epic-196 #1076): Fade and
        // Repitch modes.
        let delay_seconds = sync::delay_seconds(
            self.params.sync.value(),
            self.params.division.value() as usize,
            self.params.time_ms.value(),
            tempo,
            dsp::MAX_DELAY_SECONDS,
        );

        let scheduler = match self.params.scheduler.value() {
            0 => SchedulerMode::Sync,
            // TODO(epic-196 #1082): 2 = Pitch-Sync (PSOLA-style mono
            // mode via resonance-dsp's PitchTracker); Async until then.
            _ => SchedulerMode::Async,
        };

        let block = dsp::BlockParams {
            delay_seconds,
            grain_seconds: self.params.grain_size_ms.value() * 0.001,
            density_hz: self.params.density_hz.value(),
            scheduler,
            pitch_semitones: self.params.pitch.value(),
            detune_spread_cents: self.params.spread_cents.value(),
            texture: self.params.texture.value(),
            spray_seconds: self.params.spray_ms.value() * 0.001,
            size_jitter: self.params.size_jitter.value(),
            level_jitter: self.params.level_jitter.value(),
            reverse_probability: self.params.reverse_prob.value(),
            pan_spread: self.params.pan_spread.value(),
            // TODO(epic-196 #1083): full Lo-fi/Normal/HQ tier treatment;
            // HQ already engages the engine's tracked anti-alias filter.
            anti_alias: self.params.quality.value() == 2,
        };

        dsp.process_block(left, right, frames, &mut self.smoothers, &block);
    }
}

resonance_plugin::export_clap!(ResonanceGranularDelay);
