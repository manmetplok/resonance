//! Slice A0 (automation-control-api.md §2.1): `meter.measure` and
//! `meter.stems` must honour parameter automation, the way
//! `render.mixdown` already does.
//!
//! Before this fix `render_stem` always rendered with an empty
//! `AutomationSnapshot::default()` (`engine/bounce/stem.rs`), so a
//! measurement or stem export of a project that actually uses automation
//! silently reported the UNAUTOMATED mix. `measure.rs` and
//! `stem_export.rs` now thread the caller's real snapshot through, the
//! same way `thread/dispatch/bounce.rs` passes `ctx.automation.load_full()`
//! into `measure_mix_spawn` / `export_stems_spawn`.
//!
//! A master-gain lane ramps a sustained sine source from the floor
//! (-60 dB, which the engine renders as EXACT silence) to 0 dB across a
//! plugin-free master mix. `measure_mix` (the engine side of
//! `meter.measure`) and `export_stems` (the engine side of
//! `meter.stems`) are each driven directly, with the real automation
//! snapshot, over an early window (near the floor) and a late window
//! (near unity gain). The late window must read clearly louder than the
//! early one — and, per [feedback_silent_goldens_are_vacuous], the late
//! window is independently asserted to be far from silence, so a
//! regression that made BOTH windows read the same silent level could
//! not pass by accident.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};

use resonance_audio::test_support::{
    export_stems, measure_mix, AutomationSnapshot, MeasureSource, MixMeasurement, SharedState,
    StemBitDepth, StemSource, StemTarget,
};
use resonance_audio::types::*;
use resonance_common::{
    real_to_lane_value, AutomationLane, AutomationTarget, Breakpoint, CurveKind,
};

const SR: u32 = 48_000;
/// 4 s of sustained sine, long enough for a clean early/late split with a
/// whole second of headroom on each side.
const TOTAL_FRAMES: u64 = SR as u64 * 4;
/// One-second windows at each end of the ramp.
const WINDOW: u64 = SR as u64;
/// Correlation token the harness sends back on every measurement.
const MEASURE_ID: u64 = 9_001;

struct EngineState {
    shared: Arc<SharedState>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    tx: Sender<AudioEvent>,
    rx: Receiver<AudioEvent>,
}

impl EngineState {
    fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            shared: Arc::new(SharedState::default()),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            tx,
            rx,
        }
    }

    /// A steady 220 Hz sine on a track routed straight to master, spanning
    /// the whole render — sine (not DC) so both LUFS/peak measurement and
    /// the WAV read-back see real, non-degenerate signal.
    fn add_sine_track(&self) {
        let mut t = Track::new(1, "master sine".into());
        t.set_output(TrackOutput::Master);
        self.shared.edit_tracks(|m| {
            m.insert(1, std::sync::Arc::new(t));
        });

        let mut pcm = vec![0.0f32; TOTAL_FRAMES as usize * 2];
        let step = std::f32::consts::TAU * 220.0 / SR as f32;
        for (f, frame) in pcm.chunks_exact_mut(2).enumerate() {
            let s = 0.5 * (step * f as f32).sin();
            frame[0] = s;
            frame[1] = s;
        }
        self.shared.edit_clips(|c| {
            c.push(Arc::new(AudioClip {
                id: 1,
                track_id: 1,
                start_sample: 0,
                source: ClipSource::memory(pcm),
                name: "sine".into(),
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
            }));
        });
    }

    /// A master-gain lane ramping linearly from the floor (-60 dB, exact
    /// silence) at frame 0 to 0 dB (unity) at `TOTAL_FRAMES`.
    fn master_gain_ramp(&self) -> AutomationSnapshot {
        let unity_db = real_to_lane_value(&AutomationTarget::MasterGain, 0.0);
        let lane = AutomationLane::new(
            1,
            AutomationTarget::MasterGain,
            vec![
                Breakpoint::new(0, 0.0, CurveKind::Linear),
                Breakpoint::new(TOTAL_FRAMES, unity_db, CurveKind::Linear),
            ],
        );
        let mut snap = AutomationSnapshot::default();
        snap.mix_lanes.insert(AutomationTarget::MasterGain, lane);
        snap
    }

    /// Drive the real `measure_mix` engine command over an explicit
    /// range, exactly as `thread/dispatch/bounce.rs` calls it, and return
    /// the one measurement it reports.
    fn measure(&self, range: (u64, u64), automation: &AutomationSnapshot) -> MixMeasurement {
        measure_mix(
            MEASURE_ID,
            vec![StemSource::Master],
            Some(range),
            MeasureSource::Render,
            &self.shared,
            &self.tempo_map,
            automation,
            SR,
            &self.tx,
        );
        let events: Vec<AudioEvent> = self.rx.try_iter().collect();
        assert_eq!(events.len(), 1, "exactly one terminal event: {events:?}");
        match events.into_iter().next().unwrap() {
            AudioEvent::MixMeasured { results, .. } => {
                assert_eq!(results.len(), 1, "one target, one result");
                results.into_iter().next().unwrap()
            }
            other => panic!("expected MixMeasured, got {other:?}"),
        }
    }
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "resonance_automation_render_{}_{name}.wav",
        std::process::id()
    ));
    p
}

fn read_f32_wav(path: &std::path::Path) -> Vec<f32> {
    hound::WavReader::open(path)
        .expect("wav opens")
        .into_samples::<f32>()
        .collect::<Result<Vec<_>, _>>()
        .expect("wav decodes")
}

/// De-interleave a stereo buffer into `(left, right)`.
fn split_stereo(interleaved: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let mut left = Vec::with_capacity(interleaved.len() / 2);
    let mut right = Vec::with_capacity(interleaved.len() / 2);
    for f in interleaved.chunks_exact(2) {
        left.push(f[0]);
        right.push(f[1]);
    }
    (left, right)
}

// ---- meter.measure: `measure_mix` plays the ramp -------------------------

#[test]
fn measure_mix_honours_master_gain_automation_across_the_ramp() {
    let state = EngineState::new();
    state.add_sine_track();
    let automation = state.master_gain_ramp();

    let early = state.measure((0, WINDOW), &automation);
    let late = state.measure((TOTAL_FRAMES - WINDOW, TOTAL_FRAMES), &automation);

    assert!(
        late.sample_peak_db > early.sample_peak_db + 20.0,
        "the ramp must read clearly louder late than early: early {} dB, late {} dB",
        early.sample_peak_db,
        late.sample_peak_db,
    );
    // Guard against a vacuous pass (silent goldens are vacuous): the late
    // window sits at unity master gain, so it must be near full scale,
    // never anywhere close to the floor.
    assert!(
        late.sample_peak_db > -20.0,
        "late window must be near full scale (unity master gain), not silent: {} dB",
        late.sample_peak_db,
    );

    // Sanity check tying the rise to automation specifically: with the
    // default (empty) snapshot — the pre-fix behaviour every `meter.*`
    // path used — the unattenuated sine peaks the same in both windows,
    // so the "rising level" this test checks for would vanish. This is
    // exactly the regression this test exists to catch.
    let flat_early = state.measure((0, WINDOW), &AutomationSnapshot::default());
    let flat_late = state.measure((TOTAL_FRAMES - WINDOW, TOTAL_FRAMES), &AutomationSnapshot::default());
    assert!(
        (flat_late.sample_peak_db - flat_early.sample_peak_db).abs() < 0.5,
        "sanity: an unautomated render of the same source is flat across \
         the two windows (early {} dB, late {} dB) — the rise above comes \
         from the automation lane, not from the source or the windows",
        flat_early.sample_peak_db,
        flat_late.sample_peak_db,
    );
}

// ---- meter.stems: `export_stems` plays the ramp too -----------------------

#[test]
fn export_stems_honours_master_gain_automation_across_the_ramp() {
    let state = EngineState::new();
    state.add_sine_track();
    let automation = state.master_gain_ramp();

    let early_path = tmp_path("early");
    let late_path = tmp_path("late");
    let _ = std::fs::remove_file(&early_path);
    let _ = std::fs::remove_file(&late_path);

    let (tx, rx) = crossbeam_channel::unbounded::<AudioEvent>();
    export_stems(
        vec![StemTarget {
            source: StemSource::Master,
            path: early_path.to_str().unwrap().to_string(),
        }],
        Some((0, WINDOW)),
        SR,
        StemBitDepth::Float32,
        false,
        &state.shared,
        &AtomicBool::new(false),
        &state.tempo_map,
        &automation,
        SR,
        &tx,
    );
    let early_events: Vec<AudioEvent> = rx.try_iter().collect();
    assert!(
        matches!(early_events.last(), Some(AudioEvent::StemExportComplete { .. })),
        "early stem export must complete: {early_events:?}"
    );

    export_stems(
        vec![StemTarget {
            source: StemSource::Master,
            path: late_path.to_str().unwrap().to_string(),
        }],
        Some((TOTAL_FRAMES - WINDOW, TOTAL_FRAMES)),
        SR,
        StemBitDepth::Float32,
        false,
        &state.shared,
        &AtomicBool::new(false),
        &state.tempo_map,
        &automation,
        SR,
        &tx,
    );
    let late_events: Vec<AudioEvent> = rx.try_iter().collect();
    assert!(
        matches!(late_events.last(), Some(AudioEvent::StemExportComplete { .. })),
        "late stem export must complete: {late_events:?}"
    );

    let (early_l, early_r) = split_stereo(&read_f32_wav(&early_path));
    let (late_l, late_r) = split_stereo(&read_f32_wav(&late_path));
    let early_peak = resonance_metering::offline::sample_peak_db(&early_l, &early_r);
    let late_peak = resonance_metering::offline::sample_peak_db(&late_l, &late_r);

    assert!(
        late_peak > early_peak + 20.0,
        "the exported stems must play the ramp too: early {early_peak} dB, late {late_peak} dB"
    );
    assert!(
        late_peak > -20.0,
        "late stem must be near full scale, not silent: {late_peak} dB"
    );

    let _ = std::fs::remove_file(&early_path);
    let _ = std::fs::remove_file(&late_path);
}
