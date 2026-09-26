//! The clip-id partition (code review FU-A6a): the app allocates derived
//! clips (compose-generated MIDI, SVS vocal renders, …) from
//! [`DERIVED_CLIP_ID_BASE`] up, and the engine allocates GUI-drawn clips,
//! recordings and imports from 1 up. Every path that hands the engine a
//! concrete clip id used to bump `next_clip_id` past it, so the first
//! derived clip at `base` dragged the engine's counter to `base + 1` —
//! exactly the id the app's derived counter hands out next. The next
//! drawn clip and the next generated one then shared an id (and, for a
//! vocal render, `audio/clip_<id>.wav`).
//!
//! These drive the real handlers: an id at or above the base is taken
//! but never moves the engine's counter; one below it still does.

use std::path::PathBuf;
use std::time::Duration;

use resonance_audio::test_support::EngineHandlerHarness;
use resonance_audio::transcode_to_wav;
use resonance_audio::types::*;

const TRACK: TrackId = 3;

fn project_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-derived-clip-ids-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(dir.join("audio")).unwrap();
    dir
}

fn write_wav(path: &std::path::Path) {
    let pcm: Vec<f32> = (0..256).map(|i| i as f32 / 256.0).collect();
    transcode_to_wav(path, &pcm, 48_000).unwrap();
}

#[test]
fn a_derived_midi_clip_does_not_drag_the_engine_counter_into_the_app_range() {
    let mut h = EngineHandlerHarness::new();
    // The app generates a derived clip, draws one, generates another:
    // the app's derived counter hands out `base`, then `base + 1`.
    h.load_midi_clip_direct(DERIVED_CLIP_ID_BASE, TRACK);
    h.create_midi_clip(TRACK, 0, 1920);
    h.load_midi_clip_direct(DERIVED_CLIP_ID_BASE + 1, TRACK);

    let ids = h.midi_clip_ids();
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "clip ids collide: {ids:?}");
    assert_eq!(ids[0], 1, "the drawn clip takes the engine's own first id");
    assert!(h.next_clip_id() < DERIVED_CLIP_ID_BASE);
}

#[test]
fn a_derived_audio_clip_does_not_drag_the_engine_counter_into_the_app_range() {
    // A vocal render lands through `LoadClipFromWav` with a derived id.
    let dir = project_dir("wav");
    let wav = dir.join("audio").join("vocal_1.wav");
    write_wav(&wav);
    let mut h = EngineHandlerHarness::new();
    h.load_clip_from_wav(DERIVED_CLIP_ID_BASE, TRACK, 0, wav, "vocal".into());
    h.settle_imports(Duration::from_secs(10));
    assert_eq!(h.clip_ids(), vec![DERIVED_CLIP_ID_BASE]);
    assert!(
        h.next_clip_id() < DERIVED_CLIP_ID_BASE,
        "next_clip_id = {}",
        h.next_clip_id()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_project_dir_scan_reserves_only_engine_range_wavs() {
    // STATE-08 still reserves past every engine clip's WAV; a derived
    // clip's WAV (a persisted vocal render) is the app's to reserve.
    let dir = project_dir("scan");
    write_wav(&dir.join("audio").join("clip_7.wav"));
    write_wav(&dir.join("audio").join(format!("clip_{}.wav", DERIVED_CLIP_ID_BASE + 5)));
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(dir.clone());
    assert_eq!(h.next_clip_id(), 8);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_engine_range_id_still_raises_the_counter() {
    // A replayed engine clip (a drawn clip after undo, a reload) must
    // keep the counter above it, as before.
    let mut h = EngineHandlerHarness::new();
    h.load_midi_clip_direct(9, TRACK);
    assert_eq!(h.next_clip_id(), 10);
    h.create_midi_clip(TRACK, 0, 1920);
    assert_eq!(h.midi_clip_ids(), vec![9, 10]);
}
