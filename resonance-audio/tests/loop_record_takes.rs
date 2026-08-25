//! Proves the core cycle-record invariant of todo #408: looping over a
//! region while recording yields one distinct take per pass, and rolling
//! the capture at each loop seam never drops an input frame.
//!
//! These drive [`RecordingState::roll_audio_pass`] directly — the same
//! engine-thread routine the loop-seam poll calls — through the
//! `#[doc(hidden)] pub use` test surface in `lib.rs`, so the capture
//! mechanism is exercised without spinning up the audio engine or a real
//! input device.

use std::path::PathBuf;

use ringbuf::traits::{Producer, Split};
use ringbuf::HeapRb;

use resonance_audio::RecordingState;

fn make_tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-looprec-test-{}-{}",
        tag,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Push `frames` stereo frames into `prod`, encoding each channel as the
/// running global frame index so a later read can prove no frame was
/// dropped or reordered across a seam.
fn push_ramp(prod: &mut ringbuf::HeapProd<f32>, start_frame: u64, frames: u64) {
    let mut chunk = vec![0.0f32; 1000 * 2];
    let mut written = 0u64;
    while written < frames {
        let n = (frames - written).min(1000) as usize;
        for f in 0..n {
            let v = (start_frame + written + f as u64) as f32;
            chunk[f * 2] = v;
            chunk[f * 2 + 1] = v;
        }
        prod.push_slice(&chunk[..n * 2]);
        written += n as u64;
    }
}

#[test]
fn three_loop_passes_yield_three_takes_with_no_dropped_frames() {
    let project_dir = make_tempdir("three-passes");
    let audio_dir = project_dir.join("audio");
    let sr = 48_000u32;
    let loop_frames = 24_000u64; // half-second loop region
    let passes = 3u64;

    let mut rec = RecordingState::new(sr);
    let ring: HeapRb<f32> = HeapRb::new((loop_frames as usize) * 2 * 2);
    let (mut prod, cons) = ring.split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = sr;
    rec.start_sample = 0;

    // The first pass's writer is the one created at record start.
    let buf = RecordingState::create_track_buf(
        &project_dir, /* track */ 7, /* clip */ 1, sr, sr, /* port */ 0, /* mono */ false,
    )
    .unwrap();
    rec.buffers.insert(7, buf);

    let clips = parking_lot::RwLock::new(Vec::new());
    let mut next_clip_id = 2u64; // pass 0 already holds clip id 1

    // Feed one loop's worth of audio and roll at the seam, three times.
    for pass in 0..passes {
        push_ramp(&mut prod, pass * loop_frames, loop_frames);
        let rolled = rec.roll_audio_pass(
            sr,
            /* clip_start_sample */ 0,
            &clips,
            &audio_dir,
            &mut next_clip_id,
            /* reopen */ true,
        );
        assert_eq!(rolled.len(), 1, "pass {pass} should produce exactly one take");
        assert_eq!(
            rolled[0].duration_samples, loop_frames,
            "pass {pass} take has the wrong length"
        );
    }

    // Exactly three retained takes — the fourth writer is open but empty.
    let guard = clips.read();
    assert_eq!(guard.len(), passes as usize, "expected one clip per pass");

    // Every take is a distinct clip with its own on-disk WAV holding a full
    // loop's worth of frames, and the concatenation of all three reproduces
    // the continuous input ramp — i.e. no frame was dropped at any seam.
    let mut ids: Vec<u64> = guard.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2, 3], "takes should use fresh clip ids per pass");

    let mut expected_frame = 0u64;
    for id in [1u64, 2, 3] {
        let clip = guard.iter().find(|c| c.id == id).unwrap();
        assert_eq!(
            clip.source.frame_count(),
            loop_frames,
            "take {id} should hold one loop of frames"
        );
        let frames = clip.source.as_frames();
        for f in 0..loop_frames {
            assert_eq!(
                frames[f as usize * 2],
                expected_frame as f32,
                "take {id} dropped or reordered a frame at offset {f}"
            );
            expected_frame += 1;
        }
    }
    assert_eq!(
        expected_frame,
        passes * loop_frames,
        "total captured frames must equal everything fed in"
    );

    drop(guard);
    let _ = std::fs::remove_dir_all(&project_dir);
}

#[test]
fn trailing_pass_rolls_without_reopening_and_clears_buffers() {
    let project_dir = make_tempdir("trailing");
    let audio_dir = project_dir.join("audio");
    let sr = 48_000u32;
    let loop_frames = 12_000u64;

    let mut rec = RecordingState::new(sr);
    let ring: HeapRb<f32> = HeapRb::new((loop_frames as usize) * 2 * 2);
    let (mut prod, cons) = ring.split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = sr;
    rec.start_sample = 0;

    let buf =
        RecordingState::create_track_buf(&project_dir, 1, 1, sr, sr, 0, false).unwrap();
    rec.buffers.insert(1, buf);

    let clips = parking_lot::RwLock::new(Vec::new());
    let mut next_clip_id = 2u64;

    // One seam roll (reopen) then a final trailing roll at stop (no reopen).
    push_ramp(&mut prod, 0, loop_frames);
    let _ = rec.roll_audio_pass(sr, 0, &clips, &audio_dir, &mut next_clip_id, true);
    push_ramp(&mut prod, loop_frames, loop_frames);
    drop(prod); // emulate the input stream closing on stop
    let trailing = rec.roll_audio_pass(sr, 0, &clips, &audio_dir, &mut next_clip_id, false);

    assert_eq!(trailing.len(), 1, "trailing pass should emit one take");
    assert_eq!(clips.read().len(), 2, "two passes -> two takes");
    assert!(
        rec.buffers.is_empty(),
        "the trailing (no-reopen) roll must close out the per-track buffers"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
}

// ---------------------------------------------------------------------------
// A take's audible extent (todo #1396)
// ---------------------------------------------------------------------------
//
// `TakeCaptured` is the only news the app gets about a cycle-record pass —
// no `RecordingFinished` follows a take clip, so nothing the app holds can
// be consulted instead. If the extent the engine files is the loop slot
// rather than what the pass really recorded, the app's promote clamp and
// its take lane both silently claim material that was never recorded, and
// nothing downstream can tell.

/// A pass that punched in a quarter of the way into the loop records three
/// quarters of it, and the extent `finalize_loop_record_pass` files must
/// say so.
///
/// Drives the real roll and reads the extent off the very value the emit
/// site reads it off (`RolledAudioTake::extent`), so the capture path and
/// this test cannot drift apart. Filing `slot` instead — the pre-#1396
/// behaviour, and the tempting one, since `slot` is right there at the emit
/// site — fails both assertions below.
#[test]
fn a_punched_in_pass_reports_the_extent_it_recorded_not_its_slot() {
    use resonance_audio::__test_support::push_take;
    use resonance_common::{TakeContent, TakeGroup, TimelineRange};

    let project_dir = make_tempdir("punch-in-extent");
    let audio_dir = project_dir.join("audio");
    let sr = 48_000u32;
    let loop_frames = 48_000u64;
    let slot = TimelineRange::new(96_000, loop_frames);
    // Record started a quarter of the way through the loop region.
    let punch_in = slot.start + loop_frames / 4;
    let recorded = loop_frames - loop_frames / 4;

    let mut rec = RecordingState::new(sr);
    let ring: HeapRb<f32> = HeapRb::new((loop_frames as usize) * 2 * 2);
    let (mut prod, cons) = ring.split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = sr;
    rec.start_sample = punch_in;

    let buf = RecordingState::create_track_buf(&project_dir, 7, 1, sr, sr, 0, false).unwrap();
    rec.buffers.insert(7, buf);

    let clips = parking_lot::RwLock::new(Vec::new());
    let mut next_clip_id = 2u64;

    // Pass 0's writer starts at the punch-in, exactly as
    // `finalize_loop_record_pass` positions it.
    push_ramp(&mut prod, 0, recorded);
    let rolled = rec.roll_audio_pass(sr, punch_in, &clips, &audio_dir, &mut next_clip_id, true);
    assert_eq!(rolled.len(), 1, "one armed track, one take");

    let extent = rolled[0].extent();
    assert_eq!(
        extent,
        TimelineRange::new(punch_in, recorded),
        "the extent is the rolled clip's own span"
    );
    assert_ne!(extent, slot, "and it is emphatically not the slot");

    // Filed onto the take, it is what every consumer resolves against.
    let mut group = TakeGroup::new(1, 7, slot);
    let take_id = push_take(
        &mut group,
        slot,
        0,
        extent,
        &TakeContent::Audio {
            clip_ref: rolled[0].clip_id,
        },
    );
    let audible = group.take(take_id).expect("take").audible_extent(slot);
    assert_eq!(audible.start, punch_in, "silent up to the punch-in");
    assert_eq!(audible.end(), slot.end(), "and audible to the loop end");
    assert!(audible.length < slot.length, "strictly inside its slot");

    let _ = std::fs::remove_dir_all(&project_dir);
}

/// The other short pass: stopped mid-loop, so the trailing roll ends before
/// the slot does.
#[test]
fn a_pass_cut_short_at_stop_reports_the_shorter_extent() {
    use resonance_audio::__test_support::push_take;
    use resonance_common::{TakeContent, TakeGroup, TimelineRange};

    let project_dir = make_tempdir("cut-short-extent");
    let audio_dir = project_dir.join("audio");
    let sr = 48_000u32;
    let loop_frames = 48_000u64;
    let slot = TimelineRange::new(0, loop_frames);
    let recorded = loop_frames / 2;

    let mut rec = RecordingState::new(sr);
    let ring: HeapRb<f32> = HeapRb::new((loop_frames as usize) * 2 * 2);
    let (mut prod, cons) = ring.split();
    rec.ring_consumer = Some(cons);
    rec.input_channels = 2;
    rec.input_sample_rate = sr;
    rec.start_sample = 0;

    let buf = RecordingState::create_track_buf(&project_dir, 7, 1, sr, sr, 0, false).unwrap();
    rec.buffers.insert(7, buf);

    let clips = parking_lot::RwLock::new(Vec::new());
    let mut next_clip_id = 2u64;

    push_ramp(&mut prod, 0, recorded);
    drop(prod); // the input stream closes at stop
    let rolled = rec.roll_audio_pass(sr, slot.start, &clips, &audio_dir, &mut next_clip_id, false);
    assert_eq!(rolled.len(), 1);

    let extent = rolled[0].extent();
    let mut group = TakeGroup::new(1, 7, slot);
    let take_id = push_take(
        &mut group,
        slot,
        0,
        extent,
        &TakeContent::Audio {
            clip_ref: rolled[0].clip_id,
        },
    );
    let audible = group.take(take_id).expect("take").audible_extent(slot);
    assert_eq!(audible.start, slot.start, "it starts where the loop does...");
    assert!(
        audible.end() < slot.end(),
        "...and stops where the user did, not where the loop would have: {audible:?}"
    );

    let _ = std::fs::remove_dir_all(&project_dir);
}

// ---------------------------------------------------------------------------
// Take-id allocation and the audio/MIDI capture split (todo #409, doc #292)
// ---------------------------------------------------------------------------

/// A take id must be unique within its group. Deriving it from
/// `pass_index` was only sound while a `(group, pass)` pair could emit at
/// most once — and it cannot be relied on to: `finalize_loop_record_pass`
/// runs an audio loop and a MIDI loop that resolve to the *same* group for
/// the same track, so a track present in both emits twice at one
/// `pass_index`.
///
/// This drives the real allocator with exactly that shape. Under the old
/// `take_id = pass_index` rule both takes below would come back as `0`.
#[test]
fn two_captures_in_one_pass_get_distinct_take_ids() {
    use resonance_audio::__test_support::push_take;
    use resonance_common::{TakeContent, TakeGroup, TimelineRange};

    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(1, 7, slot);

    // Pass 0 emits twice for the same track: the audio roll, then the MIDI
    // capture — the exact sequence `finalize_loop_record_pass` produces.
    let audio_id = push_take(&mut group, slot, 0, slot, &TakeContent::Audio { clip_ref: 100 });
    let midi_id = push_take(
        &mut group,
        slot,
        0,
        slot,
        &TakeContent::Midi { notes: Vec::new() },
    );

    assert_ne!(
        audio_id, midi_id,
        "two takes captured in one pass must not share an id"
    );
    assert_eq!(group.takes.len(), 2, "both takes must be retained");
    assert!(group.take(audio_id).is_some());
    assert!(group.take(midi_id).is_some());
}

/// Ids stay unique — and resolvable — across several passes, including
/// when a pass emits twice. The group is the id's scope, so the sequence
/// is dense and monotonic regardless of how the passes broke down.
#[test]
fn take_ids_stay_unique_across_passes() {
    use resonance_audio::__test_support::push_take;
    use resonance_common::{TakeContent, TakeGroup, TimelineRange};

    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(1, 7, slot);

    let mut ids = Vec::new();
    for pass in 0..3u32 {
        ids.push(push_take(
            &mut group,
            slot,
            pass,
            slot,
            &TakeContent::Audio {
                clip_ref: 100 + u64::from(pass),
            },
        ));
        // Pass 1 also yields a MIDI take, as a dual-armed track would.
        if pass == 1 {
            ids.push(push_take(
                &mut group,
                slot,
                pass,
                slot,
                &TakeContent::Midi { notes: Vec::new() },
            ));
        }
    }

    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "take ids must all be distinct: {ids:?}");
    assert_eq!(sorted, vec![0, 1, 2, 3], "ids should be dense and monotonic");
    for id in ids {
        assert!(group.take(id).is_some(), "take {id} must be resolvable");
    }
}

/// The capture-side predicate: which armed tracks get an audio recording
/// buffer. A track whose instrument runs in-process records a MIDI
/// performance, not audio — giving it a buffer wrote a junk WAV per pass
/// and filed a spurious second take (ba doc #292).
#[test]
fn only_tracks_without_an_in_process_instrument_capture_audio() {
    use resonance_audio::types::{Track, TrackType};

    let audio = Track::new(1, "audio".into());
    assert!(
        !audio.runs_internal_instrument(),
        "an audio track captures audio"
    );

    let instrument = Track::with_type(2, "synth".into(), TrackType::Instrument);
    assert!(
        instrument.runs_internal_instrument(),
        "an in-process instrument track must NOT open an audio recording buffer"
    );
}

/// External-instrument tracks (epic #39) are `Instrument`-typed and accept
/// MIDI, but their synth is outboard: the audio comes back on the return
/// input and recording it is the entire point of the feature. Excluding
/// them — as a blanket `accepts_midi()` test would — would break it, so
/// pin the distinction rather than leaving it to the predicate's shape.
#[test]
fn external_instrument_tracks_still_capture_their_audio_return() {
    use resonance_audio::types::{Track, TrackType};

    let external = Track::with_type(3, "outboard".into(), TrackType::Instrument);
    external.set_external(true);

    assert!(external.track_type.accepts_midi(), "precondition: accepts MIDI");
    assert!(
        !external.runs_internal_instrument(),
        "an external instrument's audio return must still be recorded"
    );
}

/// `Vocal` tracks also accept MIDI, but they render through the audio path
/// rather than an in-process instrument, so they keep audio capture too.
#[test]
fn vocal_tracks_still_capture_audio() {
    use resonance_audio::types::{Track, TrackType};

    let vocal = Track::with_type(4, "vox".into(), TrackType::Vocal);
    assert!(vocal.track_type.accepts_midi(), "precondition: accepts MIDI");
    assert!(
        !vocal.runs_internal_instrument(),
        "a vocal track renders through the audio path and keeps audio capture"
    );
}

// ---------------------------------------------------------------------------
// Rehydrating the store from a saved project (todo #1394, doc #292)
// ---------------------------------------------------------------------------
//
// `store_take` was the only writer of `HandlerState::take_groups`, so a
// project load left the engine holding no groups at all: the app drew the
// lanes and the engine rendered silence. These pin the store + allocator
// contract of the restore, which is the pure half of the new
// `AudioCommand::RestoreTakeGroups` handler.

use std::collections::HashMap;

use resonance_audio::__test_support::{push_take, restore_take_groups_in_place};
use resonance_common::{Comp, CompSegment, TakeContent, TakeGroup, TakeGroupId, TimelineRange};

/// A saved group with `takes` audio takes and a comp that names the last
/// of them — the shape a comped cycle-record run persists.
fn saved_group(id: TakeGroupId, track_id: u64, takes: u64) -> TakeGroup {
    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(id, track_id, slot);
    for i in 0..takes {
        push_take(
            &mut group,
            slot,
            i as u32,
            slot,
            &TakeContent::Audio {
                clip_ref: id * 100 + i,
            },
        );
    }
    group.comp = Comp {
        segments: vec![CompSegment {
            range: slot,
            take_id: takes - 1,
        }],
    };
    group
}

/// Every saved group lands in the engine's store, keyed by its own id and
/// carrying its comp verbatim. Without this the published comp table is
/// empty and a loaded comp neither plays nor bounces.
#[test]
fn restoring_seeds_every_saved_group_with_its_comp() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 1u64;

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 3), saved_group(2, 8, 2)],
    );

    assert_eq!(store.len(), 2, "a project's groups all come back");
    let g1 = store.get(&1).expect("group 1 keyed by its own id");
    assert_eq!(g1.track_id, 7);
    assert_eq!(g1.takes.len(), 3, "no take is dropped on the way in");
    assert_eq!(
        g1.comp.segments.iter().map(|s| s.take_id).collect::<Vec<_>>(),
        vec![2],
        "the comp is what makes the group audible; it must survive"
    );
    assert_eq!(store.get(&2).expect("group 2").takes.len(), 2);
}

/// The allocator is pushed above every restored id, so the next
/// cycle-record run cannot re-issue a group a loaded project already
/// holds. It re-issued `1` before this landed, and because take ids are
/// allocated *within* a group the new run's first take then took id `0` —
/// silently replacing a restored take in the app's `(group, take)`-keyed
/// mirror.
#[test]
fn restoring_reserves_group_ids_past_the_highest_saved_one() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 1u64;

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 2), saved_group(9, 8, 1), saved_group(4, 9, 1)],
    );

    assert_eq!(
        next, 10,
        "the next group id must clear the highest restored id, not merely the last"
    );
}

/// The bump only ever raises. A project holding one low-numbered group,
/// loaded into a session that had already recorded several, must not drag
/// the allocator back down onto ids that session has handed out.
#[test]
fn restoring_never_lowers_the_group_allocator() {
    let mut store = HashMap::new();
    let mut next = 12u64;
    let mut next_clip = 1u64;

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 1)],
    );

    assert_eq!(next, 12, "a low restored id must not rewind the allocator");
}

/// Restoring replaces; it never merges. Both senders — a disk load and an
/// undo/redo diff replay — rebuild the app-side mirror from scratch first,
/// and the undo path sends no `ClearAll`, so merging would resurrect the
/// takes an undo had just deleted.
#[test]
fn restoring_replaces_the_previous_projects_groups() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 1u64;

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 3)],
    );
    // Project B holds one group, with a different id and a different track.
    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(5, 8, 1)],
    );

    assert_eq!(store.len(), 1, "project A's group must not linger");
    assert!(store.contains_key(&5));
    assert!(
        !store.contains_key(&1),
        "a stale group governs clip ids the new project reuses"
    );
}

/// An empty project still clears the store. The send is unconditional for
/// exactly this: opening a project with no take lanes on top of a comped
/// one otherwise left the old comp governing — and playing over — clip ids
/// the new project had reused.
#[test]
fn restoring_an_empty_project_clears_the_store() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 1u64;

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 2)],
    );
    restore_take_groups_in_place(&mut store, &mut next, &mut next_clip, Vec::new());

    assert!(store.is_empty(), "a take-lane-free project must empty the store");
    assert_eq!(next, 2, "clearing the store does not rewind the allocator");
}

/// Take ids need no reservation of their own — `push_take` allocates from
/// the group it is given, so a further take on a *restored* group picks up
/// where the saved takes left off. Confirming, rather than assuming, the
/// claim doc #292 makes about the allocator surviving a rehydration.
#[test]
fn a_further_take_on_a_restored_group_gets_a_fresh_id() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 1u64;
    let slot = TimelineRange::new(0, 48_000);

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 3)],
    );

    let group = store.get_mut(&1).expect("restored group");
    let saved_ids: Vec<_> = group.takes.iter().map(|t| t.id).collect();
    assert_eq!(saved_ids, vec![0, 1, 2], "precondition: three saved takes");

    let fresh = push_take(group, slot, 3, slot, &TakeContent::Audio { clip_ref: 999 });

    assert_eq!(fresh, 3, "the id continues the restored group's own sequence");
    assert_eq!(group.takes.len(), 4, "no restored take is overwritten");
    for id in saved_ids {
        assert!(group.take(id).is_some(), "restored take {id} must survive");
    }
}

// ---------------------------------------------------------------------------
// Reserving clip ids past a loaded project's take clip_refs (todo #1393)
// ---------------------------------------------------------------------------
//
// An audio take owns `audio/clip_{clip_ref}.wav` just as much as a timeline
// clip does, but nothing reserved the id: `ClearAll` resets `next_clip_id`
// to 1 and only `LoadClipFromWav` (and its MIDI twin) pushes it back up —
// paths a take clip never takes, since `roll_audio_pass` hands the take
// straight to `store_take`. The restore is where the engine learns a loaded
// project's `clip_ref`s, so it is where they get reserved.
//
// These assert the *allocator*, not that a recording succeeds: the bug is
// invisible until a WAV is clobbered, and a recording that overwrites a
// take's file succeeds just fine.

/// A saved group of `takes` MIDI takes — the shape an instrument track's
/// cycle-record run persists. Notes inline, no clip and no file.
fn saved_midi_group(id: TakeGroupId, track_id: u64, takes: u64) -> TakeGroup {
    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(id, track_id, slot);
    for i in 0..takes {
        push_take(
            &mut group,
            slot,
            i as u32,
            slot,
            &TakeContent::Midi {
                notes: vec![resonance_common::TakeNote {
                    note: 60 + i as u8,
                    velocity: 0.8,
                    start_tick: 0,
                    duration_ticks: 480,
                }],
            },
        );
    }
    group
}

/// The reported bug, in allocator terms: a project whose timeline clips
/// stop at 5 but whose takes hold `clip_ref` 100..102 must not reopen with
/// `next_clip_id = 6`. It did, and the next recording or import wrote
/// `audio/clip_100.wav` — the file a restored take was still playing.
#[test]
fn restoring_reserves_clip_ids_past_the_highest_take_clip_ref() {
    let mut store = HashMap::new();
    let mut next = 1u64;

    // Where a load leaves the clip allocator before the takes arrive:
    // `ClearAll` reset it to 1 and the timeline clips (ids 1..=5) each
    // bumped it through `LoadClipFromWav`.
    let mut next_clip = 6u64;

    // `saved_group` numbers its takes' clips `id * 100 + pass`, so group 1
    // holds 100..102 — well past anything on the timeline.
    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 3)],
    );

    assert_eq!(
        next_clip, 103,
        "the clip allocator must clear every restored take's clip_ref"
    );

    // Spelled out as the acceptance case: the id the engine hands the next
    // recording (`state.next_clip_id`, then `+= 1`) collides with no take.
    let issued = next_clip;
    let held: Vec<u64> = store
        .values()
        .flat_map(|g| g.takes.iter())
        .filter_map(|t| match t.content {
            TakeContent::Audio { clip_ref } => Some(clip_ref),
            TakeContent::Midi { .. } => None,
        })
        .collect();
    assert_eq!(held, vec![100, 101, 102], "precondition: high take clip_refs");
    assert!(
        !held.contains(&issued),
        "the next clip id ({issued}) must not name a WAV a take still plays"
    );
}

/// The bump clears the highest `clip_ref` anywhere in the project, not
/// merely the last group's or the last take's — the groups arrive in
/// whatever order the file lists them.
#[test]
fn restoring_reserves_past_the_highest_clip_ref_in_any_group() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 1u64;

    // Group 9 holds the highest clip_refs (900, 901) but is not last.
    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 2), saved_group(9, 8, 2), saved_group(4, 9, 1)],
    );

    assert_eq!(
        next_clip, 902,
        "reserving from the last group visited leaves earlier groups' clips exposed"
    );
}

/// The clip bump only ever raises, exactly like the group bump. A
/// low-numbered project loaded into a session that has already recorded
/// must not drag the allocator back onto ids that session issued — those
/// WAVs are on disk and referenced too.
#[test]
fn restoring_never_lowers_the_clip_allocator() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 500u64;

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 3)],
    );

    assert_eq!(
        next_clip, 500,
        "a restored clip_ref below the allocator must not rewind it"
    );
}

/// A take-lane-free project (and File > New) leaves the allocator where it
/// was; the restore reserves what it is given and invents nothing.
#[test]
fn restoring_an_empty_project_leaves_the_clip_allocator_alone() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 6u64;

    restore_take_groups_in_place(&mut store, &mut next, &mut next_clip, Vec::new());

    assert_eq!(next_clip, 6, "nothing restored, nothing to reserve");
}

/// **MIDI takes need no reservation.** `TakeContent::Midi` carries its
/// notes inline — it names no clip and writes no WAV, and the MIDI half of
/// `finalize_loop_record_pass` never touches `next_clip_id`. Pinned rather
/// than assumed, because the audio and MIDI capture paths sit side by side
/// and only one of them owns a file.
#[test]
fn a_midi_only_project_reserves_no_clip_ids() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 6u64;

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_midi_group(1, 7, 3), saved_midi_group(2, 8, 2)],
    );

    assert_eq!(store.len(), 2, "precondition: the MIDI groups did restore");
    assert_eq!(
        next_clip, 6,
        "a MIDI take owns no clip id, so it must not consume one"
    );
    assert_eq!(next, 3, "group ids are still reserved for MIDI lanes");
}

/// A track armed for both — or a project mixing audio and instrument lanes
/// — reserves off the audio takes and steps over the MIDI ones without
/// tripping.
#[test]
fn a_mixed_project_reserves_off_its_audio_takes_only() {
    let mut store = HashMap::new();
    let mut next = 1u64;
    let mut next_clip = 1u64;

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_midi_group(1, 7, 2), saved_group(3, 8, 2)],
    );

    assert_eq!(
        next_clip, 302,
        "the audio group's clip_refs (300, 301) still have to be cleared"
    );
}

// ---------------------------------------------------------------------------
// `ClearAll` resets the take-group state (todo #1394, covered by #1399)
// ---------------------------------------------------------------------------
//
// `handle_clear_all` cleared eight tables and five id allocators but left
// `take_groups`, `next_take_group_id` and the published comp table
// untouched — while the app's `wipe_registry` had asserted since #412 that
// it did all three. The published table is what `render/clips.rs` consults
// through `is_governed`, so a project opened on top of a comped one lost
// the clip ids it reused to the *previous* project's comp: those clips
// vanished from the ordinary clip path and the stale comp played instead.
//
// The three effects are asserted separately and each is reachable on its
// own, so dropping any one line fails at least one case here. That matters
// because the indirect coverage — every `ClearAll` sender follows up with
// an unconditional `RestoreTakeGroups`, which would paper over all three —
// is exactly what let the app-side assertion go unverified for months.
//
// This runs the real handler, not an extracted pure half of it, via the
// headless `EngineHandlerHarness` over the engine thread's own
// `HandlerCtx` + `HandlerState`.

use resonance_audio::__test_support::EngineHandlerHarness;

/// A harness holding one comped three-take group (id 1, clips 100..102),
/// with the comp table published — where a cycle-record run or a project
/// load leaves the engine.
fn harness_with_a_comped_group() -> EngineHandlerHarness {
    let mut h = EngineHandlerHarness::new();
    h.seed_take_group(saved_group(1, 7, 3));
    // As capturing group 1 would have left the allocator.
    h.set_next_take_group_id(2);

    assert_eq!(h.take_group_ids(), vec![1], "precondition: the group is stored");
    assert!(
        h.published_comp_table().is_governed(102),
        "precondition: the comp governs the take's clip"
    );
    h
}

/// The store itself: `ClearAll` must empty it. A lingering group is what
/// governs — and silences — the next project's reused clip ids.
#[test]
fn clear_all_empties_the_take_group_store() {
    let mut h = harness_with_a_comped_group();

    h.clear_all();

    assert!(
        h.take_group_ids().is_empty(),
        "the previous project's take groups must not survive a ClearAll"
    );
}

/// The allocator: `ClearAll` must reset it to 1, like every other id
/// counter it resets. Left high, a `File > New` session numbers its first
/// take group after a project that is no longer open, and the app-side
/// mirror — keyed by `(group, take)` — is built expecting a fresh start.
#[test]
fn clear_all_resets_the_take_group_allocator() {
    let mut h = harness_with_a_comped_group();
    assert_eq!(h.next_take_group_id(), 2, "precondition: the allocator moved");

    h.clear_all();

    assert_eq!(
        h.next_take_group_id(),
        1,
        "the take-group allocator must restart with the other id counters"
    );
}

/// The publish: emptying the control-thread store is not enough on its
/// own. `SharedState::take_comp` is the copy the audio callback and the
/// offline bounce read, and until it is republished the cleared project's
/// comp is still the one governing clip ids and still the one that plays.
#[test]
fn clear_all_publishes_the_now_empty_comp_table() {
    let mut h = harness_with_a_comped_group();

    h.clear_all();

    let table = h.published_comp_table();
    assert!(
        table.is_empty(),
        "the audio thread still reads the cleared project's comp table"
    );
    assert!(
        !table.is_governed(102),
        "a stale governed clip id hides the new project's clip 102 from the clip path"
    );
    assert!(
        table.track_comp(7).is_none(),
        "the cleared project's comp would still render over track 7"
    );
}

/// End to end, in the shape the bug took: comp a project, `ClearAll`, then
/// open a project that reuses clip id 102 on a different track. Nothing of
/// the first project may govern or play.
#[test]
fn a_project_loaded_after_clear_all_keeps_its_reused_clip_ids() {
    let mut h = harness_with_a_comped_group();

    h.clear_all();

    // Project B: one group on another track whose takes happen to reuse
    // clip ids the first project's comp governed.
    let mut group_b = TakeGroup::new(4, 9, TimelineRange::new(0, 48_000));
    push_take(
        &mut group_b,
        TimelineRange::new(0, 48_000),
        0,
        TimelineRange::new(0, 48_000),
        &TakeContent::Audio { clip_ref: 102 },
    );
    h.seed_take_group(group_b);

    let table = h.published_comp_table();
    assert!(
        table.track_comp(7).is_none(),
        "project A's track still has a comp after its project was closed"
    );
    let spans = &table
        .track_comp(9)
        .expect("project B's group must play on its own track")
        .spans;
    assert_eq!(
        spans.iter().map(|s| s.clip_id).collect::<Vec<_>>(),
        vec![102],
        "clip 102 must play as project B's take, not project A's"
    );
}
