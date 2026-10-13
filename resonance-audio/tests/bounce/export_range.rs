//! A ranged export (`AudioCommand::ExportAudio { range: Some(..) }`, the
//! engine half of a ranged `render.mixdown`): the file holds exactly the
//! window plus the shared FX tail, and its sample 0 is the window's
//! start — not the earliest clip's. Driven through the real renderer
//! (`export_for_test` → `run_export`).

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use resonance_audio::test_support::{export_for_test, AutomationSnapshot, SharedState};
use resonance_audio::types::*;

const SR: u32 = 48_000;
/// One 4/4 bar at the default 120 BPM.
const BAR: u64 = SR as u64 * 2;
/// The shared offline FX tail every export renders past its end
/// (`bounce_common::BOUNCE_TAIL_SECONDS`, 2 s).
const TAIL: u64 = SR as u64 * 2;

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-export-range-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("range.wav")
}

/// Sample `n` of the test signal: a slow staircase, so every position in
/// the clip has a value that says where it came from.
fn signal(n: u64) -> f32 {
    ((n / 480) % 100) as f32 / 400.0 + 0.05
}

/// One audio track carrying a four-bar clip that starts at bar 2 (sample
/// `BAR`), so the earliest clip is not at sample 0.
fn project() -> (Arc<SharedState>, Arc<arc_swap::ArcSwap<TempoMap>>) {
    let frames = 4 * BAR;
    let pcm: Vec<f32> = (0..frames).flat_map(|n| [signal(n), signal(n)]).collect();
    let shared = Arc::new(SharedState::default());
    shared.edit_tracks(|m| {
        m.insert(1, Arc::new(Track::with_type(1, "stairs".into(), TrackType::Audio)));
    });
    shared.edit_clips(|c| {
        c.push(Arc::new(AudioClip {
            id: 1,
            track_id: 1,
            start_sample: BAR,
            source: ClipSource::memory(pcm),
            name: "stairs".into(),
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

fn export(range: Option<(SamplePos, SamplePos)>, tag: &str) -> (Vec<f32>, Vec<AudioEvent>) {
    let (shared, tempo) = project();
    let path = tmp(tag);
    let events = export_for_test(
        path.to_string_lossy().into_owned(),
        &ExportSettings::default_wav(),
        range,
        &AtomicBool::new(false),
        &shared,
        &tempo,
        &AutomationSnapshot::default(),
        SR,
    );
    let left = match hound::WavReader::open(&path) {
        Ok(mut reader) => {
            let samples: Vec<f32> = reader.samples::<f32>().map(|s| s.unwrap()).collect();
            samples.chunks_exact(2).map(|f| f[0]).collect()
        }
        Err(_) => Vec::new(),
    };
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
    (left, events)
}

#[test]
fn a_two_bar_range_renders_two_bars_plus_the_tail_from_its_start() {
    // Bars 3-4 of the song = clip frames BAR..3*BAR.
    let start = 2 * BAR;
    let (left, events) = export(Some((start, start + 2 * BAR)), "two-bars");
    assert!(
        events.iter().any(|e| matches!(e, AudioEvent::ExportComplete { .. })),
        "{events:?}"
    );
    assert_eq!(left.len() as u64, 2 * BAR + TAIL, "two bars plus the tail");

    // Sample 0 is the range start: the file follows the clip from its
    // frame `start - BAR`, through the whole window.
    let clip_offset = start - BAR;
    let gain = left[1_000] / signal(clip_offset + 1_000);
    assert!(gain > 0.1, "the window is not silent (gain {gain})");
    for i in (0..2 * BAR).step_by(997) {
        let want = gain * signal(clip_offset + i);
        assert!(
            (left[i as usize] - want).abs() < 1e-4,
            "frame {i}: {} vs {want}",
            left[i as usize]
        );
    }
    // The clip ends at 5*BAR, after the window: the tail is the clip
    // played on (not a decay), cut where the tail ends.
    let i = 2 * BAR + TAIL / 2;
    let want = gain * signal(clip_offset + i);
    assert!((left[i as usize] - want).abs() < 1e-4);
}

#[test]
fn a_range_before_the_first_clip_renders_its_silence() {
    // Bar 1 holds nothing; a ranged export still renders it as asked.
    let (left, events) = export(Some((0, BAR)), "silence");
    assert!(events.iter().any(|e| matches!(e, AudioEvent::ExportComplete { .. })));
    assert_eq!(left.len() as u64, BAR + TAIL);
    assert!(left[..BAR as usize].iter().all(|s| s.abs() < 1e-6));
    // The clip starts at BAR, inside the tail.
    assert!(left[BAR as usize + 1_000].abs() > 1e-3);
}

#[test]
fn no_range_still_starts_at_the_earliest_clip() {
    let (left, _) = export(None, "whole");
    assert_eq!(left.len() as u64, 4 * BAR + TAIL);
    assert!(left[1_000].abs() > 1e-3, "sample 0 is the clip start, not bar 1");
}

#[test]
fn an_empty_range_is_refused() {
    let (left, events) = export(Some((BAR, BAR)), "empty");
    assert!(left.is_empty());
    assert!(
        events.iter().any(|e| matches!(
            e,
            AudioEvent::ExportError {
                kind: ExportErrorKind::NoAudio,
                ..
            }
        )),
        "{events:?}"
    );
}
