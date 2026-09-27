//! Audio clips on the published render graph (code review ARCH-02 A2-8,
//! refactor todo B-5), driven through the real handlers.
//!
//! The clip list was the last map behind an `RwLock`, and the only one
//! worker threads wrote: the clip-load worker, the pitch analyser, the
//! offline renderers' retune caches and the bounce-in-place worker all
//! took its write lock from their own threads. Now the list lives in the
//! immutable `RenderGraph`; workers post their results to the engine
//! thread (`SharedState::inbox`), which is the only publisher. These tests
//! pin what that relies on:
//!
//! - a removed or replaced clip's audio — in-RAM samples or an mmap — is
//!   freed by the engine thread's retire sweep, never by a reader that
//!   pinned the graph it was in;
//! - an edit copies the clip, never its audio;
//! - a worker's result lands only when the engine thread applies it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use resonance_audio::test_support::{ensure_tuning_caches, EngineHandlerHarness};
use resonance_audio::transcode_to_wav;
use resonance_audio::types::*;

const SR: u32 = 48_000;

fn memory_clip(id: ClipId, track_id: TrackId, frames: usize) -> AudioClip {
    AudioClip {
        id,
        track_id,
        start_sample: 0,
        source: ClipSource::memory(vec![0.25; frames * 2]),
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

fn samples_of(clip: &AudioClip) -> Arc<[f32]> {
    match &clip.source {
        ClipSource::Memory(samples) => Arc::clone(samples),
        ClipSource::Mapped { .. } => panic!("expected an in-RAM clip"),
    }
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-render-graph-clips-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_wav(dir: &Path, name: &str, frames: usize) -> PathBuf {
    let path = dir.join(name);
    transcode_to_wav(&path, &vec![0.25; frames * 2], SR).expect("write test wav");
    path
}

/// Drop `pinned` — a graph a reader loaded — on a thread of its own, the
/// way the audio callback lets go of the graph it rendered from.
fn release_on_reader_thread<T: Send + 'static>(pinned: T) {
    std::thread::spawn(move || drop(pinned))
        .join()
        .expect("reader thread");
}

#[test]
fn a_deleted_clips_samples_are_freed_by_the_engine_sweep_not_the_reader() {
    let mut h = EngineHandlerHarness::new();
    h.push_clip(memory_clip(1, 1, 4_096));
    let samples = Arc::downgrade(&samples_of(&h.clip(1).expect("clip")));
    h.sweep_retired();

    // The "audio thread" pins the graph that lists the clip...
    let pinned = h.render_graph();
    // ...and the engine thread deletes it meanwhile.
    h.dispatch(AudioCommand::DeleteClip { clip_id: 1 });
    assert!(h.clip_ids().is_empty());

    release_on_reader_thread(pinned);
    assert!(
        samples.upgrade().is_some(),
        "the reader's drop must not free the samples: the retire queue owns the old graph"
    );
    h.sweep_retired();
    assert!(samples.upgrade().is_none(), "the engine-thread sweep frees them");
    assert!(h.shared().retired.is_empty());
}

#[test]
fn a_removed_tracks_mapped_clip_is_unmapped_by_the_engine_sweep() {
    let dir = tempdir("unmap");
    let wav = write_wav(&dir, "take.wav", 4_096);
    let mut h = EngineHandlerHarness::new();
    h.push_track(Track::new(1, "t".into()));
    h.load_clip_from_wav(1, 1, 0, wav, "take".into());
    h.settle_imports(Duration::from_secs(30));
    let mmap = match &h.clip(1).expect("loaded").source {
        ClipSource::Mapped { mmap, .. } => Arc::downgrade(mmap),
        ClipSource::Memory(_) => panic!("a project WAV loads mapped"),
    };
    h.sweep_retired();

    let pinned = h.render_graph();
    h.remove_track(1);
    assert!(h.clip_ids().is_empty(), "the track's clips go with it");

    release_on_reader_thread(pinned);
    assert!(mmap.upgrade().is_some(), "still mapped: the retire queue owns it");
    h.sweep_retired();
    assert!(mmap.upgrade().is_none(), "unmapped by the engine-thread sweep");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_persisted_clips_replaced_samples_are_freed_by_the_engine_sweep() {
    // `PersistClipWavs` (FU-V5b) remaps an in-RAM clip onto its new WAV:
    // the replaced source is the old in-RAM buffer.
    let dir = tempdir("persist");
    let mut h = EngineHandlerHarness::new();
    h.set_project_dir(dir.clone());
    h.push_clip(memory_clip(7, 1, 4_096));
    let samples = Arc::downgrade(&samples_of(&h.clip(7).expect("clip")));
    h.sweep_retired();

    let pinned = h.render_graph();
    h.persist_clip_wavs();
    assert!(
        h.clip(7).expect("clip").source.mapped_path().is_some(),
        "the clip now plays from its persisted WAV"
    );

    release_on_reader_thread(pinned);
    assert!(samples.upgrade().is_some(), "the retire queue owns the old buffer");
    h.sweep_retired();
    assert!(samples.upgrade().is_none(), "freed by the engine-thread sweep");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn clip_edits_copy_the_clip_never_its_audio() {
    let mut h = EngineHandlerHarness::new();
    h.push_clip(memory_clip(1, 1, 4_096));
    h.push_clip(memory_clip(2, 1, 4_096));
    let before = h.render_graph();

    h.dispatch(AudioCommand::TrimClip {
        clip_id: 1,
        new_start_sample: 100,
        trim_start_frames: 100,
        trim_end_frames: 0,
    });
    h.dispatch(AudioCommand::SetClipGain {
        clip_id: 1,
        gain_db: -6.0,
    });
    h.dispatch(AudioCommand::SplitClip {
        clip_id: 1,
        new_clip_id: 3,
        at_sample: 2_000,
    });

    let after = h.render_graph();
    let (old, new) = (before.clip(1).expect("old"), after.clip(1).expect("new"));
    assert_eq!(old.trim_start_frames, 0, "the pinned graph's clip is untouched");
    assert_eq!((new.trim_start_frames, new.gain_db), (100, -6.0));
    assert!(
        Arc::ptr_eq(&samples_of(old), &samples_of(new)),
        "the edited copy shares its samples"
    );
    assert!(
        Arc::ptr_eq(&samples_of(new), &samples_of(after.clip(3).expect("tail"))),
        "and so does the split's tail"
    );
    assert!(
        Arc::ptr_eq(&before.clips[1], &after.clips[1]),
        "a clip no edit touched is the very same Arc"
    );
    assert_eq!(
        after.clips.iter().map(|c| c.id).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "list order is kept, the tail appended"
    );
}

#[test]
fn a_finished_load_lands_only_when_the_engine_thread_applies_it() {
    let dir = tempdir("apply");
    let wav = write_wav(&dir, "clip.wav", 2_048);
    let mut h = EngineHandlerHarness::new();
    h.hold_imports();
    h.load_clip_from_wav(5, 1, 0, wav, "clip".into());
    for job in h.take_held_imports() {
        job();
    }
    // The worker is done, but it only posted its clip.
    assert!(h.clip_ids().is_empty(), "the worker does not publish");
    assert!(
        !h.drain_events()
            .iter()
            .any(|e| matches!(e, AudioEvent::ClipImported { .. })),
        "nor echo"
    );

    h.apply_worker_results();
    assert_eq!(h.clip_ids(), vec![5]);
    assert!(h
        .drain_events()
        .iter()
        .any(|e| matches!(e, AudioEvent::ClipImported { clip_id: 5, .. })));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_offline_renders_retune_caches_are_attached_by_the_engine_thread() {
    let frames = 4_800;
    let mut tuning = VocalTuning {
        notes: vec![NoteBlob {
            start_frame: 0,
            end_frame: frames as u64,
            mean_pitch_midi: 57.0,
            cents_contour: Vec::new(),
            edit: NoteEdit {
                semitone_offset: 2.0,
                correction_strength: 1.0,
                drift: 1.0,
                timing_nudge_frames: 0,
            },
        }],
        ..Default::default()
    };
    tuning.global.correction_amount = 1.0;
    let mut clip = memory_clip(1, 1, frames);
    clip.vocal_tuning = Some(tuning);

    let mut h = EngineHandlerHarness::new();
    h.push_clip(clip);
    h.sweep_retired();

    // What a bounce / export / stem / freeze worker runs before its chunk
    // loop: build the caches and render through them at once...
    let overlay = ensure_tuning_caches(h.shared(), SR);
    assert_eq!(overlay.rebuilt(), 1);
    let graph = h.render_graph();
    let rendered = overlay.apply(&graph.clips);
    assert!(rendered[0].tuning_render_cache.is_some());
    // ...without touching the live clip: that is the engine thread's job.
    assert!(h.clip(1).expect("clip").tuning_render_cache.is_none());

    h.apply_worker_results();
    let live = h.clip(1).expect("clip");
    let cache = live.tuning_render_cache.as_ref().expect("attached");
    assert!(
        Arc::ptr_eq(cache, rendered[0].tuning_render_cache.as_ref().unwrap()),
        "the live clip plays the very cache the render used"
    );
    drop(graph);
    assert_eq!(h.shared().retired.len(), 1, "attached in one publish");
}
