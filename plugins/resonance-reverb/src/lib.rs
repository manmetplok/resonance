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
    params: Arc<ReverbParams>,
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

    fn param_count(&self) -> usize { PARAM_COUNT }

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
        self.viz.store_er_taps(&reverb.er_tap_times_ms(), &reverb.er_tap_gains());
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

/// Convert a linear amplitude to dBFS. `0.0` → `-inf`, `1.0` → `0 dB`.
fn linear_to_db(linear: f32) -> f32 {
    if linear <= 1e-9 {
        f32::NEG_INFINITY
    } else {
        20.0 * linear.log10()
    }
}

resonance_plugin::export_clap!(ResonanceReverb);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::PRESETS;

    #[test]
    fn param_enumeration_covers_declared_count() {
        let plugin = ResonanceReverb::new();
        assert_eq!(plugin.param_count(), PARAM_COUNT);
        let mut seen = std::collections::HashSet::new();
        for i in 0..plugin.param_count() {
            let id = plugin.param(i).id().to_string();
            assert!(seen.insert(id.clone()), "duplicate param id: {id}");
        }
    }

    #[test]
    fn every_factory_preset_parses() {
        assert!(!PRESETS.is_empty());
        for entry in PRESETS {
            let value: serde_json::Value = serde_json::from_str(entry.json)
                .unwrap_or_else(|e| panic!("preset {:?} invalid: {e}", entry.name));
            assert!(
                value.get("params").and_then(|v| v.as_object()).is_some(),
                "preset {:?} missing `params` object",
                entry.name
            );
        }
    }

    #[test]
    fn dsp_processes_impulse_without_nans() {
        let mut plugin = ResonanceReverb::new();
        plugin.initialize(48_000.0, 4096);

        let frames = 4096usize;
        let mut left = vec![0.0_f32; frames];
        let mut right = vec![0.0_f32; frames];
        left[0] = 1.0;
        right[0] = 1.0;

        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, frames, &mut ev);

        for &x in left.iter().chain(right.iter()) {
            assert!(x.is_finite(), "non-finite sample: {x}");
        }
        let tail_energy: f32 = left[200..].iter().map(|x| x.abs()).sum();
        assert!(tail_energy > 1e-4, "expected audible tail, got {tail_energy}");
    }

    /// Diagnostic: dumps the impulse response envelope so it's visible
    /// in `cargo test -- --nocapture`. Not an assertion — just a window
    /// into what the reverb actually produces for a unit impulse.
    #[test]
    fn debug_impulse_response_envelope() {
        let mut plugin = ResonanceReverb::new();
        plugin.initialize(48_000.0, 4096);
        plugin.params.mix.set_value(1.0); // 100% wet
        plugin.params.er_level.set_value(0.0); // isolate FDN path
        plugin.params.predelay.set_value(0.0);
        plugin.params.size.set_value(0.5);
        plugin.params.decay.set_value(2.0);

        let frames = 48_000usize; // 1 second
        let mut left = vec![0.0_f32; frames];
        let mut right = vec![0.0_f32; frames];
        left[0] = 1.0;
        right[0] = 1.0;

        // Process in blocks of 4096 so smoothers settle naturally.
        let block = 4096usize;
        let mut pos = 0;
        while pos < frames {
            let n = (frames - pos).min(block);
            let (l_slice, _) = left[pos..pos + n].split_at_mut(n);
            let (r_slice, _) = right[pos..pos + n].split_at_mut(n);
            let mut outs = [OutputBuffer {
                left: l_slice,
                right: r_slice,
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, n, &mut ev);
            pos += n;
        }

        // Print energy in 10 ms windows.
        println!("\nImpulse response envelope (10 ms bins, L channel, size=0.5, decay=2s):");
        let win = 480; // 10 ms @ 48k
        for bin in 0..100 {
            let start = bin * win;
            let end = (start + win).min(frames);
            let peak = left[start..end]
                .iter()
                .map(|x| x.abs())
                .fold(0.0_f32, f32::max);
            let db = if peak > 1e-9 {
                20.0 * peak.log10()
            } else {
                -120.0
            };
            let bars = ((db + 60.0).max(0.0) / 2.0) as usize;
            println!(
                "  t={:4} ms  peak={:8.5}  {:6.1} dB  {}",
                bin * 10,
                peak,
                db,
                "#".repeat(bars)
            );
        }
    }

    /// Regression test for the "reverb onset too late" bug: with default
    /// size (0.5, log-mapped to ~90 ms) and predelay 0, the first wet
    /// energy must arrive within ~50 ms (2400 samples @ 48 kHz). This
    /// pins the FDN channel-0 delay + diffusion latency to something
    /// room-like, not cathedral-like.
    #[test]
    fn wet_onset_is_room_like_not_cathedral() {
        let mut plugin = ResonanceReverb::new();
        plugin.initialize(48_000.0, 4096);
        // Crank mix to 1.0 so the signal we measure is purely wet, and
        // drop ER to 0 to isolate the FDN/diffusion path from the ER path.
        plugin.params.mix.set_value(1.0);
        plugin.params.er_level.set_value(0.0);
        plugin.params.predelay.set_value(0.0);
        plugin.params.size.set_value(0.5);

        let frames = 4096usize;
        let mut left = vec![0.0_f32; frames];
        let mut right = vec![0.0_f32; frames];
        left[0] = 1.0;
        right[0] = 1.0;

        let mut outs = [OutputBuffer {
            left: &mut left,
            right: &mut right,
        }];
        let mut ev = EventIterator::empty();
        plugin.process(&mut outs, frames, &mut ev);

        // Find first sample after t=1 where wet magnitude exceeds a small
        // threshold. Skip index 0 because it contains the dry leak from
        // the one-sample pre-delay tap-before-push ordering.
        let threshold = 0.001f32;
        let first_nonzero = (1..frames)
            .find(|&i| left[i].abs() > threshold)
            .unwrap_or(frames);
        assert!(
            first_nonzero < 2400,
            "wet onset landed at sample {first_nonzero} (> 50 ms @ 48k) — reverb is too late"
        );
    }
}
