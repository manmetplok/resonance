//! The live mix meter starts a fresh run on transport start and on a seek
//! (code review RT-03). Its integrated reading used to accumulate across
//! every play run and every jump since app start, so the value it showed
//! described no stretch of the song in particular.

use std::sync::atomic::Ordering;

use resonance_audio::test_support::MixAudioHarness;
use resonance_audio::types::*;

const SR: u32 = 48_000;
const BLOCK: usize = 128;
/// Blocks in a second of audio, comfortably past the first 400 ms gating
/// block.
const SECOND: usize = SR as usize / BLOCK;

fn harness() -> MixAudioHarness {
    let mut track = Track::new(1, "dc".into());
    track.set_output(TrackOutput::Master);
    let frames = 30 * SR as usize;
    let samples: Vec<f32> = (0..frames)
        .flat_map(|i| {
            let s = 0.3 * (std::f32::consts::TAU * 997.0 * i as f32 / SR as f32).sin();
            [s, s]
        })
        .collect();
    let clip = AudioClip {
        id: 1,
        track_id: 1,
        start_sample: 0,
        source: ClipSource::memory(samples),
        name: "tone".into(),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::Linear,
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::Linear,
        gain_db: 0.0,
        vocal_tuning: None,
        warp_enabled: false,
        original_bpm: None,
        transpose_semitones: 0.0,
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    };
    let h = MixAudioHarness::new(
        vec![track],
        Vec::new(),
        vec![clip],
        Vec::new(),
        Vec::new(),
        TempoMap::default(),
        BLOCK,
        2,
        SR,
        true,
    );
    h.shared()
        .master_volume_bits
        .store(1.0f32.to_bits(), Ordering::Relaxed);
    h.shared().playing.store(true, Ordering::Relaxed);
    h
}

fn integrated(h: &MixAudioHarness) -> f32 {
    h.shared().mix_meter.load().integrated_lufs
}

#[test]
fn a_seek_starts_a_fresh_integrated_reading() {
    let mut h = harness();
    for _ in 0..SECOND {
        h.render();
    }
    assert!(integrated(&h).is_finite(), "a second of tone reads");

    h.shared().playhead.store(10 * SR as u64, Ordering::Release);
    h.render();
    assert_eq!(
        integrated(&h),
        f32::NEG_INFINITY,
        "the block after a seek has no gating block of the new run yet"
    );
    for _ in 0..SECOND {
        h.render();
    }
    assert!(integrated(&h).is_finite());
}

#[test]
fn continuous_playback_keeps_accumulating() {
    let mut h = harness();
    for _ in 0..SECOND {
        h.render();
    }
    for _ in 0..SECOND {
        h.render();
        assert!(integrated(&h).is_finite(), "no reset while the transport rolls on");
    }
}

#[test]
fn transport_start_starts_a_fresh_integrated_reading() {
    let mut h = harness();
    for _ in 0..SECOND {
        h.render();
    }
    assert!(integrated(&h).is_finite());

    h.shared().playing.store(false, Ordering::Relaxed);
    h.render();
    h.shared().playing.store(true, Ordering::Relaxed);
    h.render();
    assert_eq!(integrated(&h), f32::NEG_INFINITY, "a new play run reads afresh");
}
