/// Resonance Reverb - An algorithmic reverb using diffusion networks and FDN.
use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod presets;
pub mod viz;

#[cfg(feature = "editor")]
mod editor;

use dsp::ReverbDsp;
use params::{ReverbParams, ReverbSmoothers, PARAM_COUNT};
use viz::ReverbViz;

pub struct ResonanceReverb {
    /// Params shared with the editor via `Arc`. All FloatParam/BoolParam
    /// storage is atomic internally so `&ReverbParams` is safe from both
    /// audio and UI threads.
    pub params: Arc<ReverbParams>,
    /// Audio-thread-only smoothers. Kept outside `params` so the audio
    /// thread can mutate smoother state through `&mut self`.
    smoothers: ReverbSmoothers,
    /// Lock-free meters + tank energies + ER tap snapshot for the editor.
    viz: Arc<ReverbViz>,
    reverb: Option<ReverbDsp>,
}

impl ResonancePlugin for ResonanceReverb {
    const CLAP_ID: &'static str = "com.resonance.reverb";
    const NAME: &'static str = "Resonance Reverb";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "Algorithmic reverb with diffusion network and FDN";
    const FEATURES: &'static [&'static str] = &["audio-effect", "stereo", "reverb"];

    const INPUT_CHANNELS: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(ReverbParams::default()),
            smoothers: ReverbSmoothers::new(),
            viz: ReverbViz::new(),
            reverb: None,
        }
    }

    fn param_count(&self) -> usize {
        PARAM_COUNT
    }

    fn param(&self, index: usize) -> &dyn Param {
        self.params.param_at(index)
    }

    fn initialize(&mut self, sample_rate: f32, _max_buffer_size: u32) -> bool {
        self.smoothers.prepare(sample_rate, &self.params);
        self.reverb = Some(ReverbDsp::new(sample_rate));
        true
    }

    fn reset(&mut self) {
        if let Some(reverb) = &mut self.reverb {
            reverb.clear();
        }
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
        _tempo: Option<TempoInfo>,
    ) {
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut *main.left;
        let right = &mut *main.right;
        resonance_common::flush_denormals();

        let Some(reverb) = &mut self.reverb else {
            return;
        };

        // Update smoother targets from the atomic param values once per block.
        self.smoothers.retarget_from(&self.params);
        let freeze = self.params.freeze.value();

        // Advance the block-rate smoothers to their end-of-block state. These
        // feed expensive DSP updates (transcendentals, 8-channel loops) and
        // don't need per-sample granularity, so the block-rate stair-step is
        // deliberate.
        let n = frames as u32;
        self.smoothers.size.skip(n);
        self.smoothers.decay.skip(n);
        self.smoothers.damping.skip(n);
        self.smoothers.predelay.skip(n);
        self.smoothers.er_level.skip(n);
        self.smoothers.er_time.skip(n);
        self.smoothers.mod_rate.skip(n);
        self.smoothers.mod_depth.skip(n);

        reverb.set_size(self.smoothers.size.current());
        reverb.set_decay(self.smoothers.decay.current());
        reverb.set_freeze(freeze);
        reverb.set_damping(self.smoothers.damping.current());
        reverb.set_predelay(self.smoothers.predelay.current());
        reverb.set_er_level(self.smoothers.er_level.current());
        reverb.set_er_time(self.smoothers.er_time.current());
        reverb.set_mod_rate(self.smoothers.mod_rate.current());
        reverb.set_mod_depth(self.smoothers.mod_depth.current());

        // Track peaks for the meter widgets.
        let mut in_l_peak = 0.0f32;
        let mut in_r_peak = 0.0f32;
        let mut out_l_peak = 0.0f32;
        let mut out_r_peak = 0.0f32;

        for i in 0..frames {
            let mix = self.smoothers.mix.next();
            let width = self.smoothers.width.next();
            let diffusion = self.smoothers.diffusion.next();

            let dry_l = left[i];
            let dry_r = right[i];
            in_l_peak = in_l_peak.max(dry_l.abs());
            in_r_peak = in_r_peak.max(dry_r.abs());

            let (wet_l, wet_r) = reverb.process(dry_l, dry_r, diffusion, width);

            let dry_amount = 1.0 - mix;
            let out_l = dry_l * dry_amount + wet_l * mix;
            let out_r = dry_r * dry_amount + wet_r * mix;
            left[i] = out_l;
            right[i] = out_r;
            out_l_peak = out_l_peak.max(out_l.abs());
            out_r_peak = out_r_peak.max(out_r.abs());
        }

        // Publish block-rate viz state. All lock-free except the tail ring.
        self.viz.store_peaks(
            linear_to_db(in_l_peak),
            linear_to_db(in_r_peak),
            linear_to_db(out_l_peak),
            linear_to_db(out_r_peak),
        );
        self.viz.store_channel_energies(&reverb.channel_energies());
        self.viz.store_fdn_delay_ms(&reverb.fdn_delay_ms());
        self.viz
            .store_er_taps(&reverb.er_tap_times_ms(), &reverb.er_tap_gains());
        self.viz.push_tail_rms(reverb.take_wet_rms());
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::ReverbEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
        )))
    }
}

use resonance_dsp::linear_to_db;

resonance_plugin::export_clap!(ResonanceReverb);

