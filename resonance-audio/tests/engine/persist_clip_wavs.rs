//! `PersistClipWavs` (code review FU-V5b): a slow-path (full-reload) undo
//! runs `ClearAll` and reloads every audio clip from
//! `{project_dir}/audio/clip_<id>.wav` — a file that used to exist only
//! after a save. A clip that lived in RAM (a bounce) or in a render's
//! `vocal_*.wav` came back silent. These drive the real handlers in the
//! order the app sends them: persist at snapshot time, then `ClearAll`,
//! then the replay's `LoadClipFromWav`, and check the audio that comes back.

use std::path::{Path, PathBuf};
use std::time::Duration;

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::transcode_to_wav;
use resonance_audio::types::*;

const TRACK: TrackId = 3;
const FRAMES: usize = 512;

fn project_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-persist-clip-wavs-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("audio")).unwrap();
    dir
}

/// A distinguishable stereo ramp scaled by `level`.
fn pcm(level: f32) -> Vec<f32> {
    (0..FRAMES * 2).map(|i| level * (i as f32 / (FRAMES * 2) as f32)).collect()
}

fn clip(id: ClipId, source: ClipSource) -> AudioClip {
    AudioClip {
        id,
        track_id: TRACK,
        start_sample: 0,
        source,
        name: format!("clip {id}"),
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
        warp_algorithm: Default::default(),
        warp_markers: Vec::new(),
        tuning_render_cache: None,
    }
}

fn clip_wav(dir: &Path, id: ClipId) -> PathBuf {
    dir.join("audio").join(format!("clip_{id}.wav"))
}

/// What the slow-path replay does: clear the engine, then reload each
/// clip from its `clip_<id>.wav`. Returns the reloaded clips' PCM by id.
fn full_reload(h: &mut EngineHandlerHarness, dir: &Path, ids: &[ClipId]) -> Vec<(ClipId, Vec<f32>)> {
    h.clear_all();
    for &id in ids {
        h.load_clip_from_wav(id, TRACK, 0, clip_wav(dir, id), format!("clip {id}"));
    }
    h.settle_imports(Duration::from_secs(10));
    let mut out: Vec<(ClipId, Vec<f32>)> = h
        .take_clips()
        .into_iter()
        .map(|c| (c.id, c.source.as_frames().to_vec()))
        .collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

#[test]
fn an_in_ram_clip_survives_a_full_reload_after_persist() {
    let dir = project_dir("memory");
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(dir.clone());
    h.push_clip(clip(7, ClipSource::memory(pcm(0.5))));

    h.persist_clip_wavs();
    assert!(clip_wav(&dir, 7).is_file(), "persist writes clip_7.wav");

    let reloaded = full_reload(&mut h, &dir, &[7]);
    assert_eq!(reloaded.len(), 1, "the clip reloads from its WAV");
    assert_eq!(reloaded[0].1, pcm(0.5), "with the audio it had");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_render_take_survives_its_file_being_unlinked() {
    // A vocal clip plays from `audio/vocal_<ts>.wav`; a re-render may
    // unlink that file before the engine gets to persist the clip — the
    // persist must still have the audio (it encodes what the engine
    // holds mapped).
    let dir = project_dir("vocal");
    let take = dir.join("audio").join("vocal_1.wav");
    transcode_to_wav(&take, &pcm(0.25), 48_000).unwrap();
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(dir.clone());
    h.push_clip(clip(4, ClipSource::open_wav(&take).unwrap()));
    std::fs::remove_file(&take).unwrap();

    h.persist_clip_wavs();

    let reloaded = full_reload(&mut h, &dir, &[4]);
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].1, pcm(0.25));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_mapped_clip_is_remapped_onto_its_persisted_file() {
    // Remapping keeps a later save from treating the clip as "mapped
    // elsewhere" and copying the old file over the new name in place — a
    // hard link shares the inode, so that copy would truncate both.
    let dir = project_dir("remap");
    let take = dir.join("audio").join("vocal_2.wav");
    transcode_to_wav(&take, &pcm(0.75), 48_000).unwrap();
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(dir.clone());
    h.push_clip(clip(9, ClipSource::open_wav(&take).unwrap()));
    h.push_clip(clip(10, ClipSource::memory(pcm(0.1))));

    h.persist_clip_wavs();

    let clips = h.take_clips();
    for c in &clips {
        assert_eq!(
            c.source.mapped_path(),
            Some(clip_wav(&dir, c.id).as_path()),
            "clip {} now plays from its own clip WAV",
            c.id
        );
    }
    assert!(take.is_file(), "the take itself is left alone");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_existing_clip_wav_is_never_overwritten() {
    let dir = project_dir("existing");
    transcode_to_wav(&clip_wav(&dir, 5), &pcm(0.9), 48_000).unwrap();
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(dir.clone());
    h.push_clip(clip(5, ClipSource::memory(pcm(0.2))));

    h.persist_clip_wavs();

    let reloaded = full_reload(&mut h, &dir, &[5]);
    assert_eq!(reloaded[0].1, pcm(0.9), "the file on disk is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn without_a_project_dir_nothing_is_written() {
    let mut h = EngineHandlerHarness::new();
    h.push_clip(clip(1, ClipSource::memory(pcm(0.5))));
    h.persist_clip_wavs();
    assert!(h.drain_events().is_empty(), "silent: no event, no error");
    assert!(h.take_clips()[0].source.mapped_path().is_none());
}
