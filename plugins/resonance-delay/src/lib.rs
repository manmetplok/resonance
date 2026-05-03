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
    params: Arc<DelayParams>,
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

        let mut in_l_peak = 0.0f32;
        let mut in_r_peak = 0.0f32;
        let mut out_l_peak = 0.0f32;
        let mut out_r_peak = 0.0f32;

        for i in 0..frames {
            let time_ms = self.smoothers.time_ms.next();
            let feedback = self.smoothers.feedback.next();
            let mix = self.smoothers.mix.next();

            let delay_samp =
                sync::delay_samples(sync, division, time_ms, tempo, self.sample_rate, max_delay);

            let dry_l = left[i];
            let dry_r = right[i];
            in_l_peak = in_l_peak.max(dry_l.abs());
            in_r_peak = in_r_peak.max(dry_r.abs());

            let (wet_l, wet_r) = dsp.process(
                dry_l,
                dry_r,
                delay_samp,
                feedback,
                character,
                routing,
                stereo_offset,
                drive,
                mod_rate,
                mod_depth,
                freeze,
            );

            let dry_amount = 1.0 - mix;
            let out_l = dry_l * dry_amount + wet_l * mix;
            let out_r = dry_r * dry_amount + wet_r * mix;
            left[i] = out_l;
            right[i] = out_r;
            out_l_peak = out_l_peak.max(out_l.abs());
            out_r_peak = out_r_peak.max(out_r.abs());
        }

        // Publish viz state.
        self.viz.store_peaks(
            linear_to_db(in_l_peak),
            linear_to_db(in_r_peak),
            linear_to_db(out_l_peak),
            linear_to_db(out_r_peak),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::PRESETS;

    #[test]
    fn param_enumeration_covers_declared_count() {
        let plugin = ResonanceDelay::new();
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
        let mut plugin = ResonanceDelay::new();
        plugin.initialize(48_000.0, 4096);

        let frames = 48_000usize;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        left[0] = 1.0;
        right[0] = 1.0;

        let block = 4096;
        let mut pos = 0;
        while pos < frames {
            let n = (frames - pos).min(block);
            let mut outs = [OutputBuffer {
                left: &mut left[pos..pos + n],
                right: &mut right[pos..pos + n],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, n, &mut ev, None);
            pos += n;
        }

        for &x in left.iter().chain(right.iter()) {
            assert!(x.is_finite(), "non-finite sample: {x}");
        }
        let echo_energy: f32 = left[15000..20000].iter().map(|x| x.abs()).sum();
        assert!(
            echo_energy > 1e-4,
            "expected audible echo, got {echo_energy}"
        );
    }

    #[test]
    fn ping_pong_crosses_channels() {
        let mut plugin = ResonanceDelay::new();
        plugin.params.routing.set_value(1); // Ping-Pong
        plugin.params.sync.set_plain(0.0); // free
        plugin.params.time_ms.set_value(100.0); // 100ms = 4800 samples
        plugin.params.feedback.set_value(0.8);
        plugin.params.mix.set_value(1.0);
        plugin.initialize(48_000.0, 4096);

        let frames = 48_000usize;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        // Impulse on left only.
        left[0] = 1.0;

        let block = 4096;
        let mut pos = 0;
        while pos < frames {
            let n = (frames - pos).min(block);
            let mut outs = [OutputBuffer {
                left: &mut left[pos..pos + n],
                right: &mut right[pos..pos + n],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, n, &mut ev, None);
            pos += n;
        }

        // First echo should appear in L (the input channel) around sample 4800.
        // Second echo should appear in R around sample 9600.
        let first_echo_l: f32 = left[4500..5500].iter().map(|x| x.abs()).sum();
        let first_echo_r: f32 = right[9000..10500].iter().map(|x| x.abs()).sum();
        assert!(
            first_echo_l > 0.01,
            "expected L echo at ~4800, got energy {first_echo_l}"
        );
        assert!(
            first_echo_r > 0.01,
            "expected R echo at ~9600, got energy {first_echo_r}"
        );
    }

    #[test]
    fn sync_division_matches_bpm() {
        let mut plugin = ResonanceDelay::new();
        plugin.initialize(48_000.0, 4096);
        plugin.params.sync.set_plain(1.0); // sync on
        plugin.params.division.set_value(4); // 1/4 note
        plugin.params.feedback.set_value(0.5);
        plugin.params.mix.set_value(1.0);
        plugin.params.mod_depth.set_value(0.0);

        let tempo = Some(TempoInfo {
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_den: 4,
            playing: true,
            song_pos_beats: 0.0,
        });

        // At 120 BPM, 1/4 note = 0.5s = 24000 samples.
        let frames = 48_000usize;
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        left[0] = 1.0;
        right[0] = 1.0;

        let block = 4096;
        let mut pos = 0;
        while pos < frames {
            let n = (frames - pos).min(block);
            let mut outs = [OutputBuffer {
                left: &mut left[pos..pos + n],
                right: &mut right[pos..pos + n],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, n, &mut ev, tempo);
            pos += n;
        }

        // Find the first significant echo.
        let threshold = 0.01;
        let first_echo = (1..frames)
            .find(|&i| left[i].abs() > threshold)
            .unwrap_or(frames);
        // Should be within 200 samples of 24000.
        assert!(
            (first_echo as i64 - 24000).unsigned_abs() < 200,
            "expected echo at ~24000, got {first_echo}"
        );
    }

    #[test]
    fn freeze_sustains_signal() {
        let mut plugin = ResonanceDelay::new();
        plugin.initialize(48_000.0, 4096);
        plugin.params.sync.set_plain(0.0);
        plugin.params.time_ms.set_value(100.0);
        plugin.params.feedback.set_value(0.8);
        plugin.params.mix.set_value(1.0);

        // Feed a tone for 0.5s.
        let sr = 48_000usize;
        let tone_frames = sr / 2;
        let total_frames = sr * 2;
        let mut left = vec![0.0f32; total_frames];
        let mut right = vec![0.0f32; total_frames];
        for i in 0..tone_frames {
            let t = i as f32 / sr as f32;
            let s = (440.0 * t * std::f32::consts::TAU).sin() * 0.5;
            left[i] = s;
            right[i] = s;
        }

        // Process first half (with signal).
        let block = 4096;
        let mut pos = 0;
        while pos < tone_frames {
            let n = (tone_frames - pos).min(block);
            let mut outs = [OutputBuffer {
                left: &mut left[pos..pos + n],
                right: &mut right[pos..pos + n],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, n, &mut ev, None);
            pos += n;
        }

        // Enable freeze.
        plugin.params.freeze.set_plain(1.0);

        // Measure RMS right after freeze.
        let pre_freeze_start = tone_frames;
        let pre_freeze_end = (tone_frames + sr / 10).min(total_frames);
        while pos < pre_freeze_end {
            let n = (pre_freeze_end - pos).min(block);
            let mut outs = [OutputBuffer {
                left: &mut left[pos..pos + n],
                right: &mut right[pos..pos + n],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, n, &mut ev, None);
            pos += n;
        }
        let rms_early: f32 = left[pre_freeze_start..pre_freeze_end]
            .iter()
            .map(|x| x * x)
            .sum::<f32>()
            / (pre_freeze_end - pre_freeze_start) as f32;

        // Continue for another second.
        while pos < total_frames {
            let n = (total_frames - pos).min(block);
            let mut outs = [OutputBuffer {
                left: &mut left[pos..pos + n],
                right: &mut right[pos..pos + n],
            }];
            let mut ev = EventIterator::empty();
            plugin.process(&mut outs, n, &mut ev, None);
            pos += n;
        }

        let late_start = total_frames - sr / 4;
        let rms_late: f32 = left[late_start..total_frames]
            .iter()
            .map(|x| x * x)
            .sum::<f32>()
            / (total_frames - late_start) as f32;

        // Frozen signal should sustain within 6 dB of the early level.
        let ratio_db = 10.0 * (rms_late / rms_early.max(1e-12)).log10();
        assert!(
            ratio_db > -6.0,
            "freeze signal decayed too much: {ratio_db:.1} dB"
        );
    }
}
