//! A reference track measures like a pool clip (warmth-width-depth.md
//! §7.5, W10 exit criterion).
//!
//! The same engine-format WAV (what `pool.import` writes into the
//! project's `audio/` folder) is measured three ways:
//!
//! 1. placed as a clip on a track and measured as that track's stem —
//!    the `meter.measure {track_id}` path;
//! 2. read the way `AudioCommand::MeasureAudio` reads a file
//!    (`measure_audio_file`: the clip loader's `open_wav_at_rate`) — the
//!    `master.assist` reference path;
//! 3. decoded the way the A/B reference player decodes a reference
//!    (`decode_file`) and measured as decoded audio (`measure_decoded`) —
//!    the `meter.measure {reference}` path.
//!
//! Every figure and detail block must agree, to within float noise.

use std::sync::Arc;

use resonance_audio::test_support::{
    measure_audio_file, measure_decoded, measure_mix_detailed, AutomationSnapshot,
    MeasureSource, MixMeasurement, SharedState, StemSource,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
/// Long enough for the 3 s short-term window and many LTAS frames.
const FRAMES: usize = SR as usize * 5;
const TRACK: TrackId = 1;
/// 20 ms of fade at each end of the programme; see [`programme`].
const EDGE_FADE: usize = SR as usize / 50;

fn detail() -> DetailSet {
    DetailSet {
        spectrum: true,
        stereo: true,
        dynamics: true,
        depth: false,
        decay: false,
        assist: true,
    }
}

/// A partly decorrelated stereo programme: two tones, a panned tone and
/// noise, so every detail block has something to say.
///
/// It fades in and out over [`EDGE_FADE`] frames, as a real recording
/// starts and ends near silence. A placed clip gets a 96-frame declick at
/// each edge (`CLIP_DECLICK_FRAMES`), which the file measured on its own
/// does not; on a hard-edged file that ramp alone moves the short-window
/// figures (momentary / short-term maxima, LRA on a 5 s range) by up to a
/// few hundredths of a dB, and it is the ONLY difference between the two.
fn programme() -> Vec<f32> {
    let mut state = 0x9e37_79b9_u32;
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        (state as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    let tau = std::f32::consts::TAU;
    let mut pcm = Vec::with_capacity(FRAMES * 2);
    for n in 0..FRAMES {
        let t = n as f32 / SR as f32;
        let env = 0.6 + 0.4 * (tau * 1.5 * t).sin();
        let body = 0.3 * env * (tau * 82.0 * t).sin() + 0.1 * (tau * 1_760.0 * t).sin();
        let edge = n.min(FRAMES - 1 - n).min(EDGE_FADE) as f32 / EDGE_FADE as f32;
        let fade = 0.5 - 0.5 * (std::f32::consts::PI * edge).cos();
        let l = fade * (body + 0.08 * (tau * 440.0 * t).sin() + 0.05 * noise());
        let r = fade * (0.9 * body + 0.05 * noise());
        pcm.push(l);
        pcm.push(r);
    }
    pcm
}

fn write_pooled_wav(path: &std::path::Path, pcm: &[f32]) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: SR,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut w = hound::WavWriter::create(path, spec).unwrap();
    for &s in pcm {
        w.write_sample(s).unwrap();
    }
    w.finalize().unwrap();
}

fn clip_measurement(path: &std::path::Path) -> MixMeasurement {
    let shared = Arc::new(SharedState::default());
    let tempo_map = Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default()));
    shared.edit_tracks(|m| {
        m.insert(TRACK, Arc::new(Track::new(TRACK, "reference clip".into())));
    });
    let source = ClipSource::open_wav_at_rate(path, SR).expect("the pooled file opens as a clip");
    shared.edit_clips(|c| {
        c.push(Arc::new(AudioClip {
            id: 1,
            track_id: TRACK,
            start_sample: 0,
            source,
            name: "reference".into(),
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
    let (tx, rx) = crossbeam_channel::unbounded();
    measure_mix_detailed(
        1,
        vec![StemSource::Track(TRACK)],
        Some((0, FRAMES as u64)),
        MeasureSource::Render,
        detail(),
        &shared,
        &tempo_map,
        &AutomationSnapshot::default(),
        SR,
        &tx,
    );
    match rx.try_recv().expect("one terminal event") {
        AudioEvent::MixMeasured { mut results, .. } => results.remove(0),
        other => panic!("expected MixMeasured, got {other:?}"),
    }
}

fn close(what: &str, a: f32, b: f32, tol: f32) {
    let same_inf = a.is_infinite() && b.is_infinite() && a.signum() == b.signum();
    assert!(same_inf || (a - b).abs() <= tol, "{what}: {a} vs {b}");
}

fn close_opt(what: &str, a: Option<f32>, b: Option<f32>, tol: f32) {
    match (a, b) {
        (Some(a), Some(b)) => close(what, a, b, tol),
        (None, None) => {}
        _ => panic!("{what}: {a:?} vs {b:?}"),
    }
}

/// Every figure of two measurements of the same audio, compared.
fn assert_same(label: &str, a: &MixMeasurement, b: &MixMeasurement) {
    const TOL: f32 = 1e-3;
    assert_eq!(a.frames, b.frames, "{label}: frames");
    close(&format!("{label} lufs_integrated"), a.lufs_integrated, b.lufs_integrated, TOL);
    close(&format!("{label} lufs_short_max"), a.lufs_short_term_max, b.lufs_short_term_max, TOL);
    close(&format!("{label} lufs_momentary_max"), a.lufs_momentary_max, b.lufs_momentary_max, TOL);
    close(&format!("{label} lra"), a.lra_lu, b.lra_lu, TOL);
    close(&format!("{label} true_peak"), a.true_peak_dbtp, b.true_peak_dbtp, TOL);
    close(&format!("{label} sample_peak"), a.sample_peak_db, b.sample_peak_db, TOL);
    close(&format!("{label} crest"), a.crest_db, b.crest_db, TOL);
    close(&format!("{label} correlation"), a.correlation, b.correlation, 1e-5);
    close(&format!("{label} mono_penalty"), a.mono_penalty_db, b.mono_penalty_db, TOL);
    assert_eq!(a.clipped_samples, b.clipped_samples, "{label}: clipped");
    close(&format!("{label} bands.low"), a.bands.low, b.bands.low, 1e-5);
    close(&format!("{label} bands.air"), a.bands.air, b.bands.air, 1e-5);

    let (sa, sb) = (
        a.detail.spectrum.as_ref().expect("spectrum"),
        b.detail.spectrum.as_ref().expect("spectrum"),
    );
    for (i, (x, y)) in sa.third_octave.iter().zip(&sb.third_octave).enumerate() {
        close(&format!("{label} third_octave[{i}]"), *x, *y, TOL);
    }
    close_opt(&format!("{label} tilt"), sa.tilt_db_per_oct, sb.tilt_db_per_oct, 1e-4);
    close_opt(&format!("{label} centroid"), sa.centroid_hz, sb.centroid_hz, 0.05);
    close_opt(
        &format!("{label} lowmid_presence"),
        sa.lowmid_presence_db,
        sb.lowmid_presence_db,
        TOL,
    );

    let (ta, tb) = (
        a.detail.stereo.as_ref().expect("stereo"),
        b.detail.stereo.as_ref().expect("stereo"),
    );
    for (i, (x, y)) in ta.bands.iter().zip(&tb.bands).enumerate() {
        close_opt(&format!("{label} stereo[{i}].correlation"), x.correlation, y.correlation, 1e-4);
        close_opt(&format!("{label} stereo[{i}].side_mid"), x.side_mid_db, y.side_mid_db, TOL);
        close_opt(&format!("{label} stereo[{i}].mono_loss"), x.mono_loss_db, y.mono_loss_db, TOL);
    }
    close_opt(&format!("{label} balance"), ta.balance_db, tb.balance_db, TOL);
    assert_eq!(ta.one_sided, tb.one_sided, "{label}: one_sided");

    let (da, db) = (a.detail.dynamics.unwrap(), b.detail.dynamics.unwrap());
    close_opt(&format!("{label} plr"), da.plr_db, db.plr_db, TOL);
    close_opt(&format!("{label} psr"), da.psr_db, db.psr_db, TOL);

    let (la, lb) = (
        a.detail.assist_ltas.as_ref().expect("assist LTAS"),
        b.detail.assist_ltas.as_ref().expect("assist LTAS"),
    );
    assert_eq!(la.len(), lb.len());
    for (i, (x, y)) in la.iter().zip(lb).enumerate() {
        close(&format!("{label} assist_ltas[{i}]"), *x, *y, TOL);
    }
}

#[test]
fn a_reference_measures_like_the_same_file_placed_as_a_clip() {
    let dir = std::env::temp_dir().join(format!("measure-reference-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("asset_1.wav");
    let pcm = programme();
    write_pooled_wav(&path, &pcm);

    let clip = clip_measurement(&path);
    assert!(clip.lufs_integrated.is_finite() && clip.lufs_integrated > -40.0, "not silent");

    let file = measure_audio_file(&path, SR, detail()).expect("the file measures");
    assert_eq!(file.source, MeasureSource::Decoded);
    assert_same("file vs clip", &file, &clip);

    let (decoded, _) = resonance_common::decode_file(path.to_str().unwrap(), SR)
        .expect("the reference player's decoder reads it");
    let reference = measure_decoded(&decoded, SR, detail());
    assert_eq!(reference.source, MeasureSource::Decoded);
    assert_same("reference vs clip", &reference, &clip);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_missing_file_is_an_error_not_a_silent_measurement() {
    let err = measure_audio_file(std::path::Path::new("/nonexistent/asset_404.wav"), SR, detail())
        .expect_err("a missing file cannot measure");
    assert!(err.to_string().contains("asset_404"), "{err}");
}
