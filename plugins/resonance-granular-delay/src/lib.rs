//! Resonance Granular Delay — a circular delay buffer granulated by
//! many enveloped, independently transposed read heads behind the write
//! head (doc #252, doc #253; epic #196).
//!
//! This crate is the plugin skeleton (ba todo #1073) — ring buffer,
//! write head, grain engines and the full §9 parameter surface — plus
//! the feedback path (#1074: in-loop damping, tanh soft clip, DC
//! blocker, Wet→Buffer / Output-only topologies, 0–110 %),
//! freeze/hold with crossfaded resume (#1075) and the stereo stage
//! (#1077: per-grain pan, decorrelated L/R scheduling behind Pan
//! Spread, smoothed M/S width on the wet sum, ping-pong feedback).
//! Shimmer and musical pitch land in #1078: FB Pitch selects
//! cumulative (octave-climb) vs constant-pitch recirculation, and the
//! per-grain effective transpose can be quantized to semitones or a
//! resonance-music-theory scale at spawn (`quantize` module).
//! Time modes (#1076), the editor (#1079), pitch-sync scheduling
//! (#1082) and quality tiers (#1083) land on top of the seams marked
//! `TODO(epic-196 #...)`.

use std::sync::Arc;

use resonance_dsp::SchedulerMode;
use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod pitch_sync;
pub mod quantize;
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

    /// Left grain source ring (test/metering aid; empty before
    /// activation). See [`GranularDsp::ring_l`] for the indexing rule.
    pub fn ring_l(&self) -> &[f32] {
        self.dsp.as_ref().map_or(&[], GranularDsp::ring_l)
    }

    /// Right grain source ring (test/metering aid).
    pub fn ring_r(&self) -> &[f32] {
        self.dsp.as_ref().map_or(&[], GranularDsp::ring_r)
    }

    /// Absolute write-head position in samples (test/metering aid).
    pub fn write_head(&self) -> u64 {
        self.dsp.as_ref().map_or(0, GranularDsp::write_head)
    }

    /// Playback rates of the currently sounding audible grains
    /// (test/metering aid; empty before activation). With pitch
    /// quantization on, every rate sits on the quantized lattice
    /// (ba todo #1078).
    pub fn active_rates(&self) -> impl Iterator<Item = f64> + '_ {
        self.dsp.iter().flat_map(GranularDsp::active_rates)
    }

    /// Playback rates of the decorrelated right engine's grains
    /// (test/metering aid).
    pub fn active_rates_decor(&self) -> impl Iterator<Item = f64> + '_ {
        self.dsp.iter().flat_map(GranularDsp::active_rates_decor)
    }

    /// Playback rates of the feedback-tap engine's grains
    /// (test/metering aid; all ±1 while the un-transposed tap runs).
    pub fn active_rates_fb(&self) -> impl Iterator<Item = f64> + '_ {
        self.dsp.iter().flat_map(GranularDsp::active_rates_fb)
    }

    /// Current effective delay-tap position in seconds (test/metering
    /// aid, ba todo #1076): the resolved target in Per-Grain mode, the
    /// glide value in Repitch mode, the committed tap in Fade mode.
    /// 0 before activation.
    pub fn effective_delay_seconds(&self) -> f32 {
        self.dsp
            .as_ref()
            .map_or(0.0, GranularDsp::effective_delay_seconds)
    }

    /// Whether the pitch-synchronous Voice/Mono scheduler was engaged
    /// on the last processed block (test/metering aid, ba todo #1082).
    pub fn pitch_sync_engaged(&self) -> bool {
        self.dsp.as_ref().is_some_and(GranularDsp::pitch_sync_engaged)
    }

    /// Total PSOLA onsets spawned since activation (test/metering aid).
    pub fn psola_onsets(&self) -> u64 {
        self.dsp.as_ref().map_or(0, GranularDsp::psola_onsets)
    }

    /// Currently sounding PSOLA voices (test/metering aid).
    pub fn psola_active_voices(&self) -> usize {
        self.dsp.as_ref().map_or(0, GranularDsp::psola_active_voices)
    }

    /// Recent PSOLA onset times, absolute output samples, oldest first
    /// (test aid; allocates — off the audio path).
    pub fn psola_recent_onsets(&self) -> Vec<f64> {
        self.dsp
            .as_ref()
            .map_or_else(Vec::new, GranularDsp::psola_recent_onsets)
    }

    /// Last known tracked fundamental period, full-rate samples
    /// (0 until the first voiced lock; test/metering aid).
    pub fn tracked_period_samples(&self) -> f32 {
        self.dsp
            .as_ref()
            .map_or(0.0, GranularDsp::tracked_period_samples)
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

        // Time-change behaviour (ba todo #1076): the resolved target
        // routes through the selected Time Mode in the DSP core —
        // Per-Grain (default; grain-latched, in-flight grains keep
        // their origin), Fade (dual-tap swap through silence, ~20 ms)
        // or Repitch (one-pole slew with the tape-style pitch swoop).
        let delay_seconds = sync::delay_seconds(
            self.params.sync.value(),
            self.params.division.value() as usize,
            self.params.time_ms.value(),
            tempo,
            dsp::MAX_DELAY_SECONDS,
        );

        // Scheduler 2 = Pitch-Sync (ba todo #1082): the PSOLA voice
        // path engages in the DSP core while the tracker is voiced; the
        // engines run Async underneath as the unvoiced/unlocked
        // fallback (and drain while the voice path is engaged).
        let scheduler_value = self.params.scheduler.value();
        let scheduler = match scheduler_value {
            0 => SchedulerMode::Sync,
            _ => SchedulerMode::Async,
        };

        let block = dsp::BlockParams {
            delay_seconds,
            time_mode: match self.params.time_mode.value() {
                0 => dsp::TimeMode::Fade,
                1 => dsp::TimeMode::Repitch,
                _ => dsp::TimeMode::PerGrain,
            },
            pitch_sync: scheduler_value == 2,
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
            fb_route: match self.params.fb_route.value() {
                0 => dsp::FbRoute::WetToBuffer,
                2 => dsp::FbRoute::PingPong,
                _ => dsp::FbRoute::OutputOnly,
            },
            filter_is_highpass: self.params.filter_type.value() == 1,
            freeze: self.params.freeze.value(),
            fb_pitch: self.params.fb_pitch.value(),
            quantize: quantize::PitchQuantize::from_index(self.params.pitch_quantize.value()),
            scale: resonance_music_theory::Scale::new(
                quantize::root_from_index(self.params.root.value()),
                quantize::mode_from_index(self.params.scale.value()),
            ),
        };

        dsp.process_block(left, right, frames, &mut self.smoothers, &block);
    }
}

resonance_plugin::export_clap!(ResonanceGranularDelay);
