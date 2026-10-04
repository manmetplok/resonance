use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod gate;
pub mod params;
pub mod presets;
pub mod sync;
pub mod viz;

#[cfg(feature = "editor")]
pub mod editor;

use dsp::DelayDsp;
use params::{DelayParams, DelaySmoothers, PARAM_COUNT};
use viz::DelayViz;

pub struct ResonanceDelay {
    pub params: Arc<DelayParams>,
    /// Which preset is loaded and whether it has been edited since.
    /// Shared with the editor thread and handed to the bridge as this
    /// plugin's extra state, so the identity survives closing the
    /// window (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    smoothers: DelaySmoothers,
    viz: Arc<DelayViz>,
    /// Handed to the editor so its controls announce edits to the host
    /// (attached in `set_host`).
    editor_announcer: EditAnnouncer,
    dsp: Option<DelayDsp>,
    sample_rate: f32,
}

impl ResonanceDelay {
    /// Shared viz state (read by the editor; exposed for tests).
    pub fn viz(&self) -> &DelayViz {
        &self.viz
    }
}

impl ResonancePlugin for ResonanceDelay {
    const CLAP_ID: &'static str = resonance_plugin::first_party::DELAY;
    const NAME: &'static str = "Resonance Delay";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "Tempo-synced stereo delay with digital and analog modes";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[features::AUDIO_EFFECT, features::DELAY, features::STEREO];

    /// The factory bank, declared once here and read by the editor's
    /// PresetBank and by the exported `resonance_factory_presets`
    /// symbol the host lists over the control API (ba todo #1333).
    const FACTORY_PRESETS: &'static [resonance_plugin::presets::FactoryPreset] =
        presets::PRESETS;

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(DelayParams::default()),
            presets: resonance_plugin::presets::PresetSession::for_plugin::<Self>(),
            smoothers: DelaySmoothers::new(),
            viz: DelayViz::new(),
            editor_announcer: EditAnnouncer::new(),
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

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.sample_rate = sample_rate;
        self.smoothers.prepare(sample_rate, &self.params);
        self.dsp = Some(DelayDsp::new(sample_rate));
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
        resonance_dsp::flush_denormals();

        let Some(dsp) = &mut self.dsp else {
            return;
        };

        self.smoothers.retarget_from(&self.params);

        let sync = self.params.sync.value();
        let division = self.params.division.value() as usize;
        let character = self.params.character.value();
        let routing = self.params.routing.value();
        let freeze = self.params.freeze.value();

        let max_delay = (self.sample_rate * 4.0) + 256.0;

        // Resolve sync/division/time/tempo into a delay-in-samples target
        // and smooth that, so a sync toggle or division change glides the
        // read tap instead of jumping it (a click).
        self.smoothers.delay_samples.set_target(sync::delay_samples(
            sync,
            division,
            self.params.time_ms.value(),
            tempo,
            self.sample_rate,
            max_delay,
        ));

        // Block-rate smoothers for filter parameters.
        let n = frames as u32;
        self.smoothers.hi_cut.skip(n);
        self.smoothers.lo_cut.skip(n);
        self.smoothers.drive.skip(n);
        self.smoothers.mod_rate.skip(n);
        self.smoothers.mod_depth.skip(n);
        self.smoothers.stereo_offset.skip(n);

        let hi_cut = self.smoothers.hi_cut.current();
        let lo_cut = self.smoothers.lo_cut.current();
        let drive = self.smoothers.drive.current();
        let mod_rate = self.smoothers.mod_rate.current();
        let mod_depth = self.smoothers.mod_depth.current();
        let stereo_offset = self.smoothers.stereo_offset.current();

        // Set tone filter coefficients once per block (avoids per-sample trig).
        dsp.set_tone_filters(hi_cut, lo_cut, character, self.smoothers.delay_samples.current());

        // Update viz with current BPM.
        if let Some(t) = tempo {
            self.viz.store_bpm(t.bpm);
        }

        let peaks = dsp.process_block(
            left,
            right,
            frames,
            &mut self.smoothers,
            &dsp::BlockParams {
                character,
                routing,
                stereo_offset,
                drive,
                mod_rate,
                mod_depth,
                freeze,
                gate_duck: gate::GateDuckParams {
                    gate_on: self.params.gate_on.value(),
                    gate_period: gate::gate_period_samples(
                        self.params.gate_rate.value() as usize,
                        tempo,
                        self.sample_rate,
                    ),
                    gate_phase: gate::gate_phase_at(self.params.gate_rate.value() as usize, tempo),
                    gate_width: self.params.gate_width.value(),
                    gate_edge: self.params.gate_shape.value(),
                    gate_depth: self.params.gate_depth.value(),
                    duck_amount: self.params.duck_amount.value(),
                    duck_threshold_db: self.params.duck_threshold.value(),
                    duck_release_ms: self.params.duck_release.value(),
                },
            },
        );

        self.viz.store_peaks(
            linear_to_db(peaks.in_l),
            linear_to_db(peaks.in_r),
            linear_to_db(peaks.out_l),
            linear_to_db(peaks.out_r),
        );
        let delay_ms = self.smoothers.delay_samples.current() / self.sample_rate * 1000.0;
        self.viz.store_delay_time_ms(delay_ms);

        // Echo tap positions for the viz. The right train carries the
        // stereo offset and, on ping-pong, alternates channels — so the
        // picture matches what the DSP above actually does.
        let delay_r_ms = delay_ms * (1.0 + stereo_offset);
        self.viz.store_taps(&viz::echo_taps(
            delay_ms,
            delay_r_ms,
            self.smoothers.feedback.current(),
            routing,
        ));
    }

    /// The loaded-preset identity rides along with the parameter values,
    /// on both bridge paths, so reopening a saved project shows the preset
    /// the sound came from instead of a blank picker.
    fn extra_state_saver(&self) -> Option<Arc<dyn resonance_plugin::ExtraStateSaver>> {
        Some(self.presets.clone())
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::DelayEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
            self.editor_announcer.clone(),
        )))
    }

    fn set_host(&mut self, host: Arc<HostHandle>) {
        self.editor_announcer.attach(host);
    }

    fn param_text_source(&self) -> Option<Arc<dyn resonance_plugin::ParamTextSource>> {
        // FU-P1a: the params are shared, so a host reads a live
        // instance's real values while the plugin is in the audio
        // processor — without this, a third-party host that never
        // flushes between blocks sees a stale mirror for any value an
        // editor edit moved while the transport is stopped.
        Some(Arc::new(DelayParamText(self.params.clone())))
    }
}

use resonance_dsp::linear_to_db;

/// Parameter text and live values over the shared `DelayParams`, for
/// the CLAP bridge while the plugin is active (FU-P1a).
struct DelayParamText(Arc<DelayParams>);

impl resonance_plugin::ParamTextSource for DelayParamText {
    fn display(&self, index: usize, value: f64) -> Option<String> {
        (index < PARAM_COUNT).then(|| self.0.param_at(index).display(value))
    }

    fn parse(&self, index: usize, text: &str) -> Option<f64> {
        if index >= PARAM_COUNT {
            return None;
        }
        self.0.param_at(index).parse(text)
    }

    fn live_value(&self, index: usize) -> Option<f64> {
        (index < PARAM_COUNT).then(|| self.0.param_at(index).get_plain())
    }
}

resonance_plugin::export_clap!(ResonanceDelay);

