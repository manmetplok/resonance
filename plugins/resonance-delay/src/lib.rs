use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod presets;
pub mod sync;
pub mod viz;

#[cfg(feature = "editor")]
mod editor;

use dsp::DelayDsp;
use params::{DelayParams, DelaySmoothers, PARAM_COUNT};
use viz::DelayViz;

pub struct ResonanceDelay {
    pub params: Arc<DelayParams>,
    smoothers: DelaySmoothers,
    viz: Arc<DelayViz>,
    dsp: Option<DelayDsp>,
    sample_rate: f32,
}

impl ResonancePlugin for ResonanceDelay {
    const CLAP_ID: &'static str = "com.resonance.delay";
    const NAME: &'static str = "Resonance Delay";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "Tempo-synced stereo delay with digital and analog modes";
    const FEATURES: &'static [&'static str] = &["audio-effect", "stereo", "delay"];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(DelayParams::default()),
            smoothers: DelaySmoothers::new(),
            viz: DelayViz::new(),
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
        resonance_common::flush_denormals();

        let Some(dsp) = &mut self.dsp else {
            return;
        };

        self.smoothers.retarget_from(&self.params);

        let sync = self.params.sync.value();
        let division = self.params.division.value() as usize;
        let character = self.params.character.value();
        let routing = self.params.routing.value();
        let freeze = self.params.freeze.value();

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

        let max_delay = (self.sample_rate * 4.0) + 256.0;

        // Set tone filter coefficients once per block (avoids per-sample trig).
        let block_delay_samp = sync::delay_samples(
            sync,
            division,
            self.smoothers.time_ms.current(),
            tempo,
            self.sample_rate,
            max_delay,
        );
        dsp.set_tone_filters(hi_cut, lo_cut, character, block_delay_samp);

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
                sync,
                division,
                max_delay_samples: max_delay,
            },
            tempo,
        );

        self.viz.store_peaks(
            linear_to_db(peaks.in_l),
            linear_to_db(peaks.in_r),
            linear_to_db(peaks.out_l),
            linear_to_db(peaks.out_r),
        );
        let time_ms_current = self.smoothers.time_ms.current();
        let delay_ms = if sync && tempo.is_some() {
            sync::delay_samples(
                sync,
                division.min(11),
                time_ms_current,
                tempo,
                self.sample_rate,
                max_delay,
            ) / self.sample_rate
                * 1000.0
        } else {
            time_ms_current
        };
        self.viz.store_delay_time_ms(delay_ms);

        // Compute echo tap positions for the viz.
        let mut echo_times_l = [0.0f32; viz::MAX_ECHO_TAPS];
        let mut echo_levels_l = [f32::NEG_INFINITY; viz::MAX_ECHO_TAPS];
        let mut echo_times_r = [0.0f32; viz::MAX_ECHO_TAPS];
        let mut echo_levels_r = [f32::NEG_INFINITY; viz::MAX_ECHO_TAPS];
        let fb = self.smoothers.feedback.current().min(1.0);
        for tap in 0..viz::MAX_ECHO_TAPS {
            let n_taps = (tap + 1) as f32;
            let t_ms = delay_ms * n_taps;
            let level = linear_to_db(fb.powf(n_taps));
            echo_times_l[tap] = t_ms;
            echo_levels_l[tap] = level;
            echo_times_r[tap] = t_ms;
            echo_levels_r[tap] = level;
        }
        self.viz
            .store_echo_taps(&echo_times_l, &echo_levels_l, &echo_times_r, &echo_levels_r);
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::DelayEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
        )))
    }
}

use resonance_dsp::linear_to_db;

resonance_plugin::export_clap!(ResonanceDelay);

