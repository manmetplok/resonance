//! Take-lane engine-event mirroring (ba todo #410, epic #15, doc #165).
//!
//! Drives `AudioEvent`s through the real dispatch and asserts the app
//! reconstructs its take groups purely from `TakeCaptured` — no
//! read-getters back into the engine. Before this seam existed the
//! dispatch matched `TakeCaptured { .. } => {}` and every captured loop
//! pass was discarded, so the "no take is ever silently lost" acceptance
//! criterion is exactly what these cases pin.
//!
//! The comp / active-take projections doc #165 also assigns to this todo
//! are asserted directly on `TakeGroupState`: their engine echoes
//! (`TakeCompChanged` / `ActiveTakeChanged`) belong to todo #409 and are
//! not on `AudioEvent` yet, so there is no event to route through
//! dispatch. Move those two cases onto `test_apply_engine_event` when the
//! variants land.

use resonance_app::Resonance;
use resonance_audio::types::AudioEvent;
use resonance_common::{CompSegment, TakeContent, TakeNote, TimelineRange};

const SLOT: TimelineRange = TimelineRange {
    start: 1_000,
    length: 44_100,
};

/// A captured audio pass. The engine derives no take id of its own yet, so
/// `pass_index` is what distinguishes takes within a group.
fn captured(group_id: u64, track_id: u64, pass_index: u32) -> AudioEvent {
    AudioEvent::TakeCaptured {
        group_id,
        track_id,
        slot: SLOT,
        pass_index,
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
        track_id: 9,
        slot: SLOT,
        pass_index: 0,
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
        track_id: 7,
        slot: SLOT,
        pass_index: 0,
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
    app.test_take_groups_mut().comp_changed(1, segments.clone());
    assert_eq!(app.test_take_groups()[0].comp.segments, segments);
    // Those two segments tile the slot exactly.
    assert!(app.test_take_groups()[0].is_full_cover());

    // A subsequent comp change supersedes the previous cover wholesale.
    let next = vec![CompSegment {
        range: SLOT,
        take_id: 1,
    }];
    app.test_take_groups_mut().comp_changed(1, next.clone());
    assert_eq!(app.test_take_groups()[0].comp.segments, next);
}

#[test]
fn active_take_changed_sets_then_clears_the_soloed_take() {
    let mut app = Resonance::new_for_test().0;
    app.test_apply_engine_event(captured(1, 7, 0));
    assert_eq!(app.test_take_groups()[0].active_take, None);

    app.test_take_groups_mut().active_take_changed(1, Some(0));
    assert_eq!(app.test_take_groups()[0].active_take, Some(0));

    app.test_take_groups_mut().active_take_changed(1, None);
    assert_eq!(app.test_take_groups()[0].active_take, None);
}

#[test]
fn comp_and_active_changes_for_unknown_group_are_noops() {
    let mut app = Resonance::new_for_test().0;

    app.test_take_groups_mut().comp_changed(
        99,
        vec![CompSegment {
            range: SLOT,
            take_id: 1,
        }],
    );
    app.test_take_groups_mut().active_take_changed(99, Some(1));

    // No group was invented from a comp/active echo alone.
    assert!(app.test_take_groups().is_empty());
}
