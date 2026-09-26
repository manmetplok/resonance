//! Regression for FU-D7c: `LoadClipFromWav`'s `submit_clip_load` worker
//! used to skip the `clear_generation` fence that `ImportAudioToPool`'s
//! worker (and a track freeze's) check (code review UPD-09;
//! `HandlerState::clear_generation`'s doc). A load queued just before a
//! `ClearAll` (a project switch, or an undo's slow path) that decoded
//! *after* the clear used to land in the new project anyway: nothing
//! stopped `submit_clip_load`'s worker from pushing into `ctx.clips`, or
//! from echoing `ClipImported`, once the project it was submitted for was
//! already gone. The deleted `ImportClip`'s UPD-09 test covered exactly
//! this shape for the pool-import path; D-7c removed it without porting
//! it here (see `engine/loop_record_takes.rs`'s note on the gap).
//!
//! Mirrors `io/import_audio_to_pool.rs`'s
//! `a_pool_import_outlived_by_its_project_never_lands_after_clear_all`:
//! a long WAV keeps the worker mid-decode while `ClearAll` runs right
//! behind the submit, via the real `handle_load_clip_from_wav` handler
//! through `EngineHandlerHarness`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::transcode_to_wav;
use resonance_audio::types::AudioEvent;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-load-stale-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write an f32-stereo WAV of `seconds` at 48 kHz: long enough that the
/// worker's `ClipSource::open_wav_at_rate` + `compute_waveform_peaks` is
/// still running when `ClearAll` — a handful of `RwLock` operations on
/// the same thread that queued the load — returns right behind it.
fn write_wav(dir: &Path, name: &str, seconds: usize) -> PathBuf {
    let sample_rate: u32 = 48_000;
    let total_frames = sample_rate as usize * seconds;
    let mut samples = Vec::with_capacity(total_frames * 2);
    for i in 0..total_frames {
        let t = i as f32 / sample_rate as f32;
        let s = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * 0.25;
        samples.push(s);
        samples.push(s);
    }
    let path = dir.join(name);
    transcode_to_wav(&path, &samples, sample_rate).expect("write test wav");
    path
}

/// A `LoadClipFromWav` outlived by its project must never land — neither
/// in the engine's clip list nor as a `ClipImported` echo — and a load
/// submitted after the clear (the new project's own) must be unaffected.
#[test]
fn a_clip_load_outlived_by_its_project_never_lands_after_clear_all() {
    let dir = make_tempdir("stale-load");
    // 20 s stereo: long enough to still be decoding when `clear_all`
    // (called right behind the submit, no sleep in between) returns.
    let stale_wav = write_wav(&dir, "stale.wav", 20);
    let current_wav = write_wav(&dir, "current.wav", 1);

    let mut engine = EngineHandlerHarness::new();
    engine.load_clip_from_wav(7, 1, 0, stale_wav.clone(), "stale".into());
    engine.clear_all();
    // Queued after the clear: belongs to the new project and must land.
    engine.load_clip_from_wav(8, 1, 0, current_wav.clone(), "current".into());

    let clips_arc = engine.clips_lock();
    let events = engine.finish_and_drain_events(Duration::from_secs(300));

    let stale_events: Vec<&AudioEvent> = events
        .iter()
        .filter(|e| matches!(e, AudioEvent::ClipImported { clip_id: 7, .. }))
        .collect();
    assert!(
        stale_events.is_empty(),
        "a clip load outlived by its project must never echo ClipImported: {stale_events:?}"
    );

    let ids: Vec<u64> = clips_arc.read().iter().map(|c| c.id).collect();
    assert!(
        !ids.contains(&7),
        "a clip load outlived by its project must never land in the engine's clip list: {ids:?}"
    );

    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::ClipImported { clip_id: 8, .. })),
        "the current project's own load must still land: {events:?}"
    );
    assert!(ids.contains(&8), "the current project's clip must be in the list: {ids:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
