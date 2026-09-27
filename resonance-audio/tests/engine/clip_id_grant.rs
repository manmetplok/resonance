//! The engine's standing clip-id grant (ARCH-04 D-7d, design doc
//! `docs/design/D-6-engine-created-ids.md` §4.2 / §6 D-7d).
//!
//! Recordings (C3), cycle-record passes (C4), live-MIDI captures (C5) and
//! realtime bounces draw their clip ids from ranges the app granted, in
//! order, and never count on their own. These pin each draw site's
//! out-of-ids behaviour, the low-water report, and `ClearAll`'s revoke.
//!
//! Everything here is hermetic: a record start is refused *before* the
//! input stream opens when the grant is short, so no test opens a device.

use std::path::{Path, PathBuf};
use std::time::Instant;

use ringbuf::traits::{Producer, Split};
use ringbuf::HeapRb;

use resonance_audio::test_support::{EngineHandlerHarness, LiveMidiEvent};
use resonance_audio::types::{
    AudioCommand, AudioEvent, EngineErrorKind, MidiClip, MidiNote, Track, TrackType,
    CLIP_GRANT_LOW_WATER,
};
use resonance_audio::{ClipIdGrant, RecordingState};

const SR: u32 = 48_000;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-clip-grant-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn wav_files(audio_dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(audio_dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn push_frames(prod: &mut ringbuf::HeapProd<f32>, frames: usize) {
    let chunk = vec![0.25f32; frames * 2];
    assert_eq!(prod.push_slice(&chunk), frames * 2, "ring too small");
}

fn armed(id: u64, track_type: TrackType) -> Track {
    let t = Track::with_type(id, format!("t{id}"), track_type);
    t.set_record_armed(true);
    t
}

fn note_on(track_id: u64, note: u8) -> LiveMidiEvent {
    LiveMidiEvent::InboundNoteOn {
        track_id,
        note,
        velocity: 0.8,
        arrival: Instant::now(),
    }
}

fn busy_errors(events: &[AudioEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::Error(err) if err.kind == EngineErrorKind::Busy => Some(err.message.clone()),
            _ => None,
        })
        .collect()
}

fn created_clip_ids(events: &[AudioEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::MidiClipCreated { clip_id, .. } => Some(*clip_id),
            _ => None,
        })
        .collect()
}

fn low_reports(events: &[AudioEvent]) -> Vec<u64> {
    events
        .iter()
        .filter_map(|e| match e {
            AudioEvent::IdGrantLow { clips_left } => Some(*clips_left),
            _ => None,
        })
        .collect()
}

/// Guard (1), C4: a cycle-record run on two tracks whose grant covers
/// exactly two further passes. The seams draw the granted ids in order;
/// the third seam finds none, so both tracks keep the pass that just
/// finished, stop recording, and say so. Nothing already on disk is
/// overwritten or deleted, however many seams follow.
#[test]
fn a_loop_record_run_draws_the_grant_in_order_and_stops_cleanly_when_it_runs_out() {
    let project_dir = make_tempdir("c4");
    let audio_dir = project_dir.join("audio");
    let pass = 4_800usize;

    let mut rec = RecordingState::new(SR);
    let (mut prod, cons) = HeapRb::<f32>::new(pass * 2 * 4).split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = SR;
    // Pass 0's writers, opened at record start from the first two ids.
    const G: u64 = 1 << 41;
    for (track, clip) in [(1u64, G), (2, G + 1)] {
        let buf = RecordingState::create_track_buf(&project_dir, track, clip, SR, SR, 0, false)
            .unwrap();
        rec.buffers.insert(track, buf);
    }
    // Exactly tracks × 2 more.
    let mut grant = ClipIdGrant::from_range(G + 2..G + 6);
    let mut clips = Vec::new();

    let mut per_seam = Vec::new();
    for _ in 0..3 {
        push_frames(&mut prod, pass);
        let rolled = rec.roll_audio_pass(SR, 0, &mut clips, &audio_dir, &mut grant, true);
        let mut ids: Vec<u64> = rolled.iter().map(|t| t.clip_id).collect();
        ids.sort_unstable();
        per_seam.push(ids);
    }
    assert_eq!(
        per_seam,
        vec![vec![G, G + 1], vec![G + 2, G + 3], vec![G + 4, G + 5]],
        "each pass's take is under the id its writer drew, in grant order"
    );
    assert!(grant.is_empty());
    assert!(rec.buffers.is_empty(), "both tracks stopped at the seam with no id");
    let (tx, rx) = crossbeam_channel::unbounded();
    rec.poll_write_errors(&tx);
    let errors: Vec<String> = rx
        .try_iter()
        .filter_map(|e| match e {
            AudioEvent::Error(err) => Some(err.message),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 2, "one per stopped track: {errors:?}");
    assert!(errors.iter().all(|m| m.contains("no clip id")), "{errors:?}");

    let files_after_run = wav_files(&audio_dir);
    assert_eq!(files_after_run.len(), 6, "three passes on two tracks: {files_after_run:?}");

    // More seams and the stop: the stopped tracks produce nothing, and
    // their last take's WAV is not mistaken for an empty pass and deleted.
    push_frames(&mut prod, pass);
    assert!(rec.roll_audio_pass(SR, 0, &mut clips, &audio_dir, &mut grant, true).is_empty());
    assert!(rec.roll_audio_pass(SR, 0, &mut clips, &audio_dir, &mut grant, false).is_empty());
    assert_eq!(wav_files(&audio_dir), files_after_run, "no take file touched");
    assert_eq!(clips.len(), 6);
    for c in &clips {
        assert_eq!(c.source.frame_count(), pass as u64, "clip {} intact", c.id);
        assert_eq!(c.name, "Take", "a bare kind; the app numbers it");
    }
    let _ = std::fs::remove_dir_all(&project_dir);
}

/// The low-water report fires once per dip below the mark, re-arms when
/// a grant lifts the total back to it, and a revoke re-arms it too.
#[test]
fn the_low_water_report_fires_once_per_dip() {
    let mut g = ClipIdGrant::from_range(0..CLIP_GRANT_LOW_WATER + 2);
    assert_eq!(g.low_water_report(), None);
    g.take();
    g.take();
    assert_eq!(g.len(), CLIP_GRANT_LOW_WATER);
    assert_eq!(g.low_water_report(), None, "at the mark is not below it");
    g.take();
    assert_eq!(g.low_water_report(), Some(CLIP_GRANT_LOW_WATER - 1));
    g.take();
    assert_eq!(g.low_water_report(), None, "once per dip");
    // A grant that doesn't reach the mark leaves the dip latched.
    g.extend(10_000..10_001);
    assert_eq!(g.low_water_report(), None);
    // One that does re-arms it.
    g.extend(20_000..20_000 + CLIP_GRANT_LOW_WATER);
    assert_eq!(g.low_water_report(), None);
    while g.len() >= CLIP_GRANT_LOW_WATER {
        g.take();
    }
    assert!(g.low_water_report().is_some(), "the next dip reports again");
    g.revoke();
    assert_eq!(g.low_water_report(), Some(0), "an empty grant asks after a revoke");
}

/// Guard (2), through the real handlers: live-MIDI first notes draw the
/// granted ids in order and `IdGrantLow` is sent once, at the crossing.
#[test]
fn live_midi_draws_the_grant_in_order_and_reports_low_once() {
    let mut h = EngineHandlerHarness::new();
    h.revoke_clip_grant();
    const G: u64 = 1 << 42;
    h.grant_clip_ids(G..G + CLIP_GRANT_LOW_WATER + 1);
    h.push_track(armed(3, TrackType::Instrument));

    let mut ids = Vec::new();
    let mut lows = Vec::new();
    for _ in 0..3 {
        h.play();
        h.live_midi_event(note_on(3, 60));
        h.stop();
        let events = h.drain_events();
        ids.extend(created_clip_ids(&events));
        lows.extend(low_reports(&events));
    }
    assert_eq!(ids, vec![G, G + 1, G + 2], "one clip per run, in grant order");
    assert_eq!(lows, vec![CLIP_GRANT_LOW_WATER - 1], "reported once, at the crossing");
}

/// Guard (3): `ClearAll` revokes the grant. A record start after it with
/// no new grant is refused cleanly — the transport rolls, nothing is
/// captured, one `Busy` error — and asks for ids. The ids of a grant from
/// before the clear are never drawn again; the next grant's are.
#[test]
fn clear_all_revokes_the_grant_and_a_stale_grant_is_never_drawn() {
    let dir = make_tempdir("clear");
    let mut h = EngineHandlerHarness::new();
    h.revoke_clip_grant();
    const OLD: u64 = 1 << 42;
    h.grant_clip_ids(OLD..OLD + 100);
    h.set_project_dir(dir.clone());

    h.clear_all();
    assert_eq!(h.clip_grant_len(), 0, "ClearAll revokes");
    h.drain_events();

    h.push_track(armed(1, TrackType::Audio));
    h.record(0);
    let events = h.drain_events();
    assert!(h.is_playing(), "the transport still rolls");
    assert!(
        !events.iter().any(|e| matches!(e, AudioEvent::RecordingStarted { .. })),
        "nothing records: {events:?}"
    );
    let busy = busy_errors(&events);
    assert_eq!(busy.len(), 1, "{events:?}");
    assert!(busy[0].contains("no clip ids"), "{busy:?}");
    assert_eq!(low_reports(&events), vec![0], "the engine asks for a grant");
    assert!(wav_files(&dir.join("audio")).is_empty(), "no take file opened");
    h.stop();

    // The replay's closing grant is what gets drawn — never the old one.
    const NEW: u64 = 1 << 43;
    h.grant_clip_ids(NEW..NEW + 10);
    h.push_track(armed(2, TrackType::Instrument));
    h.play();
    h.live_midi_event(note_on(2, 64));
    assert_eq!(created_clip_ids(&h.drain_events()), vec![NEW]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// §4.2 C3 is all or nothing: two capturing tracks and one id left refuse
/// the whole start rather than record one of them.
#[test]
fn a_record_start_short_of_ids_records_no_track() {
    let dir = make_tempdir("c3");
    let mut h = EngineHandlerHarness::new();
    h.revoke_clip_grant();
    h.grant_clip_ids(1 << 42..(1 << 42) + 1);
    h.set_project_dir(dir.clone());
    h.push_track(armed(1, TrackType::Audio));
    h.push_track(armed(2, TrackType::Audio));
    h.record(0);
    let events = h.drain_events();
    assert!(h.is_playing());
    assert_eq!(busy_errors(&events).len(), 1, "{events:?}");
    assert_eq!(h.clip_grant_len(), 1, "a refused start draws nothing");
    assert!(wav_files(&dir.join("audio")).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Guard (4), C5: with no ids the first note of a live-MIDI run is not
/// captured and the run reports one `Busy` error, however many notes
/// follow; a note after a refill opens the clip under the granted id. A
/// new run reports afresh.
#[test]
fn live_midi_with_no_ids_drops_the_note_with_one_error_and_recovers_on_refill() {
    let mut h = EngineHandlerHarness::new();
    h.revoke_clip_grant();
    h.push_track(armed(4, TrackType::Instrument));
    h.play();
    h.set_playhead(0);
    h.live_midi_event(note_on(4, 60));
    h.live_midi_event(note_on(4, 62));
    let events = h.drain_events();
    assert!(created_clip_ids(&events).is_empty(), "{events:?}");
    assert!(
        !events.iter().any(|e| matches!(e, AudioEvent::MidiNoteAdded { .. })),
        "no note captured"
    );
    assert_eq!(busy_errors(&events).len(), 1, "one error per run: {events:?}");
    assert!(h.midi_clip_ids().is_empty());

    const G: u64 = 1 << 42;
    h.grant_clip_ids(G..G + 2_000);
    h.live_midi_event(note_on(4, 64));
    let events = h.drain_events();
    assert_eq!(created_clip_ids(&events), vec![G]);
    assert!(busy_errors(&events).is_empty());
    let name = events.iter().find_map(|e| match e {
        AudioEvent::MidiClipCreated { name, .. } => Some(name.clone()),
        _ => None,
    });
    assert_eq!(name.as_deref(), Some("MIDI Take"), "a bare kind; the app numbers it");
    h.stop();

    h.revoke_clip_grant();
    h.play();
    h.live_midi_event(note_on(4, 60));
    assert_eq!(busy_errors(&h.drain_events()).len(), 1, "a new run reports again");
}

/// Design doc D-6 §4.2's unchecked claim for the realtime bounce, which
/// records through C3: a start refused for want of ids ends the bounce as
/// a cancel does — the error surfaces, and the target track the app
/// pre-created for the run is removed (`TrackRemoved`, then
/// `TrackBounceCancelled`) rather than left behind empty. It did not hold
/// before D-7d for any start failure.
#[test]
fn a_realtime_bounce_refused_for_want_of_ids_removes_its_target_track() {
    let dir = make_tempdir("bounce");
    let mut h = EngineHandlerHarness::new();
    h.revoke_clip_grant();
    h.set_project_dir(dir.clone());
    h.push_track(Track::with_type(10, "synth".into(), TrackType::Instrument));
    h.push_midi_clip(MidiClip {
        id: 1 << 42,
        track_id: 10,
        start_sample: 0,
        duration_ticks: 960,
        notes: vec![MidiNote {
            note: 60,
            velocity: 0.8,
            start_tick: 0,
            duration_ticks: 480,
        }],
        name: "clip".into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    h.add_track(11, Some("synth bounce".into()));
    h.drain_events();

    h.dispatch(AudioCommand::BounceTrackRealtimeToAudio {
        source_track_id: 10,
        target_track_id: 11,
        input_device_name: "none".into(),
        input_port_index: 0,
        mono: false,
    });
    let events = h.drain_events();
    assert_eq!(busy_errors(&events).len(), 1, "{events:?}");
    assert!(events.iter().any(|e| matches!(e, AudioEvent::TrackBounceError(_))));
    let removed = events
        .iter()
        .position(|e| matches!(e, AudioEvent::TrackRemoved { track_id: 11 }));
    let cancelled = events
        .iter()
        .position(|e| matches!(e, AudioEvent::TrackBounceCancelled { target_track_id: 11 }));
    assert!(
        matches!((removed, cancelled), (Some(r), Some(c)) if r < c),
        "{events:?}"
    );
    assert!(!h.test_track_ids().contains(&11), "the empty target track is gone");
    assert!(!h.is_playing(), "the bounce's transport run is unwound");
    assert!(wav_files(&dir.join("audio")).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// The harness grants a default block at construction, as the app does at
/// startup, so the recording and take tests keep their shape (design doc
/// D-6 §6 D-7d; claim checked here rather than assumed).
#[test]
fn the_harness_starts_with_a_default_grant() {
    let h = EngineHandlerHarness::new();
    let d = EngineHandlerHarness::DEFAULT_CLIP_GRANT;
    assert_eq!(h.clip_grant_len(), d.end - d.start);
}
