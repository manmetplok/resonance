//! Resonance Reverb - An algorithmic reverb using diffusion networks and FDN.
//!
//! Besides the room itself it carries the three return-channel moves a
//! mix reaches for a second plugin to make (warmth-width-depth.md §6.4):
//! a wet HPF/LPF *before* the tank (return EQ), ducking of the wet return
//! from an external sidechain key or the dry input, and an ER/tail depth
//! balance. All three default to a no-op.
//!
//! The sidechain key only drives the ducker's detector; it never reaches
//! the output. With no key connected the ducker keys off the dry input.
//!
//! The room itself is one of several engines (`algorithm`, reverb-
//! algorithms.md); pre-delay and decay can follow the host tempo
//! (`predelay_sync`, `decay_sync`).

use std::sync::Arc;

use resonance_plugin::*;

pub mod dsp;
pub mod params;
pub mod presets;
pub mod sync;
pub mod viz;

#[cfg(feature = "editor")]
pub mod editor;

use dsp::{Ducker, ReverbDsp};
use params::{ReverbParams, ReverbSmoothers, PARAM_COUNT};
use viz::ReverbViz;

pub struct ResonanceReverb {
    /// Params shared with the editor via `Arc`. All FloatParam/BoolParam
    /// storage is atomic internally so `&ReverbParams` is safe from both
    /// audio and UI threads.
    pub params: Arc<ReverbParams>,
    /// Which preset is loaded and whether it has been edited since.
    /// Shared with the editor thread and handed to the bridge as this
    /// plugin's extra state, so the identity survives closing the
    /// window (ba todo #1358).
    presets: Arc<resonance_plugin::presets::PresetSession>,
    /// Handed to the editor so its controls announce edits to the host
    /// (attached in `set_host`).
    editor_announcer: resonance_plugin::EditAnnouncer,
    /// Audio-thread-only smoothers. Kept outside `params` so the audio
    /// thread can mutate smoother state through `&mut self`.
    smoothers: ReverbSmoothers,
    /// Lock-free meters + tank energies + ER tap snapshot for the editor.
    viz: Arc<ReverbViz>,
    reverb: Option<ReverbDsp>,
    /// Wet-return ducker; `None` until `initialize`.
    ducker: Option<Ducker>,
    /// False until the first block after `initialize`: that block starts
    /// a synced decay's smoother on its target instead of ramping to it
    /// from the knob value.
    sync_primed: bool,
}

impl ResonancePlugin for ResonanceReverb {
    const CLAP_ID: &'static str = resonance_plugin::first_party::REVERB;
    const NAME: &'static str = "Resonance Reverb";
    const VENDOR: &'static str = "Resonance";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");
    const DESCRIPTION: &'static str = "Algorithmic reverb with diffusion network and FDN";
    const FEATURES: &'static [&'static std::ffi::CStr] =
        &[features::AUDIO_EFFECT, features::REVERB, features::STEREO];

    /// The factory bank, declared once here and read by the editor's
    /// PresetBank and by the exported `resonance_factory_presets`
    /// symbol the host lists over the control API (ba todo #1333).
    const FACTORY_PRESETS: &'static [resonance_plugin::presets::FactoryPreset] =
        presets::PRESETS;

    const INPUT_CHANNELS: Option<u32> = Some(2);
    /// Stereo key for the wet-return ducker.
    const SIDECHAIN_INPUT: Option<u32> = Some(2);

    fn new() -> Self {
        Self {
            params: Arc::new(ReverbParams::default()),
            editor_announcer: resonance_plugin::EditAnnouncer::new(),
            presets: resonance_plugin::presets::PresetSession::for_plugin::<Self>(),
            smoothers: ReverbSmoothers::new(),
            viz: ReverbViz::new(),
            reverb: None,
            ducker: None,
            sync_primed: false,
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
        self.sync_primed = false;
        self.ducker = Some(Ducker::new(
            sample_rate,
            dsp::duck::INITIAL_ATTACK_MS,
            dsp::duck::INITIAL_RELEASE_MS,
        ));
        true
    }

    fn reset(&mut self) {
        if let Some(reverb) = &mut self.reverb {
            reverb.clear();
        }
        if let Some(ducker) = &mut self.ducker {
            ducker.clear();
        }
    }

    fn process(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        _events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
    ) {
        self.render(outputs, frames, None, tempo);
    }

    fn process_with_key(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        key: Option<KeyBuffer<'_>>,
        frames: usize,
        _events: &mut EventIterator<'_>,
        tempo: Option<TempoInfo>,
    ) {
        self.render(outputs, frames, key.map(|k| (k.left, k.right)), tempo);
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

    fn param_text_source(&self) -> Option<Arc<dyn resonance_plugin::ParamTextSource>> {
        // FU-P1a: the params are shared, so a host reads a live
        // instance's real values while the plugin is in the audio
        // processor — without this, a third-party host that never
        // flushes between blocks sees a stale mirror for any value an
        // editor edit moved while the transport is stopped.
        Some(Arc::new(ReverbParamText(self.params.clone())))
    }

    #[cfg(feature = "editor")]
    fn editor_factory(&self) -> Option<Arc<dyn resonance_plugin::gui::EditorFactory>> {
        Some(Arc::new(editor::ReverbEditorFactory::new(
            self.params.clone(),
            self.viz.clone(),
            self.presets.clone(),
            self.editor_announcer.clone(),
        )))
    }
}

/// Parameter text and live values over the shared `ReverbParams`, for
/// the CLAP bridge while the plugin is active (FU-P1a).
struct ReverbParamText(Arc<ReverbParams>);

impl resonance_plugin::ParamTextSource for ReverbParamText {
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

impl ResonanceReverb {
    /// One block. `key` is the external sidechain when the host has
    /// connected one; it only feeds the ducker's detector. `tempo` drives
    /// the pre-delay and decay sync.
    fn render(
        &mut self,
        outputs: &mut [resonance_plugin::OutputBuffer<'_>],
        frames: usize,
        key: Option<(&[f32], &[f32])>,
        tempo: Option<TempoInfo>,
    ) {
        // Routing is a fact about the connection, not about this block
        // having audio in it: publish it before any early return.
        self.viz.store_key_connected(key.is_some());
        let Some(main) = outputs.first_mut() else {
            return;
        };
        let left = &mut *main.left;
        let right = &mut *main.right;
        resonance_dsp::flush_denormals();

        let (Some(reverb), Some(ducker)) = (&mut self.reverb, &mut self.ducker) else {
            return;
        };

        // Tempo sync (reverb-algorithms.md §4.5). A synced decay goes
        // through the decay smoother like a knob move; a synced pre-delay
        // goes straight to the pre-delay's own tap crossfade, so a tempo
        // change never clicks. Without a usable tempo both are `None`
        // and the knobs rule, exactly as before sync existed.
        let synced_predelay = sync::predelay_ms(self.params.predelay_sync.value(), tempo);
        let synced_decay = sync::decay_s(self.params.decay_sync.value(), tempo);
        self.viz.store_synced(synced_predelay, synced_decay);

        // Update smoother targets from the atomic param values once per block.
        self.smoothers.retarget_from(&self.params, synced_decay);
        if let (Some(t60), false) = (synced_decay, self.sync_primed) {
            // The first block starts on the synced decay rather than
            // ramping to it from the knob value `initialize` primed.
            self.smoothers.decay.reset(t60);
        }
        self.sync_primed = true;
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
        self.smoothers.wet_hpf_freq.skip(n);
        self.smoothers.wet_lpf_freq.skip(n);
        self.smoothers.er_tail_balance.skip(n);
        self.smoothers.low_decay_mult.skip(n);
        self.smoothers.low_xover.skip(n);
        self.smoothers.high_decay_mult.skip(n);
        self.smoothers.build.skip(n);

        reverb.set_algorithm(self.params.algorithm());
        reverb.set_size(self.smoothers.size.current());
        reverb.set_decay(self.smoothers.decay.current());
        reverb.set_freeze(freeze);
        reverb.set_damping(self.smoothers.damping.current());
        reverb.set_predelay(synced_predelay.unwrap_or(self.smoothers.predelay.current()));
        reverb.set_er_level(self.smoothers.er_level.current());
        reverb.set_er_time(self.smoothers.er_time.current());
        reverb.set_mod_rate(self.smoothers.mod_rate.current());
        reverb.set_mod_depth(self.smoothers.mod_depth.current());
        reverb.set_wet_filters(
            self.params.wet_hpf_on.value(),
            self.smoothers.wet_hpf_freq.current(),
            self.params.wet_lpf_on.value(),
            self.smoothers.wet_lpf_freq.current(),
            self.params.wet_filter_slope.value() == 1,
        );
        reverb.set_er_tail_balance(self.smoothers.er_tail_balance.current());
        reverb.set_decay_shape(
            self.smoothers.low_decay_mult.current(),
            self.smoothers.low_xover.current(),
            self.smoothers.high_decay_mult.current(),
        );
        reverb.set_build(self.smoothers.build.current());

        let duck_amount = self.params.duck_amount.value();
        let duck_threshold = self.params.duck_threshold.value();
        ducker.set_times(
            self.params.duck_attack.value(),
            self.params.duck_release.value(),
        );

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

            // Duck the wet only. A key shorter than the block reads as
            // silence rather than panicking. The gain is exactly 1.0
            // while ducking is off, and `x * 1.0 == x`, so a reverb that
            // never ducks renders as it did before the ducker existed.
            let (det_l, det_r) = match key {
                Some((kl, kr)) => (
                    kl.get(i).copied().unwrap_or(0.0),
                    kr.get(i).copied().unwrap_or(0.0),
                ),
                None => (dry_l, dry_r),
            };
            let duck = ducker.next_gain(det_l, det_r, duck_amount, duck_threshold);

            let dry_amount = 1.0 - mix;
            let out_l = dry_l * dry_amount + wet_l * mix * duck;
            let out_r = dry_r * dry_amount + wet_r * mix * duck;
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
        self.viz.store_duck_gr_db(ducker.gain_reduction_db());
        self.viz.store_channel_energies(&reverb.channel_energies());
        self.viz.store_fdn_delay_ms(&reverb.fdn_delay_ms());
        self.viz
            .store_er_taps(&reverb.er_tap_times_ms(), &reverb.er_tap_gains());
        self.viz.push_tail_rms(reverb.take_wet_rms());
    }
}

use resonance_dsp::linear_to_db;

resonance_plugin::export_clap!(ResonanceReverb);

