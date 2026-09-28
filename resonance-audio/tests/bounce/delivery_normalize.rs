//! Delivery normalization (warmth-width-depth.md §7.7, W11): a project
//! exported through the real offline path (`run_export`) with the
//! normalize stage `render.mixdown`'s `normalize` / `platform` maps to
//! lands on its platform target. The written file is re-measured
//! independently here, not trusted from the export's own report.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use resonance_audio::test_support::{export_for_test, AutomationSnapshot, SharedState};
use resonance_audio::types::*;
use resonance_metering::{LufsMeter, TruePeakMeter};

const SR: u32 = 48_000;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-delivery-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("master.wav")
}

/// Eight seconds of a program-like stereo signal at `gain`: a bass line,
/// a mid tone and filtered noise, so crest and spectrum are mix-like and
/// the limiter has real peaks to catch.
fn program(gain: f32) -> Vec<f32> {
    let mut state = 0x0DE1_17E5u32;
    let mut noise = || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 8_388_608.0 - 1.0
    };
    let (mut lp_l, mut lp_r) = (0.0f32, 0.0f32);
    let tau = std::f64::consts::TAU;
    (0..SR as usize * 8)
        .flat_map(|n| {
            let t = n as f64 / f64::from(SR);
            let bass = 0.35 * (tau * 55.0 * t).sin() as f32;
            let mid = 0.15 * (tau * 440.0 * t).sin() as f32;
            lp_l = 0.9 * lp_l + 0.1 * noise();
            lp_r = 0.9 * lp_r + 0.1 * noise();
            [gain * (bass + mid + 0.6 * lp_l), gain * (bass + mid + 0.6 * lp_r)]
        })
        .collect()
}

fn project(pcm: Vec<f32>) -> (Arc<SharedState>, Arc<arc_swap::ArcSwap<TempoMap>>) {
    let shared = Arc::new(SharedState::default());
    shared.edit_tracks(|m| {
        m.insert(1, Arc::new(Track::with_type(1, "mix".into(), TrackType::Audio)));
    });
    shared.edit_clips(|c| {
        c.push(Arc::new(AudioClip {
            id: 1,
            track_id: 1,
            start_sample: 0,
            source: ClipSource::memory(pcm),
            name: "program".into(),
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
    (shared, Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())))
}

/// Integrated LUFS and true peak of a written WAV, measured here.
fn measure_file(path: &PathBuf) -> (f32, f32) {
    let mut reader = hound::WavReader::open(path).expect("the export wrote a WAV");
    let samples: Vec<f32> = reader.samples::<f32>().map(|s| s.unwrap()).collect();
    let (left, right): (Vec<f32>, Vec<f32>) =
        samples.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
    let mut lufs = LufsMeter::new(SR as f32);
    lufs.push_stereo(&left, &right);
    let mut tp = TruePeakMeter::new();
    tp.push_stereo(&left, &right);
    (lufs.integrated_lufs(), tp.peak_dbtp())
}

fn export(gain: f32, target: f32, ceiling: f32, tag: &str) -> (f32, f32, Option<f32>) {
    let (shared, tempo) = project(program(gain));
    let settings = ExportSettings {
        normalize: NormalizeSpec {
            enabled: true,
            mode: NormalizeMode::IntegratedLufs,
            target_db: target,
            ceiling_dbtp: ceiling,
        },
        ..ExportSettings::default_wav()
    };
    let path = tmp(tag);
    let events = export_for_test(
        path.to_string_lossy().into_owned(),
        &settings,
        &AtomicBool::new(false),
        &shared,
        &tempo,
        &AutomationSnapshot::default(),
        SR,
    );
    let reported = events.iter().find_map(|e| match e {
        AudioEvent::ExportComplete { achieved_lufs, .. } => Some(*achieved_lufs),
        _ => None,
    });
    let reported = reported.expect("the export completed");
    let (lufs, dbtp) = measure_file(&path);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
    (lufs, dbtp, reported)
}

#[test]
fn a_quiet_mix_normalizes_to_minus_14_under_minus_1_dbtp() {
    // A mix around -26 LUFS: +12 dB of gain, peaks the limiter must catch.
    let (lufs, dbtp, reported) = export(0.25, -14.0, -1.0, "quiet");
    assert!((lufs - -14.0).abs() <= 0.2, "file measures {lufs} LUFS, want -14 +- 0.2");
    assert!(dbtp <= -1.0 + 0.05, "file true peak {dbtp} dBTP, ceiling -1");
    let reported = reported.expect("normalization reports the achieved loudness");
    assert!((reported - lufs).abs() < 0.1, "reported {reported} vs measured {lufs}");
}

#[test]
fn a_hot_mix_is_turned_down_to_the_target() {
    let (lufs, dbtp, _) = export(1.2, -14.0, -1.0, "hot");
    assert!((lufs - -14.0).abs() <= 0.2, "file measures {lufs} LUFS");
    assert!(dbtp <= -0.95, "{dbtp}");
}

#[test]
fn every_platform_lands_on_its_own_target() {
    let platforms = [("apple", -16.0, -1.0), ("amazon", -14.0, -2.0), ("deezer", -15.0, -1.0)];
    for (tag, target, ceiling) in platforms {
        let (lufs, dbtp, _) = export(0.25, target, ceiling, tag);
        assert!((lufs - target).abs() <= 0.2, "{tag}: {lufs} LUFS, want {target}");
        assert!(dbtp <= ceiling + 0.05, "{tag}: {dbtp} dBTP, ceiling {ceiling}");
    }
}
