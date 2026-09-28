//! Opt-in measurement detail through the real offline measure path
//! (warmth-width-depth.md §7.1): `measure_mix_detailed` renders each
//! target with `render_stem` and attaches exactly the detail blocks its
//! `DetailSet` asked for, without disturbing any default figure.
//!
//! The math itself is pinned against analytic answers in
//! `resonance-metering/tests/detail_*.rs`; this file pins the wiring:
//! the right block on the right target, from the rendered audio.

use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};

use resonance_audio::test_support::{
    measure_mix_detailed, AutomationSnapshot, MeasureSource, MixMeasurement, SharedState,
    StemSource,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
const FRAMES: usize = (SR as usize) * 6;

struct Project {
    shared: Arc<SharedState>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    tx: Sender<AudioEvent>,
    rx: Receiver<AudioEvent>,
}

impl Project {
    fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            shared: Arc::new(SharedState::default()),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            tx,
            rx,
        }
    }

    fn add_track(&self, id: TrackId) {
        let mut t = Track::new(id, format!("track {id}"));
        t.set_output(TrackOutput::Master);
        self.shared.edit_tracks(|m| {
            m.insert(id, Arc::new(t));
        });
    }

    /// A clip of interleaved stereo PCM on `track` at frame 0.
    fn add_clip(&self, id: ClipId, track: TrackId, pcm: Vec<f32>) {
        self.shared.edit_clips(|c| {
            c.push(Arc::new(AudioClip {
                id,
                track_id: track,
                start_sample: 0,
                source: ClipSource::memory(pcm),
                name: "signal".into(),
                trim_start_frames: 0,
                trim_end_frames: 0,
                fade_in_frames: 0,
                fade_in_curve: FadeCurve::default(),
                fade_out_frames: 0,
                fade_out_curve: FadeCurve::default(),
                gain_db: 0.0,
                vocal_tuning: None,
                warp_enabled: false,
                original_bpm: None,
                transpose_semitones: 0.0,
                warp_algorithm: WarpAlgorithm::default(),
                warp_markers: Vec::new(),
                tuning_render_cache: None,
            }))
        });
    }

    fn measure(&self, targets: Vec<StemSource>, detail: DetailSet) -> Vec<MixMeasurement> {
        measure_mix_detailed(
            7,
            targets,
            None,
            MeasureSource::Render,
            detail,
            &self.shared,
            &self.tempo_map,
            &AutomationSnapshot::default(),
            SR,
            &self.tx,
        );
        let events: Vec<AudioEvent> = self.rx.try_iter().collect();
        assert_eq!(events.len(), 1, "exactly one terminal event: {events:?}");
        match &events[0] {
            AudioEvent::MixMeasured { results, .. } => results.clone(),
            other => panic!("expected MixMeasured, got {other:?}"),
        }
    }
}

/// Paul Kellet's pink filter over a deterministic LCG: pink to well
/// within the ±0.3 dB/oct this wiring test asks for.
fn pink_stereo(amplitude: f32) -> Vec<f32> {
    let mut state = 0x5EED_2026u32;
    let (mut b0, mut b1, mut b2) = (0.0f32, 0.0f32, 0.0f32);
    let mut pcm = Vec::with_capacity(FRAMES * 2);
    for _ in 0..FRAMES {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let white = (state >> 8) as f32 / 8_388_608.0 - 1.0;
        b0 = 0.99765 * b0 + white * 0.099_046;
        b1 = 0.96300 * b1 + white * 0.296_516_4;
        b2 = 0.57000 * b2 + white * 1.052_691_3;
        let s = (b0 + b1 + b2 + white * 0.1848) * 0.2 * amplitude;
        pcm.push(s);
        pcm.push(s);
    }
    pcm
}

fn sine_stereo(freq: f32, amplitude: f32) -> Vec<f32> {
    let step = std::f32::consts::TAU * freq / SR as f32;
    (0..FRAMES)
        .flat_map(|f| {
            let s = amplitude * (step * f as f32).sin();
            [s, s]
        })
        .collect()
}

/// Deterministic white noise, mono.
fn white(seed: u32) -> Vec<f32> {
    let mut state = seed | 1;
    (0..FRAMES)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 8) as f32 / 8_388_608.0 - 1.0) * 0.25
        })
        .collect()
}

/// Interleave two mono channels.
fn stereo(left: &[f32], right: &[f32]) -> Vec<f32> {
    left.iter().zip(right).flat_map(|(&l, &r)| [l, r]).collect()
}

fn stereo_only() -> DetailSet {
    DetailSet {
        stereo: true,
        ..DetailSet::default()
    }
}

/// Measure one clip's worth of stereo PCM on the master, with `detail`.
fn master_of(pcm: Vec<f32>, detail: DetailSet) -> MixMeasurement {
    let p = Project::new();
    p.add_track(1);
    p.add_clip(1, 1, pcm);
    p.measure(vec![StemSource::Master], detail).remove(0)
}

fn spectrum_only() -> DetailSet {
    DetailSet {
        spectrum: true,
        ..DetailSet::default()
    }
}

/// Everything but the detail, for "the default figures did not move".
fn without_detail(m: &MixMeasurement) -> MixMeasurement {
    MixMeasurement {
        detail: MeasurementDetail::default(),
        ..m.clone()
    }
}

#[test]
fn a_plain_measurement_carries_no_detail() {
    let p = Project::new();
    p.add_track(1);
    p.add_clip(1, 1, pink_stereo(0.5));
    let results = p.measure(vec![StemSource::Master], DetailSet::default());
    assert_eq!(results[0].detail, MeasurementDetail::default());
}

#[test]
fn spectrum_detail_reads_the_rendered_pink_noise() {
    let p = Project::new();
    p.add_track(1);
    p.add_clip(1, 1, pink_stereo(0.5));
    let results = p.measure(vec![StemSource::Master], spectrum_only());
    let spectrum = results[0].detail.spectrum.as_ref().expect("spectrum was asked for");
    let tilt = spectrum.tilt_db_per_oct.expect("pink noise has a tilt");
    assert!((tilt - -3.0).abs() < 0.3, "rendered pink tilts {tilt} dB/oct");
    assert_eq!(spectrum.third_octave.len(), 31);
    assert!(spectrum.peaks.is_empty(), "{:?}", spectrum.peaks);
}

#[test]
fn asking_for_detail_leaves_every_default_figure_unchanged() {
    let p = Project::new();
    p.add_track(1);
    p.add_clip(1, 1, pink_stereo(0.5));
    let plain = p.measure(vec![StemSource::Master], DetailSet::default());
    let detailed = p.measure(vec![StemSource::Master], spectrum_only());
    assert_eq!(without_detail(&detailed[0]), plain[0]);
}

#[test]
fn every_target_gets_its_own_spectrum() {
    let p = Project::new();
    p.add_track(1);
    p.add_track(2);
    p.add_clip(1, 1, sine_stereo(1_000.0, 0.25));
    p.add_clip(2, 2, sine_stereo(100.0, 0.25));
    let results = p.measure(
        vec![StemSource::Master, StemSource::Track(1), StemSource::Track(2)],
        spectrum_only(),
    );
    let loudest_band = |m: &MixMeasurement| {
        let s = m.detail.spectrum.as_ref().expect("every target carries it");
        s.third_octave
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i)
            .unwrap()
    };
    // Band 17 is 1 kHz, band 7 is 100 Hz.
    assert_eq!(loudest_band(&results[1]), 17, "track 1 is its 1 kHz sine");
    assert_eq!(loudest_band(&results[2]), 7, "track 2 is its 100 Hz sine");
    let master = results[0].detail.spectrum.as_ref().unwrap();
    assert!(master.third_octave[17] > -40.0 && master.third_octave[7] > -40.0);
}

#[test]
fn rendered_mono_is_fully_correlated() {
    let x = white(1);
    let m = master_of(stereo(&x, &x), stereo_only());
    let s = m.detail.stereo.as_ref().expect("stereo was asked for");
    assert!(!s.one_sided);
    for b in &s.bands {
        assert!(b.correlation.unwrap() > 0.999, "{b:?}");
    }
    assert_eq!(s.haas_lag_ms, None);
    assert_eq!(m.detail.spectrum, None, "only the asked-for block");
}

#[test]
fn rendered_hard_pan_is_one_sided() {
    let x = white(2);
    let silent = vec![0.0f32; FRAMES];
    let m = master_of(stereo(&x, &silent), stereo_only());
    let s = m.detail.stereo.as_ref().unwrap();
    assert!(s.one_sided, "the right channel renders silent");
    assert_eq!(s.balance_db, Some(60.0));
    assert!(s.bands.iter().all(|b| b.correlation.is_none()), "{:?}", s.bands);
}

#[test]
fn rendered_ten_ms_haas_is_found() {
    let x = white(3);
    let late: Vec<f32> = (0..FRAMES)
        .map(|i| if i >= 480 { x[i - 480] } else { 0.0 })
        .collect();
    let m = master_of(stereo(&x, &late), stereo_only());
    let lag = m.detail.stereo.as_ref().unwrap().haas_lag_ms.expect("a 10 ms delay");
    assert!((lag - 10.0).abs() <= 0.1, "lag {lag} ms");
}

#[test]
fn rendered_antiphase_is_minus_one() {
    let x = white(4);
    let inverted: Vec<f32> = x.iter().map(|s| -s).collect();
    let m = master_of(stereo(&x, &inverted), stereo_only());
    let s = m.detail.stereo.as_ref().unwrap();
    for b in &s.bands {
        assert!(b.correlation.unwrap() < -0.999, "{b:?}");
    }
    assert!(s.correlation_windows.unwrap().worst < -0.999);
}

#[test]
fn dynamics_are_the_measurements_own_peak_over_its_loudness() {
    let detail = DetailSet {
        dynamics: true,
        ..DetailSet::default()
    };
    let m = master_of(pink_stereo(0.5), detail);
    let d = m.detail.dynamics.expect("dynamics was asked for");
    assert_eq!(d.plr_db, Some(m.true_peak_dbtp - m.lufs_integrated));
    assert_eq!(d.psr_db, Some(m.true_peak_dbtp - m.lufs_short_term_max));
    assert_eq!((m.detail.spectrum.as_ref(), m.detail.stereo.as_ref()), (None, None));
}
