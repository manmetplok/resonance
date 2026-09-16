//! Take-lane engine-event mirroring (ba todo #410, epic #15, doc #165).
//!
//! Drives `AudioEvent`s through the real dispatch and asserts the app
//! reconstructs its take groups purely from `TakeCaptured` — no
//! read-getters back into the engine. Before this seam existed the
//! dispatch matched `TakeCaptured { .. } => {}` and every captured loop
//! pass was discarded, so the "no take is ever silently lost" acceptance
//! criterion is exactly what these cases pin.
//!
//! The comp / active-take projections doc #165 also assigns to this seam
//! are driven through the same dispatch since todo #411 routed
//! `TakeCompChanged` / `ActiveTakeChanged`: nothing here reaches into the
//! mirror directly, so an unrouted echo fails these cases instead of
//! passing on a hand-written projection call.

use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;
use resonance_common::{CompSegment, TakeContent, TakeNote, TimelineRange};

const SLOT: TimelineRange = TimelineRange {
    start: 1_000,
    length: 44_100,
};

/// A captured audio pass, as a run that yields exactly one take per pass
/// produces it: the engine's `take_id` then counts up in step with
/// `pass_index`. The two are independent fields, though — see
/// [`two_takes_in_one_pass_are_both_retained`].
fn captured(group_id: u64, track_id: u64, pass_index: u32) -> AudioEvent {
    AudioEvent::TakeCaptured {
        group_id,
        take_id: u64::from(pass_index),
        track_id,
        slot: SLOT,
        pass_index,
        extent: SLOT,
        content: TakeContent::Audio {
            clip_ref: 5_000 + u64::from(pass_index),
        },
    }
}

#[test]
fn first_pass_creates_group_then_passes_fold_into_it() {
    let mut app = Resonance::new_for_test().0;

    app.test_apply_engine_event(captured(1, 7, 0));
    assert_eq!(app.test_take_groups().len(), 1);
    let g = &app.test_take_groups()[0];
    assert_eq!(g.id, 1);
    assert_eq!(g.track_id, 7);
    assert_eq!(g.slot, SLOT);
    assert_eq!(g.takes.len(), 1);
    assert_eq!(g.takes[0].pass_index, 0);

    // Later passes of the same record run share the group id and stack as
    // additional takes rather than starting a new group.
    app.test_apply_engine_event(captured(1, 7, 1));
    app.test_apply_engine_event(captured(1, 7, 2));
    assert_eq!(app.test_take_groups().len(), 1);
    let g = &app.test_take_groups()[0];
    let passes: Vec<u32> = g.takes.iter().map(|t| t.pass_index).collect();
    assert_eq!(passes, vec![0, 1, 2]);
    // Take ids are unique within the group, so a comp can address them.
    let ids: Vec<u64> = g.takes.iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![0, 1, 2]);
}

#[test]
fn two_takes_in_one_pass_are_both_retained() {
    // A single loop pass can yield two takes for one track — the engine's
    // audio roll and its MIDI capture resolve to the same group (ba doc
    // #292). The mirror keys on `take_id`, not `pass_index`, so both are
    // kept; while the id was derived app-side from `pass_index` the second
    // silently replaced the first.
    //
    // The two events below deliberately *share* a `pass_index`, which the
    // engine no longer emits for this shape: since todo #1392 the ordinal
    // is allocated from the group, so the audio and MIDI halves of one
    // pass come through as 0 and 1. That is the point of feeding the
    // collision by hand — the mirror must key on the id whatever ordinal
    // policy the engine happens to run, and this is the only place that
    // stays true if the policy changes again.
    let mut app = Resonance::new_for_test().0;

    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: 1,
        take_id: 0,
        track_id: 7,
        slot: SLOT,
        pass_index: 0,
        extent: SLOT,
        content: TakeContent::Audio { clip_ref: 5_000 },
    });
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: 1,
        take_id: 1,
        track_id: 7,
        slot: SLOT,
        pass_index: 0,
        extent: SLOT,
        content: TakeContent::Midi { notes: Vec::new() },
    });

    let g = &app.test_take_groups()[0];
    assert_eq!(g.takes.len(), 2, "both takes of the pass must be retained");
    assert_eq!(
        g.takes.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![0, 1],
        "the takes are addressable by their engine-assigned ids"
    );
    // The fabricated ordinals collide; `pass_index` alone cannot tell
    // these two apart, which is exactly why it is not the identity.
    assert!(g.takes.iter().all(|t| t.pass_index == 0));
}

#[test]
fn three_cycle_passes_retain_three_distinct_takes() {
    // The epic's headline acceptance: cycling three times over a loop with
    // a track armed retains three takes, each with its own content.
    let mut app = Resonance::new_for_test().0;
    for pass in 0..3 {
        app.test_apply_engine_event(captured(1, 7, pass));
    }

    let g = &app.test_take_groups()[0];
    assert_eq!(g.takes.len(), 3);
    let contents: Vec<&TakeContent> = g.takes.iter().map(|t| &t.content).collect();
    assert_eq!(
        contents,
        vec![
            &TakeContent::Audio { clip_ref: 5_000 },
            &TakeContent::Audio { clip_ref: 5_001 },
            &TakeContent::Audio { clip_ref: 5_002 },
        ]
    );
}

#[test]
fn distinct_groups_keep_insertion_order() {
    let mut app = Resonance::new_for_test().0;

    app.test_apply_engine_event(captured(1, 7, 0));
    app.test_apply_engine_event(captured(2, 8, 0));

    let ids: Vec<u64> = app.test_take_groups().iter().map(|g| g.id).collect();
    assert_eq!(ids, vec![1, 2]);
    // Each group stays bound to the track it was recorded on.
    let tracks: Vec<u64> = app.test_take_groups().iter().map(|g| g.track_id).collect();
    assert_eq!(tracks, vec![7, 8]);
}

#[test]
fn concurrently_armed_tracks_get_one_group_each_per_pass() {
    // Two armed tracks cycling together: the engine emits one event per
    // track per seam, and neither track's take may land in the other's
    // group.
    let mut app = Resonance::new_for_test().0;
    for pass in 0..2 {
        app.test_apply_engine_event(captured(1, 7, pass));
        app.test_apply_engine_event(captured(2, 8, pass));
    }

    assert_eq!(app.test_take_groups().len(), 2);
    for g in app.test_take_groups() {
        assert_eq!(g.takes.len(), 2, "group {} lost a pass", g.id);
    }
}

#[test]
fn midi_take_content_is_mirrored_verbatim() {
    let mut app = Resonance::new_for_test().0;

    let notes = vec![TakeNote {
        note: 60,
        velocity: 0.8,
        start_tick: 0,
        duration_ticks: 480,
    }];
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: 3,
        take_id: 0,
        track_id: 9,
        slot: SLOT,
        pass_index: 0,
        extent: SLOT,
        content: TakeContent::Midi {
            notes: notes.clone(),
        },
    });

    let g = &app.test_take_groups()[0];
    assert_eq!(g.takes[0].content, TakeContent::Midi { notes });
}

#[test]
fn re_delivered_take_replaces_rather_than_duplicates() {
    let mut app = Resonance::new_for_test().0;

    app.test_apply_engine_event(captured(1, 7, 0));
    // The same pass again (a re-delivery) must not create a duplicate.
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: 1,
        take_id: 0,
        track_id: 7,
        slot: SLOT,
        pass_index: 0,
        extent: SLOT,
        content: TakeContent::Audio { clip_ref: 9_999 },
    });

    let g = &app.test_take_groups()[0];
    assert_eq!(g.takes.len(), 1);
    assert_eq!(g.takes[0].content, TakeContent::Audio { clip_ref: 9_999 });
}

#[test]
fn a_captured_take_starts_with_no_comp_and_no_active_take() {
    let mut app = Resonance::new_for_test().0;
    app.test_apply_engine_event(captured(1, 7, 0));

    let g = &app.test_take_groups()[0];
    assert!(g.comp.segments.is_empty());
    assert_eq!(g.active_take, None);
    // An empty comp does not cover the slot, so the lane has nothing to
    // play until the engine echoes one.
    assert!(!g.is_full_cover());
}

#[test]
fn comp_changed_replaces_the_comp_cover() {
    let mut app = Resonance::new_for_test().0;
    app.test_apply_engine_event(captured(1, 7, 0));
    app.test_apply_engine_event(captured(1, 7, 1));

    let segments = vec![
        CompSegment {
            range: TimelineRange::new(1_000, 22_050),
            take_id: 0,
        },
        CompSegment {
            range: TimelineRange::new(23_050, 22_050),
            take_id: 1,
        },
    ];
    app.test_apply_engine_event(AudioEvent::TakeCompChanged {
        group_id: 1,
        segments: segments.clone(),
    });
    assert_eq!(app.test_take_groups()[0].comp.segments, segments);
    // Those two segments tile the slot exactly.
    assert!(app.test_take_groups()[0].is_full_cover());

    // A subsequent comp change supersedes the previous cover wholesale.
    let next = vec![CompSegment {
        range: SLOT,
        take_id: 1,
    }];
    app.test_apply_engine_event(AudioEvent::TakeCompChanged {
        group_id: 1,
        segments: next.clone(),
    });
    assert_eq!(app.test_take_groups()[0].comp.segments, next);
}

#[test]
fn active_take_changed_sets_then_clears_the_soloed_take() {
    let mut app = Resonance::new_for_test().0;
    app.test_apply_engine_event(captured(1, 7, 0));
    assert_eq!(app.test_take_groups()[0].active_take, None);

    app.test_apply_engine_event(AudioEvent::ActiveTakeChanged {
        group_id: 1,
        take_id: Some(0),
    });
    assert_eq!(app.test_take_groups()[0].active_take, Some(0));

    app.test_apply_engine_event(AudioEvent::ActiveTakeChanged {
        group_id: 1,
        take_id: None,
    });
    assert_eq!(app.test_take_groups()[0].active_take, None);
}

#[test]
fn comp_and_active_changes_for_unknown_group_are_noops() {
    let mut app = Resonance::new_for_test().0;

    app.test_apply_engine_event(AudioEvent::TakeCompChanged {
        group_id: 99,
        segments: vec![CompSegment {
            range: SLOT,
            take_id: 1,
        }],
    });
    app.test_apply_engine_event(AudioEvent::ActiveTakeChanged {
        group_id: 99,
        take_id: Some(1),
    });

    // No group was invented from a comp/active echo alone.
    assert!(app.test_take_groups().is_empty());
}

// ---------------------------------------------------------------------
// The peak cache is keyed by take, validated by recording (ba todo #1400)
// ---------------------------------------------------------------------

/// A cached waveform is only served to the recording it was read from.
///
/// `TakeGroupState::peaks` survives a diff-replay rebuild
/// (`clear_for_snapshot`) so an undo costs no WAV reads. That is safe
/// only because a lookup proves *which* recording it is asking about:
/// `(group, take)` names a slot in the mirror, not a recording, and the
/// same pair can come to name a different pass — see
/// [`a_reused_take_id_does_not_inherit_the_previous_recordings_waveform`]
/// for the sequence that does it.
///
/// This is the unit-level statement of that rule, on the two accessors
/// that are the only ways into the map (the field is private). Delete the
/// `clip_ref` compare in either and this fails; without it, nothing did —
/// the whole suite stayed green through exactly that mutation, which is
/// why this test exists.
#[test]
fn cached_peaks_are_served_only_to_the_recording_they_were_read_from() {
    use resonance_app::state::TakeGroupState;

    const GROUP: u64 = 1;
    const TAKE: u64 = 2;
    const RECORDED: u64 = 100;
    const OTHER: u64 = 101;

    let mut takes = TakeGroupState::default();
    let table = vec![(-0.5, 0.5), (-0.25, 0.25)];
    takes.set_peaks(GROUP, TAKE, RECORDED, table.clone());

    assert_eq!(
        takes.peaks(GROUP, TAKE, RECORDED),
        table.as_slice(),
        "the recording it was read from gets its waveform"
    );
    assert!(takes.has_peaks(GROUP, TAKE, RECORDED));

    assert!(
        takes.peaks(GROUP, TAKE, OTHER).is_empty(),
        "a different recording under the same key gets nothing — drawing \
         the cached table here is a lane showing the wrong take's audio"
    );
    assert!(
        !takes.has_peaks(GROUP, TAKE, OTHER),
        "and reports a miss, so the caller re-reads instead of skipping"
    );

    // Neighbouring keys are unaffected either way round.
    assert!(takes.peaks(GROUP, TAKE + 1, RECORDED).is_empty());
    assert!(takes.peaks(GROUP + 1, TAKE, RECORDED).is_empty());
}

/// Removing a lane forgets its waveforms too (ba todo #1400 x #1397).
///
/// `TakeGroupState::peaks`'s doc promises that "every site that forgets a
/// take forgets both" — the missing flag and the cached waveform. That was
/// true at `remove_take` and, for one merge, **false at `remove_group`**:
/// #1397 wrote that function against a state with no peak map, so it cited
/// `remove_take`'s reasoning while doing half of it, and every take of a
/// removed lane orphaned a peak table for the rest of the session.
///
/// A leak rather than a corruption, because a lookup compares the
/// `clip_ref` and a reused group id therefore misses rather than drawing
/// the orphaned table — but a promise with one exception is not a promise,
/// and the next reader would have to rediscover which half was true.
#[test]
fn removing_a_take_group_forgets_its_cached_waveforms() {
    use resonance_app::state::TakeGroupState;

    const GROUP: u64 = 3;
    const OTHER: u64 = 4;

    let mut takes = TakeGroupState::default();
    takes.set_peaks(GROUP, 0, 100, vec![(-0.5, 0.5)]);
    takes.set_peaks(GROUP, 1, 101, vec![(-0.25, 0.25)]);
    takes.set_peaks(OTHER, 0, 200, vec![(-0.75, 0.75)]);
    takes.mark_missing(GROUP, 1);
    takes.take_captured(
        GROUP,
        7,
        TimelineRange { start: 0, length: 4_800 },
        resonance_common::Take::new(
            0,
            0,
            0,
            TimelineRange { start: 0, length: 4_800 },
            TakeContent::Audio { clip_ref: 100 },
        ),
    );

    assert!(takes.remove_group(GROUP), "the lane was there to remove");

    assert!(
        takes.peaks(GROUP, 0, 100).is_empty() && takes.peaks(GROUP, 1, 101).is_empty(),
        "no take of the removed lane keeps a waveform"
    );
    assert!(
        !takes.has_peaks(GROUP, 0, 100),
        "and a lane that later reuses the id starts from the recording, not a memory"
    );
    assert!(
        !takes.is_missing(GROUP, 1),
        "the missing flag goes too, as it always did"
    );
    assert_eq!(
        takes.peaks(OTHER, 0, 200),
        [(-0.75, 0.75)].as_slice(),
        "and another lane's waveforms are untouched"
    );
}
