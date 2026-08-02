//! Offline mix measurement (ba todo #1218, doc #273).
//!
//! Drives the synchronous measurement core (`measure_mix`) directly — no
//! worker thread, no CLAP host, no audio device — and asserts the single
//! terminal event it emits:
//!
//! * Master + one track measured in ONE command come back in request
//!   order, over the SAME range, with sane BS.1770 numbers, and the
//!   isolated track is quieter than the master it contributes to.
//! * Repeating the identical command returns identical numbers (the
//!   offline render is deterministic) — the property `meter.measure`
//!   depends on to be usable as a measuring instrument.
//! * A bus target folds in every member track, and excludes tracks routed
//!   elsewhere: the group attribution that makes measurement immune to the
//!   drum-bleed error class of mute-and-bounce.
//! * The live path is master-only and reports itself as such.
//! * Nothing is written to disk, and the command refuses (with no
//!   numbers) on a rolling transport, an empty target list, or while
//!   another offline render holds the renderer.
//!
//! Plugin-free: tracks carry plain audio clips, so no CLAP host is needed
//! and every number is reproducible. That is also the one limit of this
//! file — see `track_target_folds_in_its_sub_tracks`.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use indexmap::IndexMap;
use parking_lot::{Mutex, RwLock};

use resonance_audio::__test_support::{
    measure_mix, measure_rendered_buffer, stem_filter, MeasureSource, MixMeasurement, SharedState,
    StemSource, SyncClapInstance, MEASURE_BUSY_MSG,
};
use resonance_audio::types::*;

const SR: u32 = 48_000;
/// Long enough that the 3 s short-term window fills, so every LUFS
/// readout in a measurement is finite and assertable.
const FRAMES: usize = (SR as usize) * 4;

struct EngineState {
    shared: Arc<SharedState>,
    tracks: Arc<RwLock<IndexMap<TrackId, Track>>>,
    busses: Arc<RwLock<IndexMap<BusId, Bus>>>,
    master: Arc<RwLock<MasterBus>>,
    clips: Arc<RwLock<Vec<AudioClip>>>,
    midi_clips: Arc<RwLock<Vec<MidiClip>>>,
    plugins: Arc<RwLock<IndexMap<PluginInstanceId, Mutex<SyncClapInstance>>>>,
    tempo_map: Arc<arc_swap::ArcSwap<TempoMap>>,
    tx: Sender<AudioEvent>,
    rx: Receiver<AudioEvent>,
}

impl EngineState {
    fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        Self {
            shared: Arc::new(SharedState::default()),
            tracks: Arc::new(RwLock::new(IndexMap::new())),
            busses: Arc::new(RwLock::new(IndexMap::new())),
            master: Arc::new(RwLock::new(MasterBus::new())),
            clips: Arc::new(RwLock::new(Vec::new())),
            midi_clips: Arc::new(RwLock::new(Vec::new())),
            plugins: Arc::new(RwLock::new(IndexMap::new())),
            tempo_map: Arc::new(arc_swap::ArcSwap::from_pointee(TempoMap::default())),
            tx,
            rx,
        }
    }

    fn add_track(&self, id: TrackId, output: TrackOutput) {
        let t = Track::new(id, format!("track {id}"));
        t.set_output(output);
        self.tracks.write().insert(id, t);
    }

    /// Push a steady 220 Hz sine clip of the given amplitude on `track`,
    /// starting at frame 0. A sine (not DC) because K-weighting rejects
    /// DC almost entirely, which would make every LUFS readout `-inf`.
    fn add_sine_clip(&self, id: ClipId, track: TrackId, amplitude: f32) {
        let mut pcm = vec![0.0f32; FRAMES * 2];
        let step = std::f32::consts::TAU * 220.0 / SR as f32;
        for (f, frame) in pcm.chunks_exact_mut(2).enumerate() {
            let s = amplitude * (step * f as f32).sin();
            frame[0] = s;
            frame[1] = s;
        }
        self.clips.write().push(AudioClip {
            id,
            track_id: track,
            start_sample: 0,
            source: ClipSource::Memory(pcm),
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
        });
    }

    fn measure(&self, targets: Vec<StemSource>, source: MeasureSource) {
        measure_mix(
            targets,
            None,
            source,
            &self.shared,
            &self.tracks,
            &self.busses,
            &self.master,
            &self.clips,
            &self.midi_clips,
            &self.plugins,
            &self.tempo_map,
            SR,
            &self.tx,
        );
    }

    /// Run a measurement and expect it to succeed, returning the results.
    fn measured(&self, targets: Vec<StemSource>) -> Vec<MixMeasurement> {
        self.measure(targets, MeasureSource::Render);
        let events: Vec<AudioEvent> = self.rx.try_iter().collect();
        assert_eq!(events.len(), 1, "exactly one terminal event: {events:?}");
        match &events[0] {
            AudioEvent::MixMeasured { results } => results.clone(),
            other => panic!("expected MixMeasured, got {other:?}"),
        }
    }

    /// Run a measurement and expect it to fail, returning the message.
    fn measure_error(&self, targets: Vec<StemSource>, source: MeasureSource) -> String {
        self.measure(targets, source);
        let events: Vec<AudioEvent> = self.rx.try_iter().collect();
        assert_eq!(events.len(), 1, "exactly one terminal event: {events:?}");
        match &events[0] {
            AudioEvent::MixMeasureError(message) => message.clone(),
            other => panic!("expected MixMeasureError, got {other:?}"),
        }
    }
}

/// A two-track project: track 1 loud, track 2 quiet, both to master.
fn two_track_project() -> EngineState {
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    state.add_track(2, TrackOutput::Master);
    state.add_sine_clip(1, 1, 0.5);
    state.add_sine_clip(2, 2, 0.125);
    state
}

// ---- the happy path ------------------------------------------------------

#[test]
fn measures_master_and_track_over_one_shared_range() {
    let state = two_track_project();
    let results = state.measured(vec![StemSource::Master, StemSource::Track(1)]);

    assert_eq!(results.len(), 2, "one measurement per target");
    assert_eq!(results[0].target, StemSource::Master, "request order kept");
    assert_eq!(results[1].target, StemSource::Track(1));

    // Every target is measured over the SAME range — that is what makes
    // the numbers comparable and lets `meter.stems` be one engine pass.
    assert_eq!(results[0].range_start, results[1].range_start);
    assert_eq!(results[0].range_end, results[1].range_end);
    assert_eq!(results[0].frames, results[1].frames);
    assert_eq!(
        results[0].frames,
        results[0].range_end - results[0].range_start,
        "frames match the reported range"
    );
    assert_eq!(results[0].range_end, FRAMES as u64, "full project range");

    for m in &results {
        assert_eq!(m.source, MeasureSource::Render);
        assert!(
            m.lufs_integrated.is_finite() && (-60.0..0.0).contains(&m.lufs_integrated),
            "integrated LUFS is finite and sane: {}",
            m.lufs_integrated
        );
        assert!(
            m.lufs_short_term_max.is_finite(),
            "4 s of audio fills the 3 s short-term window"
        );
        assert!(m.lufs_momentary_max.is_finite());
        // A steady sine peaks at its own amplitude on every scale.
        assert!(m.sample_peak_db < 0.0, "sine below full scale");
        assert!(
            m.true_peak_dbtp >= m.sample_peak_db - 0.01,
            "true peak never reads below the sample peak ({} vs {})",
            m.true_peak_dbtp,
            m.sample_peak_db
        );
        assert_eq!(m.clipped_samples, 0, "nothing clips");
        // Identical L and R: perfectly correlated, nothing lost in mono.
        assert!(m.correlation > 0.99, "correlation {}", m.correlation);
        assert!(m.mono_penalty_db.abs() < 0.01, "mono penalty is ~0");
        // 220 Hz sine: essentially all the energy sits in the low band.
        assert!(m.bands.low > 0.9, "band shares {:?}", m.bands);
        assert!(m.crest_db > 0.0, "a sine has a positive crest factor");
    }

    // The isolated track is quieter than the master it feeds, because
    // master sums it with the second track.
    assert!(
        results[1].lufs_integrated < results[0].lufs_integrated - 0.05,
        "track alone ({}) must be quieter than the master sum ({})",
        results[1].lufs_integrated,
        results[0].lufs_integrated
    );
}

#[test]
fn repeated_measurement_returns_identical_numbers() {
    let state = two_track_project();
    let first = state.measured(vec![StemSource::Master, StemSource::Track(2)]);
    let second = state.measured(vec![StemSource::Master, StemSource::Track(2)]);
    assert_eq!(
        first, second,
        "the offline render is deterministic, so measurement is repeatable"
    );
}

#[test]
fn measurement_writes_no_files_and_leaves_the_renderer_free() {
    let state = two_track_project();
    let before = state.clips.read().len();
    state.measured(vec![StemSource::Master]);
    assert_eq!(state.clips.read().len(), before, "no clip was added");
    assert!(
        !state.shared.playing.load(Ordering::SeqCst),
        "transport untouched"
    );
    assert_eq!(
        state.shared.offline_render_count.load(Ordering::SeqCst),
        0,
        "the exclusive renderer lock is released when the measurement ends"
    );
    // And a second measurement therefore still works.
    state.measured(vec![StemSource::Master]);
}

// ---- group attribution ---------------------------------------------------

#[test]
fn bus_target_folds_in_every_member_track_and_excludes_the_others() {
    let state = EngineState::new();
    state.busses.write().insert(7, Bus::new(7, "drum bus".into()));
    state.add_track(1, TrackOutput::Bus(7));
    state.add_track(2, TrackOutput::Bus(7));
    state.add_track(3, TrackOutput::Master);
    state.add_sine_clip(1, 1, 0.4);
    state.add_sine_clip(2, 2, 0.4);
    state.add_sine_clip(3, 3, 0.4);

    let results = state.measured(vec![
        StemSource::Bus(7),
        StemSource::Track(1),
        StemSource::Master,
    ]);
    let (bus, track, master) = (&results[0], &results[1], &results[2]);

    // Summing two equal sines is ~6 dB over one of them: the bus target
    // measures the GROUP, not one member.
    assert!(
        bus.lufs_integrated > track.lufs_integrated + 3.0,
        "bus ({}) must be louder than one member ({})",
        bus.lufs_integrated,
        track.lufs_integrated
    );
    // Track 3 is routed to master, not to the bus, so it must not leak
    // into the bus measurement — master (three sines) stays louder.
    assert!(
        master.lufs_integrated > bus.lufs_integrated + 1.0,
        "master ({}) includes the master-routed track the bus ({}) excludes",
        master.lufs_integrated,
        bus.lufs_integrated
    );
}

#[test]
fn track_target_folds_in_its_sub_tracks() {
    // A multi-output instrument's extra ports become sibling sub-tracks.
    // `measure_mix` renders through `render_stem`, whose `stem_filter`
    // decides which tracks contribute, so this is the assertion that a
    // "Drums" measurement covers the whole kit and not just port 0.
    //
    // It is asserted on the filter rather than on rendered audio because
    // a sub-track carries no clips of its own — its signal arrives only
    // from the parent instrument's port fan-out, which needs a real
    // multi-output CLAP plugin that this plugin-free harness cannot host
    // (`mixer/render_core.rs:796` skips sub-tracks in the clip pass).
    let state = EngineState::new();
    state.add_track(1, TrackOutput::Master);
    state.add_track(2, TrackOutput::Master);
    state
        .tracks
        .write()
        .insert(10, Track::new_sub_track(10, "kick".into(), 1, 1));

    let filter = stem_filter(StemSource::Track(1), &state.tracks.read());
    assert!(filter.contains(1), "the instrument track itself");
    assert!(
        filter.contains(10),
        "its sub-track (an extra instrument output) is measured with it"
    );
    assert!(!filter.contains(2), "an unrelated track is not");
}

// ---- the live path -------------------------------------------------------

#[test]
fn live_measurement_is_master_only_and_says_so() {
    let state = two_track_project();

    let err = state.measure_error(vec![StemSource::Track(1)], MeasureSource::Live);
    assert!(
        err.contains("master"),
        "a live track measurement is refused, not silently rendered: {err}"
    );
    let err = state.measure_error(
        vec![StemSource::Master, StemSource::Master],
        MeasureSource::Live,
    );
    assert!(err.contains("master"), "one live target only: {err}");

    state.measure(vec![StemSource::Master], MeasureSource::Live);
    let events: Vec<AudioEvent> = state.rx.try_iter().collect();
    match &events[..] {
        [AudioEvent::MixMeasured { results }] => {
            assert_eq!(results.len(), 1);
            assert_eq!(results[0].target, StemSource::Master);
            assert_eq!(
                results[0].source,
                MeasureSource::Live,
                "the result reports which path produced it"
            );
            assert_eq!(results[0].frames, 0, "the live meter measures no range");
        }
        other => panic!("expected one MixMeasured, got {other:?}"),
    }
}

// ---- refusals ------------------------------------------------------------

#[test]
fn measurement_is_refused_while_another_offline_render_runs() {
    let state = two_track_project();
    // Stand in for a bounce / export / freeze worker holding the renderer.
    state.shared.offline_render_count.store(1, Ordering::SeqCst);

    let err = state.measure_error(vec![StemSource::Master], MeasureSource::Render);
    assert_eq!(
        err, MEASURE_BUSY_MSG,
        "a read-only measurement never disturbs a render that writes a file"
    );

    // Once the other render finishes, measurement works again.
    state.shared.offline_render_count.store(0, Ordering::SeqCst);
    assert_eq!(state.measured(vec![StemSource::Master]).len(), 1);
}

#[test]
fn measurement_is_refused_while_the_transport_rolls() {
    let state = two_track_project();
    state.shared.playing.store(true, Ordering::SeqCst);

    let err = state.measure_error(vec![StemSource::Master], MeasureSource::Render);
    assert!(err.contains("transport"), "guard names the transport: {err}");
    assert_eq!(
        state.shared.offline_render_count.load(Ordering::SeqCst),
        0,
        "the refused measurement released the renderer lock"
    );
}

#[test]
fn measurement_is_refused_with_no_targets_or_no_audio() {
    let state = two_track_project();
    let err = state.measure_error(Vec::new(), MeasureSource::Render);
    assert!(err.contains("targets"), "{err}");

    // An empty project has no range to measure.
    let empty = EngineState::new();
    empty.add_track(1, TrackOutput::Master);
    let err = empty.measure_error(vec![StemSource::Master], MeasureSource::Render);
    assert!(err.contains("No audio"), "{err}");
}

// ---- the pure measurement core -------------------------------------------

#[test]
fn measure_rendered_buffer_reports_peaks_clips_and_phase() {
    // Half a second of hard-clipped, fully anti-phase stereo.
    let frames = SR as usize / 2;
    let mut pcm = vec![0.0f32; frames * 2];
    let step = std::f32::consts::TAU * 440.0 / SR as f32;
    for (f, frame) in pcm.chunks_exact_mut(2).enumerate() {
        let s = 1.5 * (step * f as f32).sin();
        frame[0] = s.clamp(-1.0, 1.0);
        frame[1] = -s.clamp(-1.0, 1.0);
    }

    let m = measure_rendered_buffer(StemSource::Master, 0, frames as u64, &pcm, SR);
    assert_eq!(m.frames, frames as u64);
    assert!(
        (m.sample_peak_db - 0.0).abs() < 1e-3,
        "clipped material reads full scale: {}",
        m.sample_peak_db
    );
    assert!(m.clipped_samples > 0, "the clipped samples are counted");
    assert!(
        m.correlation < -0.99,
        "anti-phase channels correlate at -1: {}",
        m.correlation
    );
    assert!(
        m.mono_penalty_db < -20.0,
        "an anti-phase mix all but vanishes in mono: {}",
        m.mono_penalty_db
    );
    assert!(
        m.crest_db > 0.0 && m.crest_db < 6.0,
        "a near-square wave has a small crest factor: {}",
        m.crest_db
    );
}
