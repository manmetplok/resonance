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
    let (take_id, _) = push_take(
        &mut group,
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
    let (take_id, _) = push_take(
        &mut group,
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
    let (audio_id, _) = push_take(&mut group, slot, &TakeContent::Audio { clip_ref: 100 });
    let (midi_id, _) = push_take(&mut group, slot, &TakeContent::Midi { notes: Vec::new() });

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
        ids.push(
            push_take(
                &mut group,
                slot,
                &TakeContent::Audio {
                    clip_ref: 100 + u64::from(pass),
                },
            )
            .0,
        );
        // Pass 1 also yields a MIDI take, as a dual-armed track would.
        if pass == 1 {
            ids.push(push_take(&mut group, slot, &TakeContent::Midi { notes: Vec::new() }).0);
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
// `store_take_in` was the only writer of `HandlerState::take_groups`, so a
// project load left the engine holding no groups at all: the app drew the
// lanes and the engine rendered silence. These pin the store + allocator
// contract of the restore, which is the pure half of the new
// `AudioCommand::RestoreTakeGroups` handler.

use std::collections::HashMap;

use resonance_audio::__test_support::{
    build_comp_table, capture_take_event, push_take, resolve_take_group,
    restore_take_groups_in_place, slots_match, store_take_in, take_group_for_slot, TakeGroupStore,
    SAME_SLOT_TOLERANCE_FRAMES,
};
use resonance_audio::types::AudioEvent;
use resonance_common::{
    Comp, CompSegment, TakeContent, TakeGroup, TakeGroupId, TakeId, TimelineRange,
};

/// A saved group with `takes` audio takes and a comp that names the last
/// of them — the shape a comped cycle-record run persists.
fn saved_group(id: TakeGroupId, track_id: u64, takes: u64) -> TakeGroup {
    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(id, track_id, slot);
    for i in 0..takes {
        push_take(
            &mut group,
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

    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 3)],
    );

    let group = store.get_mut(&1).expect("restored group");
    let saved_ids: Vec<_> = group.takes.iter().map(|t| t.id).collect();
    assert_eq!(saved_ids, vec![0, 1, 2], "precondition: three saved takes");

    let slot = TimelineRange::new(0, 48_000);
    let (fresh, ordinal) = push_take(group, slot, &TakeContent::Audio { clip_ref: 999 });

    assert_eq!(fresh, 3, "the id continues the restored group's own sequence");
    assert_eq!(
        ordinal, 3,
        "so does the ordinal the lane stacks and labels by"
    );
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
// straight to `store_take_in`. The restore is where the engine learns a loaded
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

// ---------------------------------------------------------------------------
// One lane per slot: reusing the take group across record runs
// (todo #1392, doc #292)
// ---------------------------------------------------------------------------
//
// The ruling: takes accumulate into a single `TakeGroup` for a given track
// + loop region however many times record is pressed. Three passes, stop,
// two more over the same region is one lane of five takes — Logic /
// Pro Tools / Reaper take-folder behaviour — not two lanes.
//
// These drive `capture_take_event`, which is the *entire* engine-side
// capture glue: `finalize_loop_record_pass` calls it once per rolled audio
// take and once per captured MIDI take and does nothing with the result
// but send it. So asserting on the `AudioEvent` it returns is asserting on
// what the transport actually emits — the group id, the take id, the slot
// and the ordinal all have to come from the *group* rather than the run,
// and there is no second copy of that assembly to drift out of step.
//
// What a *stop* does between two runs is tear down the
// `LoopRecordSession`. These reproduce that by simply starting the next
// run: the session carried the pass counter and the group cache, and
// neither is consulted here any more.

const TRACK: u64 = 7;
const OTHER_TRACK: u64 = 8;
/// The loop region both runs cycle over: one second starting at 2 s.
const SLOT: TimelineRange = TimelineRange {
    start: 96_000,
    length: 48_000,
};

/// What a captured take reports back, unpacked from the `TakeCaptured`
/// event `capture_take_event` builds. Panics on any other event, so a
/// capture that stopped announcing itself fails loudly.
#[derive(Debug, Clone, PartialEq)]
struct Captured {
    group_id: TakeGroupId,
    take_id: TakeId,
    track_id: u64,
    slot: TimelineRange,
    pass_index: u32,
    extent: TimelineRange,
    content: TakeContent,
}

/// Replay one captured pass through the real transport glue, with an
/// explicit `extent` — what the pass really recorded (todo #1396), which
/// is a property of the run and not of the lane it lands in.
fn record_pass_over(
    store: &mut TakeGroupStore,
    next_group_id: &mut TakeGroupId,
    track_id: u64,
    run_slot: TimelineRange,
    extent: TimelineRange,
    content: TakeContent,
) -> Captured {
    match capture_take_event(store, next_group_id, track_id, run_slot, extent, content) {
        AudioEvent::TakeCaptured {
            group_id,
            take_id,
            track_id,
            slot,
            pass_index,
            extent,
            content,
        } => Captured {
            group_id,
            take_id,
            track_id,
            slot,
            pass_index,
            extent,
            content,
        },
        other => panic!("a capture must announce itself as TakeCaptured, got {other:?}"),
    }
}

/// Replay one captured pass that filled the region it cycled over.
fn record_pass(
    store: &mut TakeGroupStore,
    next_group_id: &mut TakeGroupId,
    track_id: u64,
    run_slot: TimelineRange,
    content: TakeContent,
) -> Captured {
    record_pass_over(store, next_group_id, track_id, run_slot, run_slot, content)
}

/// An audio take's content, tagged by clip id so a later assertion can say
/// which pass of which run it came from.
fn audio(clip_ref: u64) -> TakeContent {
    TakeContent::Audio { clip_ref }
}

/// Record `passes` passes over `run_slot` as one record run.
/// `clip_base` tags the run's clips.
fn record_run(
    store: &mut TakeGroupStore,
    next_group_id: &mut TakeGroupId,
    track_id: u64,
    run_slot: TimelineRange,
    passes: u32,
    clip_base: u64,
) -> Vec<Captured> {
    (0..passes)
        .map(|pass| {
            record_pass(
                store,
                next_group_id,
                track_id,
                run_slot,
                audio(clip_base + u64::from(pass)),
            )
        })
        .collect()
}

/// The acceptance case. Three passes, stop, two more over the same loop
/// region on the same track: **one** group holding five takes with five
/// distinct ids, and no take lost.
///
/// The ordinals matter as much as the ids. The *run's* pass counter
/// restarts at 0 at every record press, and the app both sorts a lane by
/// `(pass_index, id)` and labels each take `T{pass_index + 1}` — so
/// emitting the run's counter would stack this lane `0,3,1,4,2` and label
/// it `T1, T1, T2, T2, T3`. The ordinal is allocated from the group
/// instead, so the lane reads `T1..T5` in capture order.
#[test]
fn two_runs_over_one_region_make_one_lane_of_five_takes() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;

    let run1 = record_run(&mut store, &mut next, TRACK, SLOT, 3, 100);
    // -- transport stops here; the loop-record session is gone --
    let run2 = record_run(&mut store, &mut next, TRACK, SLOT, 2, 200);
    let all: Vec<_> = run1.iter().chain(&run2).collect();

    assert_eq!(
        store.len(),
        1,
        "the second run must join the lane, not open a second one"
    );
    let group_ids: Vec<_> = all.iter().map(|c| c.group_id).collect();
    assert!(
        group_ids.windows(2).all(|w| w[0] == w[1]),
        "every take of both runs belongs to one group: {group_ids:?}"
    );
    assert_eq!(next, 2, "only one group id may be consumed by two runs");

    let group = store.values().next().expect("the one lane");
    assert_eq!(group.takes.len(), 5, "no take may be lost by the reuse");

    assert_eq!(
        all.iter().map(|c| c.take_id).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4],
        "five distinct ids, the second run continuing the first's sequence"
    );
    assert_eq!(
        all.iter().map(|c| c.pass_index).collect::<Vec<_>>(),
        vec![0, 1, 2, 3, 4],
        "the lane must stack and label T1..T5 in capture order, not restart at the second run"
    );
    assert_eq!(
        group
            .takes
            .iter()
            .map(|t| (t.id, t.pass_index))
            .collect::<Vec<_>>(),
        vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4)],
        "the stored takes must agree with what was announced"
    );
    assert_eq!(
        group
            .takes
            .iter()
            .map(|t| match t.content {
                TakeContent::Audio { clip_ref } => clip_ref,
                _ => unreachable!("audio takes only"),
            })
            .collect::<Vec<_>>(),
        vec![100, 101, 102, 200, 201],
        "both runs' recordings are present, in capture order"
    );
}

/// A run over a *materially* different region gets its own lane — and this
/// is the case the matching rule exists to get right, not merely a
/// symmetry check.
///
/// A rule loose enough to accept 4 bars against 7 would fold this take
/// into the 4-bar lane, which never grows: `build_comp_table` resolves
/// spans against `group.slot`, so the last three bars would be covered by
/// nothing, and the clip is governed, so it could not play on the ordinary
/// clip path either. The take would be silently lost. Here it keeps its
/// own lane and its own full-length span.
#[test]
fn a_longer_loop_gets_its_own_lane_and_plays_in_full() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;
    let four_bars = TimelineRange::new(0, 4 * 96_000);
    let seven_bars = TimelineRange::new(0, 7 * 96_000);

    let first = record_run(&mut store, &mut next, TRACK, four_bars, 2, 100);
    let longer = record_pass(&mut store, &mut next, TRACK, seven_bars, audio(200));

    assert_ne!(
        longer.group_id, first[0].group_id,
        "a region half again as long is not the same slot"
    );
    assert_eq!(longer.slot, seven_bars, "the new lane spans what was recorded");
    assert_eq!(longer.pass_index, 0, "a new lane starts its labels at T1");

    let table = build_comp_table(&store);
    let spans = &table.track_comp(TRACK).expect("track has a comp").spans;
    assert!(
        spans
            .iter()
            .any(|s| s.clip_id == 200 && s.range == seven_bars),
        "the longer take must be audible over its whole region, not clipped \
         to a shorter lane's slot: {spans:?}"
    );
}

/// A run over a different region on the same track still gets its own
/// lane, and the reuse rule must not collapse the user's sections.
#[test]
fn a_run_over_a_different_region_gets_its_own_lane() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;

    let first = record_run(&mut store, &mut next, TRACK, SLOT, 2, 100);
    let elsewhere = TimelineRange::new(SLOT.end() + 48_000, SLOT.length);
    let second = record_run(&mut store, &mut next, TRACK, elsewhere, 1, 200);

    assert_eq!(store.len(), 2, "two regions, two lanes");
    assert_ne!(
        first[0].group_id, second[0].group_id,
        "the lanes must not share a group"
    );
    assert_eq!(
        second[0].slot, elsewhere,
        "the new lane is bound to the region it was recorded over"
    );
}

/// Same region, different track: still its own lane. Groups are per
/// (track, slot), and two armed tracks in one run each keep their own.
#[test]
fn a_run_on_another_track_over_the_same_region_gets_its_own_lane() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;

    let a = record_run(&mut store, &mut next, TRACK, SLOT, 1, 100);
    let b = record_run(&mut store, &mut next, OTHER_TRACK, SLOT, 1, 200);

    assert_eq!(store.len(), 2, "one lane per track");
    assert_ne!(a[0].group_id, b[0].group_id);
}

/// The MIDI half of a pass resolves to the group the audio half just
/// created. `finalize_loop_record_pass` runs an audio loop and a MIDI loop
/// and both call the glue for the same track; with the per-run cache gone,
/// the store lookup is what keeps them together — and the two takes still
/// get distinct ids and distinct ordinals.
#[test]
fn the_midi_half_of_a_pass_joins_the_group_the_audio_half_created() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;

    let a = record_pass(&mut store, &mut next, TRACK, SLOT, audio(1));
    let m = record_pass(
        &mut store,
        &mut next,
        TRACK,
        SLOT,
        TakeContent::Midi { notes: Vec::new() },
    );

    assert_eq!(a.group_id, m.group_id, "one pass, one group");
    assert_ne!(a.take_id, m.take_id, "two captures, two ids");
    assert_ne!(a.pass_index, m.pass_index, "two captures, two lane rows");
    assert_eq!(store.len(), 1);
    assert_eq!(next, 2, "the MIDI half must not burn a second group id");
}

/// A comp drawn after the first run still resolves once the second run has
/// added takes to the same lane — and a segment promoted from a *second*
/// run take renders alongside one from the first. This is the "the comp
/// still resolves correctly across takes from both runs" half of the
/// acceptance, asserted through the real `build_comp_table`.
#[test]
fn a_comp_resolves_across_takes_from_both_runs() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;

    let run1 = record_run(&mut store, &mut next, TRACK, SLOT, 3, 100);
    let group_id = run1[0].group_id;
    let mid = SLOT.start + SLOT.length / 2;

    // Comp the first run: first half from take 0, second half from take 2.
    store.get_mut(&group_id).unwrap().comp = Comp {
        segments: vec![
            CompSegment {
                range: TimelineRange::from_bounds(SLOT.start, mid),
                take_id: run1[0].take_id,
            },
            CompSegment {
                range: TimelineRange::from_bounds(mid, SLOT.end()),
                take_id: run1[2].take_id,
            },
        ],
    };

    let run2 = record_run(&mut store, &mut next, TRACK, SLOT, 2, 200);

    // The comp survives the second run untouched.
    let group = &store[&group_id];
    assert!(
        group.comp.is_full_cover(group.slot),
        "the existing comp must still cover the lane's slot"
    );
    let table = build_comp_table(&store);
    let spans = &table.track_comp(TRACK).expect("track has a comp").spans;
    assert_eq!(
        spans.iter().map(|s| s.clip_id).collect::<Vec<_>>(),
        vec![100, 102],
        "the comp still names the first run's takes"
    );

    // Promote the second half to a take from the *second* run.
    store.get_mut(&group_id).unwrap().comp.segments[1].take_id = run2[0].take_id;
    let table = build_comp_table(&store);
    let spans = &table.track_comp(TRACK).expect("track has a comp").spans;
    assert_eq!(
        spans.iter().map(|s| s.clip_id).collect::<Vec<_>>(),
        vec![100, 200],
        "a promotion may cross runs once they share a lane"
    );
    for clip in [100u64, 101, 102, 200, 201] {
        assert!(
            table.is_governed(clip),
            "every take clip in the lane must stay off the raw clip path ({clip})"
        );
    }
}

/// A loop region jittered by a frame joins the same lane **and leaves its
/// slot where it was**. The lane's region is fixed by the run that created
/// it; a rebind would drag it onto the new region and leave every
/// already-drawn `CompSegment` describing positions outside their own slot.
#[test]
fn a_jittered_region_joins_the_lane_without_moving_it() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;

    let run1 = record_run(&mut store, &mut next, TRACK, SLOT, 2, 100);
    let group_id = run1[0].group_id;
    store.get_mut(&group_id).unwrap().comp = Comp {
        segments: vec![CompSegment {
            range: SLOT,
            take_id: run1[1].take_id,
        }],
    };

    // The loop end lands a single frame out from where it was.
    let jittered = TimelineRange::from_bounds(SLOT.start, SLOT.end() + 1);
    let joined = record_pass(&mut store, &mut next, TRACK, jittered, audio(200));

    assert_eq!(joined.group_id, group_id, "a one-frame drift is the same slot");
    assert_eq!(
        joined.slot, SLOT,
        "the take is announced against the lane's own region, not the jittered one"
    );
    let group = &store[&group_id];
    assert_eq!(group.slot, SLOT, "the lane's region must not move");
    assert!(
        group.comp.is_full_cover(group.slot),
        "the comp drawn against the old region must stay valid"
    );
    assert!(
        group
            .comp
            .segments
            .iter()
            .all(|s| s.range.start >= group.slot.start && s.range.end() <= group.slot.end()),
        "no comp segment may end up outside its own slot"
    );
}

/// What a within-tolerance join actually costs, measured rather than
/// asserted away. This is the load-bearing half of why the tolerance is
/// safe to have at all (see `slots_match`): a joining run records its own
/// `extent`, `Take::audible_extent` intersects that with the lane's slot,
/// and the surplus — bounded by the tolerance, at each edge — is simply
/// not covered. That is the same state a punch-in pass or a pass cut off
/// at stop already produces, by far more frames, and #1396 made it visible
/// rather than assumed.
#[test]
fn a_within_tolerance_join_strands_at_most_the_tolerance() {
    const TOL: u64 = SAME_SLOT_TOLERANCE_FRAMES;
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;

    record_run(&mut store, &mut next, TRACK, SLOT, 1, 100);
    // The loop drifts out by the full tolerance at both ends, and the run
    // records every frame of the region it cycled over.
    let drifted = TimelineRange::from_bounds(SLOT.start - TOL, SLOT.end() + TOL);
    let joined = record_pass_over(&mut store, &mut next, TRACK, drifted, drifted, audio(200));

    let group = &store[&joined.group_id];
    assert_eq!(group.slot, SLOT, "precondition: it joined and did not move the lane");
    assert_eq!(
        joined.extent, drifted,
        "the take reports what it recorded, not what the lane covers"
    );

    let take = group.take(joined.take_id).expect("the joined take");
    let audible = take.audible_extent(group.slot);
    assert_eq!(
        audible, SLOT,
        "the surplus is clamped to the lane rather than mis-placed"
    );
    let stranded = (SLOT.start - drifted.start) + (drifted.end() - SLOT.end());
    assert_eq!(stranded, 2 * TOL, "512 frames, ~10.7 ms across both edges");
    assert!(
        stranded <= 2 * SAME_SLOT_TOLERANCE_FRAMES,
        "and it can never exceed one tolerance per edge — that bound is \
         the whole reason an absolute tolerance was chosen over a ratio"
    );
}

/// **The `slot` / `extent` pair, pinned by argument position.**
///
/// `store_take_in(store, group_id, track_id, slot, extent, &content)` takes
/// two adjacent `TimelineRange`s that mean opposite things: `slot` binds a
/// *newly created* lane, `extent` records what the *pass* captured. Every
/// other case in this file hands them the same value — a run that filled
/// the region it cycled over — so transposing them at the
/// `capture_take_event` call site passed the whole suite. The two punch-in
/// cases above miss it because they drive `push_take` directly and never
/// reach `store_take_in`.
///
/// Transposed, a lane opened by a punched-in pass would bind to the
/// punch-in *extent* while the take stored the full slot as its extent —
/// the reload-side twin of the bug #1396 fixed, since both values persist:
/// the lane would be narrower than the region it was recorded over, and
/// `audible_extent` would claim material the pass never captured.
///
/// The assertions are on the **store**, deliberately. `TakeCaptured` builds
/// its `slot` and `extent` fields from the caller's own locals, so the
/// event looks perfectly correct under the transposition and only the
/// stored group and take disagree.
#[test]
fn a_new_lane_binds_to_the_run_region_while_the_take_stores_its_own_extent() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;

    // Punched in a quarter of a second late and cut off an eighth early:
    // strictly inside the run's region, and asymmetric so no swap can
    // coincide.
    let recorded = TimelineRange::from_bounds(SLOT.start + 12_000, SLOT.end() - 6_000);
    assert_ne!(recorded, SLOT, "the fixture must be able to tell them apart");

    let cap = record_pass_over(&mut store, &mut next, TRACK, SLOT, recorded, audio(100));

    let group = &store[&cap.group_id];
    assert_eq!(
        group.slot, SLOT,
        "a new lane binds to the region the run cycled over, never to what \
         one pass happened to capture inside it"
    );
    let take = group.take(cap.take_id).expect("the stored take");
    assert_eq!(
        take.extent, recorded,
        "the take stores what the pass recorded, never its lane's slot"
    );

    // The echo has to agree with the store, or the app mirrors a lane the
    // engine does not have.
    assert_eq!(cap.slot, group.slot);
    assert_eq!(cap.extent, take.extent);

    // And the pair still resolves the way #1396 specified: the audible
    // stretch is the recording, not the lane.
    assert_eq!(take.audible_extent(group.slot), recorded);
}

/// The other half of "a lane's region never moves", pinned on the writer
/// itself. `store_take_in` is what used to do `group.slot = slot` on every
/// take, and it is reachable independently of the resolver — so assert it
/// leaves an existing group's slot alone even when handed a different one.
#[test]
fn storing_a_take_never_rebinds_an_existing_lane() {
    let mut store = TakeGroupStore::new();
    let elsewhere = TimelineRange::new(SLOT.start + 4 * 96_000, 12_000);

    let (first, _) = store_take_in(&mut store, 1, TRACK, SLOT, SLOT, &audio(100));
    let comp = Comp {
        segments: vec![CompSegment {
            range: SLOT,
            take_id: first,
        }],
    };
    store.get_mut(&1).unwrap().comp = comp.clone();

    // A slot nothing would ever match, handed straight to the writer.
    let (second, ordinal) =
        store_take_in(&mut store, 1, TRACK, elsewhere, elsewhere, &audio(200));

    let group = &store[&1];
    assert_eq!(
        group.slot, SLOT,
        "the founding run's region is the lane's region, whatever a later caller passes"
    );
    assert_eq!(group.comp, comp, "and the comp drawn against it is untouched");
    assert_eq!(group.takes.len(), 2, "the take is still filed");
    assert_eq!((second, ordinal), (1, 1), "and still allocated from the group");
}

/// A run over a slot whose lane came back from disk joins **that** lane
/// (todo #1394 restores it; #1392 reuses it), instead of starting a second
/// one beside it.
///
/// This is the first time the restore and the reuse mechanisms fire
/// together, so it gets a case of its own rather than being assumed from
/// either side's. Three allocators meet here and all three have to hold at
/// once:
///
/// - the **group** id — the run must consume none, having joined a lane
///   that already exists (#1394 raised it past the restored ids so a
///   *non*-matching run could not collide either);
/// - the **take** id — `push_take` allocates from the group, so the new
///   take continues the restored sequence instead of replacing take 0
///   (#409);
/// - the **clip** id — `restore_take_groups_in_place` reserved past every
///   restored `clip_ref`, so the WAV this pass writes cannot overwrite a
///   restored take's (#1393).
///
/// The take's lane ordinal continues the restored run too, and the comp
/// the project saved must still be what plays afterwards.
#[test]
fn a_run_over_a_restored_lane_joins_it_rather_than_forking() {
    let mut store = TakeGroupStore::new();
    let mut next = 1u64;
    let mut next_clip = 1u64;
    let saved_slot = TimelineRange::new(0, 48_000);

    // `saved_group(1, ..)` holds three audio takes on clips 100..102.
    restore_take_groups_in_place(
        &mut store,
        &mut next,
        &mut next_clip,
        vec![saved_group(1, 7, 3)],
    );
    assert_eq!(next, 2, "precondition: the group allocator cleared the saved id");
    assert_eq!(next_clip, 103, "precondition: the clip allocator cleared 100..102");

    // Record one more pass over the same region, taking its clip id from
    // the reserved allocator exactly as `roll_audio_pass` does.
    let recorded_clip = next_clip;
    let joined = record_pass(&mut store, &mut next, 7, saved_slot, audio(recorded_clip));

    assert_eq!(joined.group_id, 1, "the pass must join the restored lane");
    assert_eq!(store.len(), 1, "no second lane over the same slot");
    assert_eq!(next, 2, "joining a restored lane consumes no group id");
    assert_eq!(joined.slot, saved_slot);

    let group = &store[&1];
    assert_eq!(
        joined.take_id, 3,
        "the id continues the restored group's sequence"
    );
    assert_eq!(
        joined.pass_index, 3,
        "and so does the lane row — a reloaded lane reads T1..T4, not T1..T3, T1"
    );
    assert_eq!(group.takes.len(), 4, "no restored take is overwritten");
    for restored in 0..3u64 {
        assert!(
            group.take(restored).is_some(),
            "restored take {restored} must survive the new pass"
        );
    }
    assert_eq!(
        group
            .takes
            .iter()
            .map(|t| match t.content {
                TakeContent::Audio { clip_ref } => clip_ref,
                _ => unreachable!("audio takes only"),
            })
            .collect::<Vec<_>>(),
        vec![100, 101, 102, 103],
        "the joined take's WAV must not land on a restored take's clip id"
    );

    // The comp `saved_group` persisted names take 2 (clip 102) and must
    // still be what plays.
    let table = build_comp_table(&store);
    let spans = &table.track_comp(7).expect("track has a comp").spans;
    assert_eq!(
        spans.iter().map(|s| s.clip_id).collect::<Vec<_>>(),
        vec![102],
        "the restored comp must survive a further record run"
    );
    assert!(
        table.is_governed(recorded_clip),
        "the newly recorded take joins the lane the comp governs"
    );
}

/// The "same slot" predicate itself, case by case. Exact `TimelineRange`
/// equality would fork a lane on a frame of jitter; anything appreciably
/// looser accepts a materially different region, and — since the lane
/// keeps the *first* region — silently drops whatever the new take
/// recorded outside it. So: both endpoints within one crossfade window.
#[test]
fn same_slot_means_both_endpoints_within_one_crossfade() {
    const TOL: u64 = SAME_SLOT_TOLERANCE_FRAMES;
    let base = TimelineRange::new(96_000, 48_000);

    let cases: &[(TimelineRange, bool, &str)] = &[
        (base, true, "identical"),
        (
            TimelineRange::new(96_000, 48_001),
            true,
            "loop end out by one frame",
        ),
        (
            TimelineRange::new(96_000 - 1, 48_000),
            true,
            "loop start back by one frame",
        ),
        (
            TimelineRange::new(96_000 + TOL, 48_000),
            true,
            "both endpoints drifted by exactly the tolerance",
        ),
        (
            TimelineRange::new(96_000, 48_000 + TOL),
            true,
            "loop end out by exactly the tolerance",
        ),
        (
            TimelineRange::new(96_000, 48_000 + TOL + 1),
            false,
            "loop end out by one frame past the tolerance",
        ),
        (
            TimelineRange::new(96_000 - TOL - 1, 48_000 + TOL + 1),
            false,
            "loop start back past the tolerance",
        ),
        (
            TimelineRange::new(96_000, 48_000 * 2),
            false,
            "loop length doubled",
        ),
        (
            TimelineRange::new(96_000, 24_000),
            false,
            "a short loop inside the long one",
        ),
        (
            TimelineRange::new(144_000, 48_000),
            false,
            "abutting, sharing no frame",
        ),
        (
            TimelineRange::new(96_000 + 6_000, 48_000),
            false,
            "moved by a 16th note at 120 bpm",
        ),
    ];

    for (other, expected, why) in cases {
        assert_eq!(
            slots_match(base, *other),
            *expected,
            "{why}: {other:?} vs {base:?}"
        );
        assert_eq!(
            slots_match(*other, base),
            *expected,
            "{why}: the rule must be symmetric"
        );
    }

    // Degenerate ranges match only themselves: letting a zero-length
    // region absorb a run near it would lose the whole take, not an edge.
    let empty = TimelineRange::new(96_000, 0);
    assert!(slots_match(empty, empty));
    assert!(!slots_match(empty, base));
    assert!(!slots_match(empty, TimelineRange::new(96_001, 0)));
}

/// When more than one lane matches — which recording cannot produce, but a
/// saved project can hold — the answer is the same every time: the closest
/// lane wins, and the lowest id breaks a tie. The store is a `HashMap`, so
/// iteration order must not be able to decide it.
#[test]
fn a_contested_lookup_is_deterministic() {
    const TOL: u64 = SAME_SLOT_TOLERANCE_FRAMES;
    let run = TimelineRange::new(96_000, 48_000);

    let mut store = TakeGroupStore::new();
    // Lane 3 sits one frame off the run; lanes 7 and 2 are both a full
    // tolerance off, and equally so.
    for (id, start) in [(3u64, 96_001u64), (7, 96_000 + TOL), (2, 96_000 - TOL)] {
        store.insert(
            id,
            TakeGroup::new(id, TRACK, TimelineRange::new(start, 48_000)),
        );
    }

    for _ in 0..200 {
        assert_eq!(
            take_group_for_slot(&store, TRACK, run),
            Some(3),
            "the closest lane must win every time"
        );
    }

    // Drop the closest and the tie between the two equals resolves to the
    // lower id, again every time.
    store.remove(&3);
    for _ in 0..200 {
        assert_eq!(take_group_for_slot(&store, TRACK, run), Some(2));
    }
}

/// No lane, no match — and a lane on another track is never a candidate,
/// however well its region lines up.
#[test]
fn an_unmatched_run_opens_a_new_lane() {
    let mut store = TakeGroupStore::new();
    assert_eq!(take_group_for_slot(&store, TRACK, SLOT), None);

    store.insert(1, TakeGroup::new(1, OTHER_TRACK, SLOT));
    assert_eq!(
        take_group_for_slot(&store, TRACK, SLOT),
        None,
        "an identically-placed lane on another track is not this track's"
    );

    let mut next = 5u64;
    let (group_id, slot) = resolve_take_group(&store, &mut next, TRACK, SLOT);
    assert_eq!(group_id, 5, "a new lane takes the next id");
    assert_eq!(next, 6, "and only one");
    assert_eq!(slot, SLOT, "bound to the region it was recorded over");
}

// ---------------------------------------------------------------------------
// The clip-id reservation and the take-clip load compose (ba todo #1402/#1393)
// ---------------------------------------------------------------------------
//
// A project load now raises `next_clip_id` from two places: #1393's
// reservation inside `restore_take_groups_in_place`, and the
// `max(clip_id + 1)` every clip load does. Both take a `max` and neither
// ever lowers the counter, so the order they arrive in *should* be moot —
// which is worth measuring rather than assuming, because getting it wrong
// re-opens #1393's bug in its nastiest form: the allocator would hand the
// next recording an id whose WAV a restored take is still playing, and
// nothing would look wrong until the file was clobbered.

/// A real, decodable stereo WAV at `audio/clip_{clip_id}.wav`.
fn write_take_clip_wav(dir: &std::path::Path, clip_id: u64) -> std::path::PathBuf {
    let audio = dir.join("audio");
    std::fs::create_dir_all(&audio).expect("create audio dir");
    let path = audio.join(format!("clip_{clip_id}.wav"));
    resonance_audio::transcode_to_wav(&path, &vec![0.5f32; 2_048], 48_000).expect("write wav");
    path
}

/// Drive both real handlers through the engine harness and report where
/// the clip allocator ends up.
fn allocator_after(order: &[resonance_audio::types::AudioCommand]) -> u64 {
    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    for cmd in order {
        assert!(
            engine.replay_take_lane_command(cmd),
            "harness must understand every command in this fixture"
        );
    }
    assert!(
        engine.wait_for_clips(1, std::time::Duration::from_secs(5)),
        "the take clip never loaded"
    );
    engine.next_clip_id()
}

/// Restore-then-load and load-then-restore leave the clip allocator in the
/// same place, past every restored `clip_ref`.
#[test]
fn the_clip_reservation_and_the_take_clip_load_compose_in_either_order() {
    let dir = make_tempdir("reserve-compose");
    // `saved_group(1, 7, 3)` holds clip_refs 100..102, so the reservation
    // has to reach 103 whichever way round the two commands arrive.
    let path = write_take_clip_wav(&dir, 100);

    let restore = resonance_audio::types::AudioCommand::RestoreTakeGroups {
        groups: vec![saved_group(1, 7, 3)],
    };
    let load = resonance_audio::types::AudioCommand::LoadTakeClipFromWav {
        clip_id: 100,
        track_id: 7,
        start_sample: 0,
        path,
        name: "Take 100".into(),
    };

    let restore_first = allocator_after(&[restore.clone(), load.clone()]);
    let load_first = allocator_after(&[load, restore]);

    assert_eq!(
        restore_first, 103,
        "the reservation must clear every restored clip_ref, and the load \
         (which only knows about clip 100) must not pull it back down"
    );
    assert_eq!(
        load_first, restore_first,
        "the two allocator bumps both take a max, so their order is moot"
    );
}

/// Two loads for the same take clip issued **back to back**, with no wait
/// between them, still leave exactly one clip.
///
/// This is the case the submit-time early return cannot cover, and the
/// reason the binding duplicate check lives *inside* the `clips.write()`
/// block in `submit_clip_load`: the load is asynchronous, so when the
/// second command is dispatched the first one's worker has not published
/// yet and the submit-time scan of `ctx.clips` finds nothing. Delete the
/// lock-scoped check and this yields `[100, 100]` — a duplicated
/// `AudioClip` doubling that take's level everywhere the comp reads it.
///
/// Deliberately distinct from the test below, which waits between loads
/// and therefore only ever exercises the submit-time optimisation.
#[test]
fn two_take_clip_loads_racing_each_other_still_leave_one_clip() {
    let dir = make_tempdir("load-race");
    let path = write_take_clip_wav(&dir, 100);
    let load = resonance_audio::types::AudioCommand::LoadTakeClipFromWav {
        clip_id: 100,
        track_id: 7,
        start_sample: 0,
        path,
        name: "Take 100".into(),
    };

    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    // No `wait_for_clips` between these two: the point is that the second
    // dispatch happens while the first load is still in flight.
    engine.replay_take_lane_command(&load);
    engine.replay_take_lane_command(&load);

    let wait_start = std::time::Instant::now();
    assert!(
        engine.wait_for_clips(1, std::time::Duration::from_secs(5)),
        "the take clip never loaded at all"
    );
    // Let a second worker publish if one is going to. A flat sleep here can
    // false-pass under load: a buggy-but-slow second worker can miss a
    // fixed window and land only after the assertion below has already
    // read the list. There is no flush to wait on instead — the duplicate
    // check that actually binds lives inside the worker's own
    // `clips.write()` (see `submit_clip_load`), and this harness has no
    // handle on the worker pool to drain it deterministically. So the
    // grace scales with how long the wait above actually took to see the
    // first publish — a live reading of how loaded this machine is right
    // now — floored so a fast, idle machine still gets a real window, and
    // capped so a pathologically slow one doesn't stall the suite.
    let grace = (wait_start.elapsed() * 5)
        .max(std::time::Duration::from_millis(200))
        .min(std::time::Duration::from_secs(5));
    std::thread::sleep(grace);

    assert_eq!(
        engine.clip_ids(),
        vec![100],
        "two racing loads of one take clip must not both push it"
    );
}

/// Loading a take clip the engine already holds is a no-op on the clip
/// list — it neither duplicates the `AudioClip` nor reloads it.
///
/// The restore path fires on the undo/redo diff replay too, where every
/// take clip is already loaded; a duplicated `AudioClip` would double the
/// take's level everywhere the comp reads it. (This is also the guard that
/// keeps a restore from resurrecting a clip that take removal has parked
/// out of the render's input — see ba todo #1397.)
#[test]
fn re_loading_a_take_clip_the_engine_already_holds_is_a_no_op() {
    let dir = make_tempdir("reload-noop");
    let path = write_take_clip_wav(&dir, 100);
    let load = resonance_audio::types::AudioCommand::LoadTakeClipFromWav {
        clip_id: 100,
        track_id: 7,
        start_sample: 0,
        path,
        name: "Take 100".into(),
    };

    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    engine.replay_take_lane_command(&load);
    assert!(
        engine.wait_for_clips(1, std::time::Duration::from_secs(5)),
        "first load must land"
    );

    // Three more times, as repeated undo/redo replays would.
    for _ in 0..3 {
        engine.replay_take_lane_command(&load);
    }
    std::thread::sleep(std::time::Duration::from_millis(100));

    assert_eq!(
        engine.clip_ids(),
        vec![100],
        "the clip list must still hold exactly one copy of the take clip"
    );
    assert_eq!(
        engine.next_clip_id(),
        101,
        "and the allocator still clears it"
    );
}

// ---------------------------------------------------------------------------
// Restoring a take clip vs. take removal's park (ba todo #1402 x #1397)
// ---------------------------------------------------------------------------
//
// #1397 makes a removed take's recording *parked*: lifted out of the shared
// clip list into `HandlerState::orphaned_take_clips`, so it stops sounding
// without being destroyed (a removal is undoable and the WAV is untouched).
// #1402 makes a restore *load* take clips from disk. The two meet on the
// undo path, where a restore both re-claims a take and is followed by a load
// for it — and the danger is that the take ends up in the clip list twice,
// once un-parked and once freshly mapped, which would double its level
// everywhere the comp reads it.
//
// This was argued from the ordering when the two were on separate branches;
// now that both are in the tree it is measured.

/// An in-RAM DC clip, standing in for the recording capture left in the
/// clip list. Deliberately a different level from what
/// [`write_take_clip_wav`] puts on disk, so a test can tell "the parked
/// clip survived" from "the loader re-read the WAV".
fn memory_clip(id: u64, value: f32) -> resonance_audio::types::AudioClip {
    resonance_audio::types::AudioClip {
        id,
        track_id: 7,
        start_sample: 0,
        source: resonance_audio::types::ClipSource::Memory(vec![value; 48_000 * 2]),
        name: format!("Take {id}"),
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: Default::default(),
        fade_out_frames: 0,
        fade_out_curve: Default::default(),
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

/// A take group holding one audio take that names `clip_ref`.
fn one_audio_take_group(clip_ref: u64) -> TakeGroup {
    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(1, 7, slot);
    push_take(&mut group, slot, &TakeContent::Audio { clip_ref });
    group
}

/// An undo that brings a removed take back un-parks its recording, and the
/// take-clip load that follows the restore finds it already there and does
/// nothing. One clip, not two, and it is the **parked** one — never
/// re-read from disk.
#[test]
fn a_restored_take_clip_does_not_resurrect_the_parked_one() {
    let dir = make_tempdir("unpark-vs-load");
    let path = write_take_clip_wav(&dir, 100);
    let group = one_audio_take_group(100);

    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    engine.seed_take_group(group.clone());
    // The clip capture left behind, distinguishable from anything the
    // loader would produce: the WAV on disk is DC 0.5, this is DC 1.0.
    engine.push_clip(memory_clip(100, 1.0));
    assert_eq!(engine.clip_ids(), vec![100], "precondition: the take is loaded");

    // Remove the take — #1397 parks its recording out of the render.
    engine.remove_take(1, 0);
    assert_eq!(engine.clip_ids(), Vec::<u64>::new(), "the removal parks the clip");
    assert_eq!(engine.parked_clip_ids(), vec![100]);

    // Undo: the restore re-claims the take and un-parks its clip...
    engine.restore_take_groups(vec![group]);
    assert_eq!(engine.clip_ids(), vec![100], "the restore un-parks it");
    assert!(engine.parked_clip_ids().is_empty(), "and the park is emptied");

    // ...and then the load that always follows a restore arrives.
    engine.replay_take_lane_command(&resonance_audio::types::AudioCommand::LoadTakeClipFromWav {
        clip_id: 100,
        track_id: 7,
        start_sample: 0,
        path,
        name: "Take 100".into(),
    });
    std::thread::sleep(std::time::Duration::from_millis(200));

    assert_eq!(
        engine.clip_ids(),
        vec![100],
        "the load must not add a second copy alongside the un-parked one"
    );
    // The surviving clip is the parked original (DC 1.0), not a fresh read
    // of the WAV (DC 0.5) — the load really did nothing at all.
    let out = engine.render_track(7, 0, 4_096);
    let body = out[1_024];
    assert!(
        (body - 1.0).abs() < 1e-6,
        "the un-parked recording must be what plays, got {body}"
    );
}

/// A take the restore does **not** re-claim stays parked: no load is ever
/// sent for it, because the app derives its load list from the same
/// restored groups the engine derives its claim set from.
#[test]
fn a_take_left_removed_stays_parked_across_a_restore() {
    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    engine.seed_take_group(one_audio_take_group(100));
    engine.push_clip(memory_clip(100, 1.0));
    engine.remove_take(1, 0);
    assert_eq!(engine.parked_clip_ids(), vec![100]);

    // A restore carrying the group *without* that take — the redo of the
    // removal. Nothing un-parks, and the app sends no load for a take it
    // is not restoring.
    let slot = TimelineRange::new(0, 48_000);
    engine.restore_take_groups(vec![TakeGroup::new(1, 7, slot)]);

    assert_eq!(
        engine.clip_ids(),
        Vec::<u64>::new(),
        "a take that is still removed must not come back"
    );
    assert_eq!(engine.parked_clip_ids(), vec![100], "it stays parked");
}

// ---------------------------------------------------------------------------
// A removal racing a take-clip load (ba todo #1403)
// ---------------------------------------------------------------------------
//
// #1397 makes a removal silent by lifting the take's recording *out of*
// `ctx.clips`. #1402 makes a project load put it there **asynchronously** —
// `handle_load_take_clip_from_wav` only submits to a worker pool, and the
// `AudioClip` arrives some milliseconds later. A `RemoveTake` handled inside
// that window used to find nothing to park and park nothing, and the worker
// then published a recording no comp table governs: it plays raw, at full
// gain, on the ordinary clip path, on top of the comp. That is #1397's
// "deleting a take makes it louder" returning through a timing window rather
// than a logic error, and doc #292 records it at peak 1.25 against 1.0.
//
// The fix interlocks the removal with the worker's *binding* duplicate check
// — the one inside `clips.write()` — rather than with the advisory
// submit-time early return, which by definition cannot see a load already in
// flight. `park_take_clip` leaves a **claim** in the shared take-clip park
// when the clip is not in the list yet, under that same write lock, and the
// worker delivers into the park instead of into the render's input.
//
// The two orderings are tested apart: the one below is the real race, and
// carries a fixture check so it can never pass by quietly winning the race it
// meant to lose; the one after it constructs the same state deterministically.

/// A real, decodable stereo WAV of constant `level`, `frames` long, at
/// `audio/clip_{clip_id}.wav`.
fn write_dc_take_wav(
    dir: &std::path::Path,
    clip_id: u64,
    level: f32,
    frames: usize,
) -> std::path::PathBuf {
    let audio = dir.join("audio");
    std::fs::create_dir_all(&audio).expect("create audio dir");
    let path = audio.join(format!("clip_{clip_id}.wav"));
    resonance_audio::transcode_to_wav(&path, &vec![level; frames * 2], 48_000)
        .expect("write take wav");
    path
}

fn take_clip_load(clip_id: u64, path: std::path::PathBuf) -> resonance_audio::types::AudioCommand {
    resonance_audio::types::AudioCommand::LoadTakeClipFromWav {
        clip_id,
        track_id: 7,
        start_sample: 0,
        path,
        name: format!("Take {clip_id}"),
    }
}

/// A lane of two audio takes over `[0, 48_000)`: take 0 names `first`, take
/// 1 names `second` — the newer pass, and therefore what the un-comped
/// cover falls back to.
fn two_audio_take_group(first: u64, second: u64) -> TakeGroup {
    let slot = TimelineRange::new(0, 48_000);
    let mut group = TakeGroup::new(1, 7, slot);
    push_take(&mut group, slot, &TakeContent::Audio { clip_ref: first });
    push_take(&mut group, slot, &TakeContent::Audio { clip_ref: second });
    group
}

fn peak(out: &[f32]) -> f32 {
    out.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

/// Spin until every submitted load has landed *somewhere* — the clip list
/// or the park — so the assertions that follow read a settled engine.
///
/// Deliberately counts both, so a broken interlock (which lands the clip in
/// the list) finishes just as fast as a working one and the test fails on
/// its assertions rather than on a timeout.
fn settle(engine: &resonance_audio::__test_support::EngineHandlerHarness, landed: usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline
        && engine.clip_ids().len() + engine.parked_clip_ids().len() < landed
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // A short grace period on top, so a second, wrong publish has time to
    // show up rather than landing after the assertions have read the list.
    std::thread::sleep(std::time::Duration::from_millis(200));
}

/// **The race.** A `RemoveTake` dispatched while the take's recording is
/// still on its way in from a load worker parks that recording anyway, and
/// the lane plays only its survivor.
///
/// This is the real ordering, exactly as a project load produces it:
/// `RestoreTakeGroups`, then one `LoadTakeClipFromWav` per take, then a
/// right-click on a take card the instant the lane is drawn — with nothing
/// waited on in between.
#[test]
fn a_removal_racing_the_take_clip_load_still_parks_the_recording() {
    let dir = make_tempdir("removal-vs-load-race");
    // Take 0's recording is a long one on purpose: its load has to still be
    // in flight when the removal lands. The fixture check below turns a lost
    // race into a failure rather than a vacuous pass.
    let removed = write_dc_take_wav(&dir, 100, 1.0, 240_000);
    let survivor = write_dc_take_wav(&dir, 101, 0.25, 48_000);

    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    engine.restore_take_groups(vec![two_audio_take_group(100, 101)]);
    engine.replay_take_lane_command(&take_clip_load(100, removed));
    engine.replay_take_lane_command(&take_clip_load(101, survivor));

    assert!(
        !engine.clip_ids().contains(&100),
        "fixture check: take 0's load must still be in flight when the \
         removal lands, or this case proves nothing — clips={:?}",
        engine.clip_ids()
    );

    engine.remove_take(1, 0);

    settle(&engine, 2);

    assert_eq!(
        engine.clip_ids(),
        vec![101],
        "the removed take's recording must not reach the render's input, \
         however late its load lands — parked={:?}",
        engine.parked_clip_ids()
    );
    assert_eq!(
        engine.parked_clip_ids(),
        vec![100],
        "and it is parked rather than dropped: the undo has to bring the \
         audio back"
    );

    let out = engine.render_track(7, 0, 4_096);
    assert!(
        (out[2_048] - 0.25).abs() < 1e-6,
        "the surviving pass covers the slot, got {}",
        out[2_048]
    );
    assert!(
        peak(&out) <= 0.25 + 1e-4,
        "peak {} is above the 0.25 the survivor plays — the removed take's \
         recording is summing onto the comp un-governed",
        peak(&out)
    );
}

/// The same interlock, reached deterministically: a load that lands *after*
/// its take was removed is delivered to the park, never to the render.
///
/// No race at all here — the removal happens with nothing loaded, which is
/// precisely the state the race leaves behind — so this case pins the
/// mechanism on every machine and every schedule, while the one above pins
/// that the real ordering reaches it.
#[test]
fn a_take_clip_load_landing_after_its_take_was_removed_goes_to_the_park() {
    let dir = make_tempdir("load-after-removal");
    let removed = write_dc_take_wav(&dir, 100, 1.0, 48_000);

    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    engine.restore_take_groups(vec![two_audio_take_group(100, 101)]);

    engine.remove_take(1, 0);
    assert!(
        engine.parked_clip_ids().is_empty(),
        "precondition: there is no recording to park yet — that is the point"
    );

    engine.replay_take_lane_command(&take_clip_load(100, removed));
    settle(&engine, 1);

    assert_eq!(
        engine.clip_ids(),
        Vec::<u64>::new(),
        "a load for a take that is already gone must not enter the clip list"
    );
    assert_eq!(engine.parked_clip_ids(), vec![100], "it lands in the park");

    let out = engine.render_track(7, 0, 4_096);
    assert!(
        peak(&out) < 1e-4,
        "nothing may play: take 1's recording was never loaded and take 0 \
         was removed — peak {}",
        peak(&out)
    );
}

/// Undoing a removal that raced the load brings the recording back
/// **audibly**, with no second load — the undo arriving **after** the load
/// delivered into the park.
///
/// This is why the worker *delivers* into the park rather than dropping the
/// clip on the floor. Dropping would be enough to make the removal silent —
/// and would leave the undo restoring a card with no audio under it, which
/// is the failure #1397 ruling 4 exists to prevent.
///
/// The other half of the pair — the undo arriving while the claim is still
/// standing — is
/// [`undoing_a_removal_before_the_load_lands_restores_the_audio`]. Both
/// orderings are ordinary gestures and the failure mode is the same silent
/// card, so neither stands in for the other.
#[test]
fn undoing_a_removal_that_raced_the_load_restores_the_audio() {
    let dir = make_tempdir("undo-raced-removal");
    let removed = write_dc_take_wav(&dir, 100, 1.0, 48_000);

    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    engine.restore_take_groups(vec![two_audio_take_group(100, 101)]);
    engine.remove_take(1, 0);
    engine.replay_take_lane_command(&take_clip_load(100, removed));
    settle(&engine, 1);
    assert_eq!(engine.parked_clip_ids(), vec![100], "fixture check: parked");

    // The undo: the app rebuilds its mirror and replays it wholesale.
    engine.restore_take_groups(vec![two_audio_take_group(100, 101)]);
    // Solo the restored take, so what plays is unambiguously *it* rather
    // than the cover's latest-pass fallback.
    engine.set_active_take(1, Some(0));

    assert_eq!(
        engine.clip_ids(),
        vec![100],
        "the restore un-parks the recording the worker delivered"
    );
    assert!(engine.parked_clip_ids().is_empty(), "and empties the park");

    let out = engine.render_track(7, 0, 4_096);
    assert!(
        (out[2_048] - 1.0).abs() < 1e-6,
        "the restored take must be audible, not a silent card, got {}",
        out[2_048]
    );
}

/// And undoing it **before** the load lands restores the audio too: the
/// in-flight recording goes to the clip list, as it would have had the
/// removal never happened.
///
/// Delete a take while its recording is still loading, then hit undo before
/// the worker publishes — an ordinary gesture inside the very window this
/// whole section exists for. What has to happen is that the restore drops
/// the standing **claim**, not merely that it finds no recording to give
/// back: a claim left in place would divert the finished clip into the park
/// a moment later and the take would come back as a visible card with no
/// audio under it, for the rest of the session. Same ruling-4 failure as
/// the case above, reached from the other side of the delivery — and
/// `release` returning `None` for a claim is not enough on its own, so the
/// two cases are not interchangeable.
#[test]
fn undoing_a_removal_before_the_load_lands_restores_the_audio() {
    let dir = make_tempdir("undo-before-load-lands");
    // Long, so the load cannot land before the removal and the undo do.
    let removed = write_dc_take_wav(&dir, 100, 1.0, 240_000);

    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    engine.restore_take_groups(vec![two_audio_take_group(100, 101)]);
    engine.replay_take_lane_command(&take_clip_load(100, removed));
    assert!(
        !engine.clip_ids().contains(&100),
        "fixture check: the load must still be in flight — clips={:?}",
        engine.clip_ids()
    );

    engine.remove_take(1, 0);
    // The removal has to have left a *claim*, not a parked recording, or
    // this case is the one above wearing a different name.
    assert!(
        engine.parked_clip_ids().is_empty() && !engine.clip_ids().contains(&100),
        "fixture check: the removal must leave a standing claim — clips={:?} \
         parked={:?}",
        engine.clip_ids(),
        engine.parked_clip_ids()
    );

    // The undo, dispatched while the claim is still standing.
    engine.restore_take_groups(vec![two_audio_take_group(100, 101)]);
    // Solo the restored take, so what plays is unambiguously *it* rather
    // than the cover's latest-pass fallback.
    engine.set_active_take(1, Some(0));

    settle(&engine, 1);

    // The audible assertion first — it is the one that states the
    // requirement. A claim the restore failed to drop diverts the finished
    // recording into the park, and this reads 0.0: the silent card.
    let out = engine.render_track(7, 0, 4_096);
    assert!(
        (out[2_048] - 1.0).abs() < 1e-6,
        "the restored take must be audible, not a silent card, got {} \
         (clips={:?} parked={:?})",
        out[2_048],
        engine.clip_ids(),
        engine.parked_clip_ids()
    );
    // And the mechanism underneath it.
    assert_eq!(
        engine.clip_ids(),
        vec![100],
        "the in-flight load must land in the render's input, not in the park"
    );
    assert!(
        engine.parked_clip_ids().is_empty(),
        "nothing is parked: the take was un-removed before its recording ever \
         arrived"
    );
}

/// A park claim does not outlive the project that made it, and an ordinary
/// timeline clip is never swallowed by one.
///
/// The claim is on the clip *id*, the worker's interlock is unconditional
/// (so no future caller can route a load around it), and `ClearAll` resets
/// `next_clip_id` to 1 — which is the whole hazard: ids **are** reused
/// across projects. A claim left standing would divert the next project's
/// clip 100 into the park, and a timeline clip would go missing from the
/// arrangement with no echo to the app. `ClearAll` therefore empties the
/// park, claims included.
///
/// Driven through `LoadClipFromWav` rather than the take load because that
/// is where the hazard lives: on the take path a restore always precedes
/// the load and would drop the claim on its own.
#[test]
fn a_park_claim_does_not_outlive_its_project() {
    let dir = make_tempdir("claim-vs-clear-all");
    let wav = write_dc_take_wav(&dir, 100, 1.0, 48_000);

    let mut engine = resonance_audio::__test_support::EngineHandlerHarness::new();
    engine.restore_take_groups(vec![two_audio_take_group(100, 101)]);
    // Take 0 is removed with its recording still in flight: a claim on 100
    // and no recording behind it.
    engine.remove_take(1, 0);

    engine.clear_all();
    engine.drain_events();

    // The next project's *timeline* clip happens to take id 100 — which it
    // will, because `ClearAll` put the allocator back to 1.
    engine.load_clip_from_wav(100, 7, 0, wav, "Timeline clip".into());
    settle(&engine, 1);

    assert_eq!(
        engine.clip_ids(),
        vec![100],
        "a claim from the previous project must not swallow this one's clip"
    );
    assert!(engine.parked_clip_ids().is_empty());
    assert!(
        engine.drain_events().iter().any(|e| matches!(
            e,
            resonance_audio::types::AudioEvent::ClipImported { clip_id: 100, .. }
        )),
        "and the app must still be told the clip arrived"
    );
}
