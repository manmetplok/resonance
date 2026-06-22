//! Multi-target stem export plumbing (ba todo #325).
//!
//! Drives the synchronous export core (`export_stems`) directly — no
//! worker thread, no CLAP host, no audio device — and asserts the event
//! queue the engine streams to the app:
//!
//! * Two targets → both WAVs land on disk and the
//!   `StemExportProgress` / `StemExportTargetDone` sequence carries the
//!   right indices, capped by a `StemExportComplete` listing both files.
//! * A target that cannot be written reports `StemExportTargetError`
//!   for its index, the export KEEPS the earlier stem and continues, and
//!   `StemExportComplete` lists only the file that was written.
//! * The transport guard refuses to export while playing (no files).
//! * A cancel flag set before the run stops the queue and reports the
//!   (here empty) set of finished stems via `StemExportCancelled`.
//!
//! Plugin-free: tracks carry plain DC audio clips, so the rendered PCM
//! is known and every assertion is deterministic.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{
    export_stems, SharedState, StemBitDepth, StemSource, StemTarget, SyncClapInstance,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;

struct EngineState {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
}

impl EngineState {
    fn new() -> Self {
        Self {
            shared: Arc::new(SharedState::default()),
            tracks: Arc::new(RwLock::new(IndexMap::new())),
            busses: Arc::new(RwLock::new(IndexMap::new())),
            master: Arc::new(RwLock::new(MasterBus::new())),
            clips: Arc::new(RwLock::new(Vec::new())),
            midi_clips: Arc::new(RwLock::new(Vec::new())),
            plugins: Arc::new(RwLock::new(IndexMap::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
        }
    }

    fn add_track(&self, id: TrackId, output: TrackOutput) {
        let t = Track::new(id, format!("track {id}"));
        t.set_output(output);
        self.tracks.write().insert(id, t);
    }

    /// Push a constant-`value` DC clip on `track` over `[start, start+frames)`.
    fn add_dc_clip(&self, id: ClipId, track: TrackId, start: u64, frames: usize, value: f32) {
        self.clips.write().push(AudioClip {
            id,
            track_id: track,
            start_sample: start,
            source: ClipSource::Memory(vec![value; frames * 2]),
            name: "dc".into(),
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
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn export(
        &self,
        targets: Vec<StemTarget>,
        range: Option<(SamplePos, SamplePos)>,
        out_rate: u32,
        bit_depth: StemBitDepth,
        include_fx_tail: bool,
        event_tx: &Sender<AudioEvent>,
    ) {
        export_stems(
            targets,
            range,
            out_rate,
            bit_depth,
            include_fx_tail,
            &self.shared,
            &self.tracks,
            &self.busses,
            &self.master,
            &self.clips,
            &self.midi_clips,
            &self.plugins,
            &self.tempo_map,
            SR,
            event_tx,
        );
    }
}

fn drain(rx: &Receiver<AudioEvent>) -> Vec<AudioEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        out.push(ev);
    }
    out
}

fn tmp_path(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "resonance_stem_export_{}_{name}.wav",
        std::process::id()
    ));
    p
}

/// A two-track project whose clips span frames `[0, 300)`.
fn two_track_project() -> EngineState {
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    state.add_track(2, TrackOutput::Master);
    state.add_dc_clip(1, 1, 0, 100, 0.5); // track 1: [0,100)
    state.add_dc_clip(2, 2, 200, 100, 0.25); // track 2: [200,300)
    state
}

#[test]
fn two_targets_write_both_wavs_and_emit_progress_sequence() {
    let state = two_track_project();
    let p1 = tmp_path("seq_t1");
    let p2 = tmp_path("seq_t2");
    let _ = std::fs::remove_file(&p1);
    let _ = std::fs::remove_file(&p2);

    let targets = vec![
        StemTarget {
            source: StemSource::Track(1),
            path: p1.to_str().unwrap().to_string(),
        },
        StemTarget {
            source: StemSource::Track(2),
            path: p2.to_str().unwrap().to_string(),
        },
    ];

    let (tx, rx) = crossbeam_channel::unbounded::<AudioEvent>();
    // range = None exercises the full-project-range path; no FX tail so
    // the WAV length is exactly the project span.
    state.export(targets, None, SR, StemBitDepth::Float32, false, &tx);
    let events = drain(&rx);

    // Both stems exist, are stereo, and span the shared [0,300) range.
    for (label, p) in [("t1", &p1), ("t2", &p2)] {
        let reader = hound::WavReader::open(p).unwrap_or_else(|e| panic!("{label} WAV: {e}"));
        assert_eq!(reader.spec().channels, 2, "{label} stereo");
        assert_eq!(reader.spec().sample_rate, SR, "{label} rate");
        assert_eq!(reader.len(), 300 * 2, "{label} spans the shared range");
    }

    // Event queue: progress(0) → done(0) → progress(1) → done(1) → complete.
    assert!(
        matches!(
            events[0],
            AudioEvent::StemExportProgress {
                target_index: 0,
                total: 2,
                ..
            }
        ),
        "first event is target 0 progress, got {:?}",
        events[0]
    );
    assert!(
        matches!(&events[1], AudioEvent::StemExportTargetDone { index: 0, path } if path == p1.to_str().unwrap()),
        "second event is target 0 done, got {:?}",
        events[1]
    );
    assert!(
        matches!(
            events[2],
            AudioEvent::StemExportProgress {
                target_index: 1,
                total: 2,
                ..
            }
        ),
        "third event is target 1 progress, got {:?}",
        events[2]
    );
    assert!(
        matches!(&events[3], AudioEvent::StemExportTargetDone { index: 1, path } if path == p2.to_str().unwrap()),
        "fourth event is target 1 done, got {:?}",
        events[3]
    );
    match &events[4] {
        AudioEvent::StemExportComplete { files } => {
            assert_eq!(
                files,
                &[p1.to_str().unwrap().to_string(), p2.to_str().unwrap().to_string()],
                "complete lists both files in queue order"
            );
        }
        other => panic!("fifth event is complete, got {other:?}"),
    }
    assert_eq!(events.len(), 5, "no extra events: {events:?}");

    let _ = std::fs::remove_file(&p1);
    let _ = std::fs::remove_file(&p2);
}

#[test]
fn target_failure_keeps_prior_stem_and_continues() {
    let state = two_track_project();
    let p1 = tmp_path("fail_t1");
    let _ = std::fs::remove_file(&p1);
    // Target 2 writes into a directory that does not exist, so the WAV
    // writer fails — but target 1 must already be on disk and the export
    // must still finish (so the app can offer "retry remaining").
    let bad = format!(
        "{}/resonance_stem_export_nodir_{}/stem.wav",
        std::env::temp_dir().display(),
        std::process::id()
    );

    let targets = vec![
        StemTarget {
            source: StemSource::Track(1),
            path: p1.to_str().unwrap().to_string(),
        },
        StemTarget {
            source: StemSource::Track(2),
            path: bad.clone(),
        },
    ];

    let (tx, rx) = crossbeam_channel::unbounded::<AudioEvent>();
    state.export(targets, Some((0, 300)), SR, StemBitDepth::Float32, false, &tx);
    let events = drain(&rx);

    // Target 1's stem survived the target-2 failure.
    assert!(p1.exists(), "first stem is kept when a later target fails");
    assert!(
        !std::path::Path::new(&bad).exists(),
        "the failing target wrote no file"
    );

    // Target 2 reported a per-target error...
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::StemExportTargetError { index: 1, .. })),
        "a per-target error fires for the failing target: {events:?}"
    );
    // ...target 1 still completed...
    assert!(
        events
            .iter()
            .any(|e| matches!(e, AudioEvent::StemExportTargetDone { index: 0, .. })),
        "the earlier target still completes: {events:?}"
    );
    // ...and the export finished, listing only the written file.
    match events.last() {
        Some(AudioEvent::StemExportComplete { files }) => {
            assert_eq!(
                files,
                &[p1.to_str().unwrap().to_string()],
                "complete lists only the successfully written stem"
            );
        }
        other => panic!("export must finish with complete, got {other:?}"),
    }

    let _ = std::fs::remove_file(&p1);
}

#[test]
fn refuses_to_export_while_transport_playing() {
    let state = two_track_project();
    state.shared.playing.store(true, Ordering::SeqCst);
    let p1 = tmp_path("guard_t1");
    let _ = std::fs::remove_file(&p1);

    let targets = vec![StemTarget {
        source: StemSource::Track(1),
        path: p1.to_str().unwrap().to_string(),
    }];

    let (tx, rx) = crossbeam_channel::unbounded::<AudioEvent>();
    state.export(targets, None, SR, StemBitDepth::Float32, false, &tx);
    let events = drain(&rx);

    assert!(!p1.exists(), "no stem is written while the transport rolls");
    assert_eq!(events.len(), 1, "exactly one event: {events:?}");
    match &events[0] {
        AudioEvent::StemExportError(msg) => {
            assert!(msg.contains("transport"), "guard names the transport: {msg}")
        }
        other => panic!("expected StemExportError, got {other:?}"),
    }
}

#[test]
fn cancel_flag_stops_the_queue_and_reports_finished_stems() {
    let state = two_track_project();
    // Flag set before the run: the worker polls it at the top of the
    // first target and stops before rendering anything.
    state.shared.bounce_cancel.store(true, Ordering::SeqCst);
    let p1 = tmp_path("cancel_t1");
    let _ = std::fs::remove_file(&p1);

    let targets = vec![StemTarget {
        source: StemSource::Track(1),
        path: p1.to_str().unwrap().to_string(),
    }];

    let (tx, rx) = crossbeam_channel::unbounded::<AudioEvent>();
    state.export(targets, Some((0, 300)), SR, StemBitDepth::Float32, false, &tx);
    let events = drain(&rx);

    assert!(!p1.exists(), "a cancelled target writes nothing");
    match events.as_slice() {
        [AudioEvent::StemExportCancelled { files }] => {
            assert!(files.is_empty(), "no stems finished before cancel: {files:?}")
        }
        other => panic!("expected a single StemExportCancelled, got {other:?}"),
    }
    // The flag is consumed so the next export starts clean.
    assert!(
        !state.shared.bounce_cancel.load(Ordering::SeqCst),
        "cancel flag is reset after firing"
    );
}
