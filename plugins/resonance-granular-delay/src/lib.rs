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

use resonance_plugin::*;

pub mod choice;
pub mod dsp;
pub mod params;
pub mod pitch_sync;
pub mod presets;
pub mod quantize;
pub mod sync;
pub mod viz;

// Public so the group/label tables get unit tests
// (tests/editor_groups.rs); the factory is only consumed through
// `editor_factory` below.
#[cfg(feature = "editor")]
pub mod editor;

use choice::ChoiceParam;
use dsp::GranularDsp;
use params::{GranularDelayParams, GranularSmoothers, PARAM_COUNT};
use viz::GranularViz;

pub struct ResonanceGranularDelay {
    pub params: Arc<GranularDelayParams>,
    /// Which preset is loaded and whether it has been edited since.
    /// Shared with the editor thread and handed to the bridge as this
    /// plugin's extra state, so the identity survives closing the
    /// window (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Handed to the editor so its controls announce edits to the host
    /// (attached in `set_host`).
    editor_announcer: resonance_plugin::EditAnnouncer,
    smoothers: GranularSmoothers,
    viz: Arc<GranularViz>,
    dsp: Option<GranularDsp>,
    sample_rate: f32,
}

impl ResonanceGranularDelay {
    /// Shared editor metering state (read by the editor each frame;
    /// exposed for tests, ba todo #1079).
    pub fn viz(&self) -> &GranularViz {
        &self.viz
    }

    /// Total grains spawned since activation (metering aid; exposed for
    /// tests and the editor, ba todo #1079).
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

    /// Grain onsets the WSOLA aligner moved off their nominal position
    /// since activation (ba todo #1320; test/metering aid — 0 whenever
    /// the Align parameter is off).
    pub fn aligned_spawns(&self) -> u64 {
        self.dsp.as_ref().map_or(0, GranularDsp::aligned_spawns)
    }

    /// Largest onset-alignment lag magnitude applied since activation,
    /// in samples (test/metering aid).
    pub fn max_align_lag_samples(&self) -> f64 {
        self.dsp
            .as_ref()
            .map_or(0.0, GranularDsp::max_align_lag_samples)
    }

    /// Whether the diffusion stage touched the wet path on the last
    /// block (ba todo #1321; test/metering aid — false at Diffusion 0,
    /// which is what makes the bypass exact).
    pub fn diffusion_engaged(&self) -> bool {
        self.dsp
            .as_ref()
            .is_some_and(GranularDsp::diffusion_engaged)
    }
}

impl ResonancePlugin for ResonanceGranularDelay {
    const CLAP_ID: &'static str = "com.resonance.granular-delay";
    const NAME: &'static str = "Resonance Granular Delay";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str =
        "Granular delay with per-grain pitch, texture and jitter";
    const FEATURES: &'static [&'static std::ffi::CStr] = &[
        features::AUDIO_EFFECT,
        features::DELAY,
        features::GRANULAR,
        features::STEREO,
    ];

    /// The factory bank, declared once here and read by the editor's
    /// PresetBank and by the exported `resonance_factory_presets`
    /// symbol the host lists over the control API (ba todo #1333).
    const FACTORY_PRESETS: &'static [resonance_plugin::presets::FactoryPreset] =
        presets::PRESETS;

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(GranularDelayParams::default()),
            editor_announcer: resonance_plugin::EditAnnouncer::new(),
            presets: resonance_plugin::presets::PresetSession::for_plugin::<Self>(),
            smoothers: GranularSmoothers::new(),
            viz: GranularViz::new(),
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
        resonance_dsp::flush_denormals();

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

        // Scheduler = Pitch-Sync (ba todo #1082): the PSOLA voice path
        // engages in the DSP core while the tracker is voiced; the
        // engines run Async underneath as the unvoiced/unlocked
        // fallback (and drain while the voice path is engaged).
        //
        // Every choice parameter resolves through its enum's
        // `from_index` (ba todo #1267) — the one place that knows what
        // each integer means, and the same table the editor labels come
        // from.
        let scheduler = dsp::Scheduler::from_index(self.params.scheduler.value());

        let block = dsp::BlockParams {
            delay_seconds,
            time_mode: dsp::TimeMode::from_index(self.params.time_mode.value()),
            pitch_sync: scheduler.pitch_sync(),
            grain_seconds: self.params.grain_size_ms.value() * 0.001,
            // Tempo-locked grain rate (ba todo #1322): with PER-BEAT on
            // and a host tempo, one grain per selected division instead
            // of the free-running knob. Resolved per block, so a tempo
            // change re-locks the cloud immediately.
            density_hz: sync::grain_density_hz(
                self.params.density_sync.value(),
                self.params.density_division.value() as usize,
                self.params.density_hz.value(),
                tempo,
            ),
            scheduler: scheduler.engine_mode(),
            // WSOLA onset alignment (ba todo #1320): a plain bool
            // parameter, so it is reachable from the editor chip and
            // from track/bus/master.set_plugin_param alike.
            align: self.params.align.value(),
            pitch_semitones: self.params.pitch.value(),
            detune_spread_cents: self.params.spread_cents.value(),
            texture: self.params.texture.value(),
            spray_seconds: self.params.spray_ms.value() * 0.001,
            size_jitter: self.params.size_jitter.value(),
            level_jitter: self.params.level_jitter.value(),
            reverse_probability: self.params.reverse_prob.value(),
            pan_spread: self.params.pan_spread.value(),
            // Quality tiers (ba todo #1083): Lo-fi = linear reads +
            // µ-law + reduced pool, Normal = Hermite, HQ = 6-pt
            // Lagrange + forced anti-alias. Grain-latched, so tier
            // switches are click-free.
            quality: dsp::QualityTier::from_index(self.params.quality.value()),
            fb_route: dsp::FbRoute::from_index(self.params.fb_route.value()),
            filter_is_highpass: dsp::DampingFilter::from_index(self.params.filter_type.value())
                .is_highpass(),
            freeze: self.params.freeze.value(),
            fb_pitch: self.params.fb_pitch.value(),
            // Allpass smear of the wet path (ba todo #1321); smoothed
            // per sample off `GranularSmoothers::diffusion`.
            diffusion: self.params.diffusion.value(),
            quantize: quantize::PitchQuantize::from_index(self.params.pitch_quantize.value()),
            scale: resonance_music_theory::Scale::new(
                quantize::root_from_index(self.params.root.value()),
                quantize::mode_from_index(self.params.scale.value()),
            ),
        };

        dsp.process_block(left, right, frames, &mut self.smoothers, &block);

        // Block-rate editor metering (ba todo #1079): relaxed atomic
        // stores only — allocation-free, covered by the audio-path
        // no-allocation guards.
        let period = dsp.tracked_period_samples();
        let period_hz = if period > 0.0 {
            self.sample_rate / period
        } else {
            0.0
        };
        self.viz.store_block(
            dsp.effective_delay_seconds() * 1000.0,
            tempo.map_or(0.0, |t| t.bpm),
            period_hz,
            dsp.pitch_sync_engaged(),
            dsp.active_grains(),
            dsp.psola_active_voices(),
        );
        // Grain-snapshot slots + coarse buffer peaks for the editor's
        // hero cloud (ba todo #1135) — same cadence, same contract.
        dsp.publish_viz(&self.viz, &block, self.smoothers.feedback.current());
    }

    /// The loaded-preset identity rides along with the parameter values,
    /// on both bridge paths, so reopening a saved project shows the preset
    /// the sound came from instead of a blank picker.
    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    fn set_host(&mut self, host: Arc<resonance_plugin::HostHandle>) {
        self.editor_announcer.attach(host);
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::GranularEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
            self.editor_announcer.clone(),
        )))
    }
}

resonance_plugin::export_clap!(ResonanceGranularDelay);
