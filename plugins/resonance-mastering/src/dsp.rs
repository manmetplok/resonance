//! Metering core — owns every measurement DSP struct and pumps the
//! per-block results into the shared [`MasteringViz`].
//!
//! Phase 2 does not modify audio. This module is effectively a sidechain
//! tap: the plugin's `process()` passes audio through unchanged and calls
//! [`MeteringCore::feed`] once per block to update the meters.

use std::sync::Arc;

use resonance_metering::{
    CorrelationMeter, CrestMeter, LraMeter, LufsMeter, MeterSnapshot, PlrMeter, SpectrumAnalyzer,
    TruePeakMeter,
};

use crate::viz::MasteringViz;

/// How often to push a short-term mean-square into the LRA meter.
const LRA_TICK_SECONDS: f32 = 1.0;
/// How often to push a sample into the LUFS-momentary and TP history rings.
const HISTORY_TICK_SECONDS: f32 = 0.06; // ~17 Hz, matches 60 fps UI nicely.

pub struct MeteringCore {
    lufs: LufsMeter,
    true_peak: TruePeakMeter,
    spectrum: SpectrumAnalyzer,
    correlation: CorrelationMeter,
    crest: CrestMeter,
    lra: LraMeter,

    samples_since_lra: usize,
    lra_tick_samples: usize,
    samples_since_history: usize,
    history_tick_samples: usize,
}

impl MeteringCore {
    pub fn new(sample_rate: f32, viz: &MasteringViz) -> Self {
        let spectrum = SpectrumAnalyzer::spawn(sample_rate);
        viz.set_spectrum_handle(spectrum.handle());
        Self {
            lufs: LufsMeter::new(sample_rate),
            true_peak: TruePeakMeter::new(),
            spectrum,
            correlation: CorrelationMeter::new(sample_rate),
            crest: CrestMeter::new(sample_rate),
            lra: LraMeter::new(),
            samples_since_lra: 0,
            lra_tick_samples: (LRA_TICK_SECONDS * sample_rate) as usize,
            samples_since_history: 0,
            history_tick_samples: (HISTORY_TICK_SECONDS * sample_rate).max(1.0) as usize,
        }
    }

    pub fn reset(&mut self) {
        self.lufs.reset();
        self.true_peak.reset();
        self.spectrum.reset();
        self.correlation.reset();
        self.crest.reset();
        self.lra.reset();
        self.samples_since_lra = 0;
        self.samples_since_history = 0;
    }

    /// Feed one stereo block to every metering stream and publish a
    /// fresh snapshot. Audio is not modified.
    pub fn feed(&mut self, left: &[f32], right: &[f32], viz: &MasteringViz) {
        let frames = left.len().min(right.len());
        if frames == 0 {
            return;
        }

        self.lufs.push_stereo(left, right);
        self.true_peak.push_stereo(left, right);
        self.spectrum.push_stereo(left, right);
        self.correlation.push_stereo(left, right);
        self.crest.push_stereo(left, right);

        self.tick_lra(frames);
        self.publish_snapshot(viz);
        self.tick_history(frames, viz);
    }

    fn tick_lra(&mut self, frames: usize) {
        self.samples_since_lra += frames;
        while self.samples_since_lra >= self.lra_tick_samples {
            self.samples_since_lra -= self.lra_tick_samples;
            let st = self.lufs.short_term_lufs();
            if st.is_finite() {
                // Invert the LUFS formula to recover mean-square energy.
                let ms = 10.0_f64.powf((st as f64 + 0.691) / 10.0);
                self.lra.push_short_term_mean_square(ms);
            }
        }
    }

    fn publish_snapshot(&self, viz: &MasteringViz) {
        let (tp_l, tp_r) = self.true_peak.per_channel_dbtp();
        let tp_max = self.true_peak.peak_dbtp();
        let integrated = self.lufs.integrated_lufs();
        let short_term = self.lufs.short_term_lufs();
        let plr = PlrMeter::compute(tp_max, tp_max, integrated, short_term);
        let snap = MeterSnapshot {
            momentary_lufs: self.lufs.momentary_lufs(),
            short_term_lufs: short_term,
            integrated_lufs: integrated,
            true_peak_left_dbtp: tp_l,
            true_peak_right_dbtp: tp_r,
            true_peak_max_dbtp: tp_max,
            correlation: self.correlation.correlation(),
            crest_db: self.crest.crest_db(),
            plr_db: plr.plr_db,
            psr_db: plr.psr_db,
            lra_lu: self.lra.lra_lu(),
        };
        viz.snapshot.store(Arc::new(snap));
    }

    fn tick_history(&mut self, frames: usize, viz: &MasteringViz) {
        self.samples_since_history += frames;
        if self.samples_since_history < self.history_tick_samples {
            return;
        }
        self.samples_since_history = 0;
        let snap = viz.load_snapshot();
        {
            let mut h = viz.lufs_history.lock();
            h.push(snap.momentary_lufs);
        }
        {
            let mut h = viz.tp_history.lock();
            h.push(snap.true_peak_max_dbtp);
        }
    }
}
