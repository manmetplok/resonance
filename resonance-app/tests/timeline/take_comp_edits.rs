//! Take-lane comping: messages, update handlers, comp edits + undo
//! (ba todo #411, epic #15, design doc #165).
//!
//! Every case drives a real [`TakeMessage`] through `Resonance::update`
//! against a command-capturing engine, so it pins three things at once:
//! what the app-side mirror becomes, what `AudioCommand`s the engine is
//! actually told, and what the undo history did about it. Asserting only
//! the mirror would pass for a handler that never spoke to the engine —
//! and a comp the engine has not been told about is a comp nobody hears.
//!
//! The findings from the #409 / #410 reviews (ba doc #292) each have a
//! case here:
//!
//! - `SetTakeComp` validates nothing, so segments are derived from the
//!   slot and the comp helpers, never taken from the caller.
//! - A take's audible extent is its clip's, not the slot's, so a promote
//!   is clamped to it — a segment over a region its take cannot fill is a
//!   silent hole in the composite.
//! - `SetActiveTake` drops an unknown take id with **no** echo, so a
//!   selection the group cannot honour is refused before it is sent.
//! - A MIDI take soloed on a group that also holds audio silences the
//!   audio path; that is by design and is reported rather than hidden.

use resonance_app::message::{Message, TakeMessage, TransportMessage};
use resonance_app::state::ClipState;
use resonance_app::Resonance;
use resonance_audio::__test_support::Receiver;
use resonance_audio::types::{AudioCommand, AudioEvent, FadeCurve, TrackType};
use resonance_common::{CompSegment, TakeContent, TakeNote, TimelineRange};

const TRACK: u64 = 7;
const MIDI_TRACK: u64 = 8;
const GROUP: u64 = 1;

/// Two bars at 120 BPM / 48 kHz, starting one bar in.
const SLOT: TimelineRange = TimelineRange {
    start: 96_000,
    length: 192_000,
};
const MID: u64 = SLOT.start + SLOT.length / 2;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// An app with an active, saved project (the undo gate needs a path) and a
/// command-capturing engine.
fn capturing_app() -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-take-comp-test"));
    app.test_add_track(TRACK, TrackType::Audio);
    app.test_add_track(MIDI_TRACK, TrackType::Instrument);
    let rx = app.test_capture_engine();
    (app, rx)
}

/// One captured audio pass, as the engine reports it.
fn capture(app: &mut Resonance, take_id: u64, pass_index: u32) {
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        take_id,
        track_id: TRACK,
        slot: SLOT,
        pass_index,
        content: TakeContent::Audio {
            clip_ref: 5_000 + take_id,
        },
    });
}

/// A group of `n` audio takes with ids `0..n`, no comp and no solo — the
/// state a cycle-record run leaves behind.
fn app_with_takes(n: u64) -> (Resonance, Receiver<AudioCommand>) {
    let (mut app, rx) = capturing_app();
    for i in 0..n {
        capture(&mut app, i, i as u32);
    }
    drain(&rx);
    (app, rx)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

fn send(app: &mut Resonance, m: TakeMessage) {
    let _ = app.update(Message::Take(m));
}

fn seek(app: &mut Resonance, pos: u64) {
    let _ = app.update(Message::Transport(TransportMessage::SeekToSample(pos)));
}

/// The mirrored comp of the one group under test, as `(start, end, take)`.
fn comp(app: &Resonance) -> Vec<(u64, u64, u64)> {
    app.test_take_groups()[0]
        .comp
        .segments
        .iter()
        .map(|s| (s.range.start, s.range.end(), s.take_id))
        .collect()
}

/// The segments of the last `SetTakeComp` in `cmds`, if any.
fn last_comp_command(cmds: &[AudioCommand]) -> Option<Vec<CompSegment>> {
    cmds.iter().rev().find_map(|c| match c {
        AudioCommand::SetTakeComp { group_id, segments } if *group_id == GROUP => {
            Some(segments.clone())
        }
        _ => None,
    })
}

fn active_take_commands(cmds: &[AudioCommand]) -> Vec<Option<u64>> {
    cmds.iter()
        .filter_map(|c| match c {
            AudioCommand::SetActiveTake { group_id, take_id } if *group_id == GROUP => {
                Some(*take_id)
            }
            _ => None,
        })
        .collect()
}

/// A clip the app knows about, standing in for a recorded take's audio.
fn take_clip(clip_ref: u64, start: u64, len: u64) -> ClipState {
    ClipState {
        id: clip_ref,
        track_id: TRACK,
        start_sample: start,
        duration_samples: len,
        name: format!("Take {clip_ref}"),
        total_frames: len,
        trim_start_frames: 0,
        trim_end_frames: 0,
        fade_in_frames: 0,
        fade_in_curve: FadeCurve::default(),
        fade_out_frames: 0,
        fade_out_curve: FadeCurve::default(),
        gain_db: 0.0,
        waveform_peaks: Vec::new(),
        vocal_tuning: None,
        asset_ref: None,
    }
}

// ---------------------------------------------------------------------------
// Promote
// ---------------------------------------------------------------------------

#[test]
fn a_first_promote_materializes_the_engines_default_cover_around_it() {
    // A freshly recorded group has NO comp segments, but it is not silent:
    // the engine covers the slot with the latest pass. If the first
    // promote pushed only its own segment, everything outside it would
    // turn from "the newest take" into a silent hole.
    let (mut app, rx) = app_with_takes(3);
    assert!(app.test_take_groups()[0].comp.segments.is_empty());

    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, MID),
        },
    );

    assert_eq!(
        comp(&app),
        vec![(SLOT.start, MID, 0), (MID, SLOT.end(), 2)],
        "the promoted half, and the rest still on the take the engine was playing"
    );
    let cmds = drain(&rx);
    let sent = last_comp_command(&cmds).expect("the comp reached the engine");
    assert_eq!(
        sent.iter()
            .map(|s| (s.range.start, s.range.end(), s.take_id))
            .collect::<Vec<_>>(),
        comp(&app),
        "what the engine renders is what the lane shows"
    );
    assert!(
        app.test_take_groups()[0].is_full_cover(),
        "the comp still tiles the whole slot"
    );
}

#[test]
fn promote_is_clamped_to_the_takes_own_recorded_audio() {
    // Pass 0 punched in late, so its clip starts after the slot does. The
    // engine does not sanitise segment ranges, and a segment over a region
    // its take cannot fill renders as silence (ba doc #292) — so the
    // request is trimmed to what take 0 actually recorded.
    let (mut app, rx) = app_with_takes(2);
    let punch_in = SLOT.start + 40_000;
    app.test_push_clip(take_clip(5_000, punch_in, SLOT.end() - punch_in));

    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: SLOT, // the user asked for the whole slot
        },
    );

    assert_eq!(
        comp(&app),
        vec![(SLOT.start, punch_in, 1), (punch_in, SLOT.end(), 0)],
        "take 0 covers only what it recorded; the head stays on take 1"
    );
    assert!(last_comp_command(&drain(&rx)).is_some());
}

#[test]
fn a_promote_entirely_outside_the_takes_audio_is_refused() {
    // Nothing survives the clamp, so there is no edit — and therefore no
    // command, no undo entry and no dirty flag.
    let (mut app, rx) = app_with_takes(2);
    app.test_push_clip(take_clip(5_000, SLOT.start + 100_000, 20_000));
    app.test_set_dirty(false);

    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, SLOT.start + 10_000),
        },
    );

    assert!(comp(&app).is_empty(), "the comp is untouched");
    assert!(drain(&rx).is_empty(), "nothing was sent to the engine");
    assert!(!app.test_can_undo(), "a refused edit spends no undo entry");
    assert!(!app.test_dirty(), "and does not dirty the project");
}

#[test]
fn a_promote_is_clipped_to_the_slot_even_when_the_take_is_unbounded() {
    // The app has no clip for this take, so its extent is unknown and
    // falls back to the slot — a request reaching outside must still not
    // put a segment outside the group's slot.
    let (mut app, _rx) = app_with_takes(2);

    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(0, SLOT.end() + 500_000),
        },
    );

    assert_eq!(comp(&app), vec![(SLOT.start, SLOT.end(), 0)]);
}

#[test]
fn a_promote_on_an_unknown_group_or_take_is_refused() {
    let (mut app, rx) = app_with_takes(2);

    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: 99,
            take_id: 0,
            range: SLOT,
        },
    );
    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 99,
            range: SLOT,
        },
    );

    assert!(comp(&app).is_empty());
    assert!(drain(&rx).is_empty());
    assert!(!app.test_can_undo());
}

// ---------------------------------------------------------------------------
// Split at the playhead
// ---------------------------------------------------------------------------

#[test]
fn split_at_the_playhead_cuts_the_cover_in_two_at_that_frame() {
    let (mut app, rx) = app_with_takes(2);
    seek(&mut app, MID);
    drain(&rx);

    send(
        &mut app,
        TakeMessage::SplitCompAtPlayhead { group_id: GROUP },
    );

    assert_eq!(
        comp(&app),
        vec![(SLOT.start, MID, 1), (MID, SLOT.end(), 1)],
        "one boundary at the playhead, both halves still the same take"
    );
    assert!(
        last_comp_command(&drain(&rx)).is_some(),
        "the boundary is pushed so a later promote edits the same cover the engine has"
    );
}

#[test]
fn splitting_off_the_slot_or_on_an_existing_boundary_is_refused() {
    let (mut app, rx) = app_with_takes(2);

    // Before the slot.
    seek(&mut app, SLOT.start - 1);
    drain(&rx);
    send(
        &mut app,
        TakeMessage::SplitCompAtPlayhead { group_id: GROUP },
    );
    assert!(drain(&rx).is_empty(), "no cut outside the slot");
    assert!(!app.test_can_undo());

    // Exactly on the slot start: the boundary already exists.
    seek(&mut app, SLOT.start);
    drain(&rx);
    send(
        &mut app,
        TakeMessage::SplitCompAtPlayhead { group_id: GROUP },
    );
    assert!(drain(&rx).is_empty(), "no cut on an existing boundary");
    assert!(!app.test_can_undo());

    // A real cut, then the same cut again.
    seek(&mut app, MID);
    drain(&rx);
    send(
        &mut app,
        TakeMessage::SplitCompAtPlayhead { group_id: GROUP },
    );
    assert!(last_comp_command(&drain(&rx)).is_some());
    send(
        &mut app,
        TakeMessage::SplitCompAtPlayhead { group_id: GROUP },
    );
    assert!(
        drain(&rx).is_empty(),
        "cutting the same frame twice is a no-op"
    );
    assert_eq!(comp(&app).len(), 2);
}

#[test]
fn a_lone_take_still_yields_a_cover_to_cut() {
    // The materialized cover comes from the group's fallback take, so a
    // one-pass group is comp-editable without a second take to promote.
    let (mut app, rx) = app_with_takes(1);
    seek(&mut app, MID);
    drain(&rx);

    send(
        &mut app,
        TakeMessage::SplitCompAtPlayhead { group_id: GROUP },
    );

    assert_eq!(comp(&app), vec![(SLOT.start, MID, 0), (MID, SLOT.end(), 0)]);
}

// ---------------------------------------------------------------------------
// Active take
// ---------------------------------------------------------------------------

#[test]
fn selecting_an_active_take_mirrors_it_and_tells_the_engine() {
    let (mut app, rx) = app_with_takes(3);

    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(1),
        },
    );

    assert_eq!(app.test_take_groups()[0].active_take, Some(1));
    assert_eq!(active_take_commands(&drain(&rx)), vec![Some(1)]);

    // And clearing it puts the comp back in charge.
    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: None,
        },
    );
    assert_eq!(app.test_take_groups()[0].active_take, None);
    assert_eq!(active_take_commands(&drain(&rx)), vec![None]);
}

#[test]
fn selecting_a_take_the_group_does_not_hold_is_refused_not_optimistically_mirrored() {
    // The engine drops such a command silently and echoes NOTHING (ba doc
    // #292), so an optimistic mirror would assert a solo that never
    // happened and never be corrected.
    let (mut app, rx) = app_with_takes(2);

    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(99),
        },
    );

    assert_eq!(app.test_take_groups()[0].active_take, None);
    assert!(drain(&rx).is_empty());
    assert!(!app.test_can_undo());
}

#[test]
fn reselecting_the_current_take_is_refused() {
    let (mut app, rx) = app_with_takes(2);
    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(1),
        },
    );
    drain(&rx);
    let entries_before = app.test_undo_history().test_undo_entries().len();

    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(1),
        },
    );

    assert!(drain(&rx).is_empty());
    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        entries_before,
        "a selection that changes nothing spends no undo entry"
    );
}

#[test]
fn a_comp_edit_ends_the_take_solo() {
    // An active take overrides the comp entirely, so a split or promote
    // made while soloing would change nothing audible. Editing the comp
    // means "play the comp".
    let (mut app, rx) = app_with_takes(2);
    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(0),
        },
    );
    drain(&rx);

    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, MID),
        },
    );

    assert_eq!(app.test_take_groups()[0].active_take, None);
    let cmds = drain(&rx);
    assert_eq!(active_take_commands(&cmds), vec![None]);
    assert!(last_comp_command(&cmds).is_some());
}

#[test]
fn a_midi_active_take_is_reported_as_silencing_the_groups_audio() {
    // Soloing a MIDI take resolves to zero audio spans while the group's
    // audio clips stay governed, so the lane goes silent on the audio
    // path. Intentional, but the UI has to be able to say so.
    let (mut app, _rx) = app_with_takes(2);
    app.test_apply_engine_event(AudioEvent::TakeCaptured {
        group_id: GROUP,
        take_id: 2,
        track_id: TRACK,
        slot: SLOT,
        pass_index: 2,
        content: TakeContent::Midi {
            notes: vec![TakeNote {
                note: 60,
                velocity: 0.8,
                start_tick: 0,
                duration_ticks: 480,
            }],
        },
    });

    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(0),
        },
    );
    assert!(!app.test_active_take_silences_audio(GROUP));

    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(2),
        },
    );
    assert!(
        app.test_active_take_silences_audio(GROUP),
        "a MIDI solo over audio takes is flagged, not left looking like a bug"
    );
}

// ---------------------------------------------------------------------------
// Delete
// ---------------------------------------------------------------------------

#[test]
fn deleting_a_take_re_covers_the_slot_from_the_survivors() {
    // The deleted take's segments cannot simply be dropped: a gap in the
    // comp is silence in the middle of the part.
    let (mut app, rx) = app_with_takes(3);
    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, MID),
        },
    );
    drain(&rx);
    assert_eq!(comp(&app), vec![(SLOT.start, MID, 0), (MID, SLOT.end(), 2)]);

    send(
        &mut app,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    let takes: Vec<u64> = app.test_take_groups()[0]
        .takes
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(takes, vec![1, 2], "the take is gone from the lane");
    assert_eq!(
        comp(&app),
        vec![(SLOT.start, SLOT.end(), 2)],
        "its half is handed to the survivor the engine would fall back to, and merged"
    );
    assert!(
        last_comp_command(&drain(&rx)).is_some(),
        "the re-cover is pushed, or the engine keeps playing the deleted take"
    );
}

#[test]
fn deleting_the_soloed_take_clears_the_solo() {
    let (mut app, rx) = app_with_takes(2);
    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(0),
        },
    );
    drain(&rx);

    send(
        &mut app,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    assert_eq!(app.test_take_groups()[0].active_take, None);
    assert_eq!(active_take_commands(&drain(&rx)), vec![None]);
}

#[test]
fn deleting_a_take_that_is_not_soloed_leaves_the_solo_alone() {
    let (mut app, rx) = app_with_takes(3);
    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(2),
        },
    );
    drain(&rx);

    send(
        &mut app,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    assert_eq!(app.test_take_groups()[0].active_take, Some(2));
    assert!(active_take_commands(&drain(&rx)).is_empty());
}

#[test]
fn a_groups_last_take_cannot_be_deleted() {
    // There is no engine-side take-removal command, and a group whose comp
    // is empty falls back to playing its most recent pass — so a "deleted"
    // last take would keep sounding. Refusing is the honest outcome.
    let (mut app, rx) = app_with_takes(1);

    send(
        &mut app,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    assert_eq!(app.test_take_groups()[0].takes.len(), 1);
    assert!(drain(&rx).is_empty());
    assert!(!app.test_can_undo());
}

// ---------------------------------------------------------------------------
// Undo / redo
// ---------------------------------------------------------------------------

#[test]
fn a_comp_edit_undoes_on_screen_and_in_the_engine() {
    let (mut app, rx) = app_with_takes(2);
    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, MID),
        },
    );
    let after_edit = comp(&app);
    assert_eq!(after_edit.len(), 2);
    drain(&rx);

    let _ = app.update(Message::Undo);

    assert!(
        comp(&app).is_empty(),
        "the lane is back to the un-comped state"
    );
    let sent = last_comp_command(&drain(&rx)).expect("the engine is told about the undo");
    assert!(
        sent.is_empty(),
        "and told the comp is empty again, not left rendering the undone edit"
    );

    let _ = app.update(Message::Redo);
    assert_eq!(comp(&app), after_edit, "redo re-applies the promote");
    let sent = last_comp_command(&drain(&rx)).expect("the engine is told about the redo");
    assert_eq!(
        sent.iter()
            .map(|s| (s.range.start, s.range.end(), s.take_id))
            .collect::<Vec<_>>(),
        after_edit
    );
}

#[test]
fn a_take_solo_undoes_through_the_same_path() {
    let (mut app, rx) = app_with_takes(2);
    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(0),
        },
    );
    drain(&rx);

    let _ = app.update(Message::Undo);

    assert_eq!(app.test_take_groups()[0].active_take, None);
    assert_eq!(
        active_take_commands(&drain(&rx)),
        vec![None],
        "the engine stops soloing too"
    );
}

#[test]
fn a_delete_undoes_the_take_back_into_the_lane() {
    let (mut app, rx) = app_with_takes(3);
    send(
        &mut app,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 1,
        },
    );
    assert_eq!(app.test_take_groups()[0].takes.len(), 2);
    drain(&rx);

    let _ = app.update(Message::Undo);

    let takes: Vec<u64> = app.test_take_groups()[0]
        .takes
        .iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(takes, vec![0, 1, 2], "no take is ever silently lost");
    // Specifically a comp push, not merely *some* traffic: the diff
    // replay emits plenty of unrelated commands, so asserting the
    // receiver is non-empty would pass even with the resync gone.
    let sent = last_comp_command(&drain(&rx))
        .expect("the restored lane's comp is re-asserted onto the engine");
    assert_eq!(
        sent.iter()
            .map(|s| (s.range.start, s.range.end(), s.take_id))
            .collect::<Vec<_>>(),
        comp(&app),
        "and it is the cover the lane came back to"
    );
}

#[test]
fn each_comp_edit_is_one_undo_entry_with_its_own_label() {
    let (mut app, rx) = app_with_takes(3);
    seek(&mut app, MID);
    drain(&rx);

    send(
        &mut app,
        TakeMessage::SplitCompAtPlayhead { group_id: GROUP },
    );
    assert_eq!(app.test_undo_history().undo_label(), Some("split comp"));

    send(
        &mut app,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, MID),
        },
    );
    assert_eq!(app.test_undo_history().undo_label(), Some("promote take"));

    send(
        &mut app,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(1),
        },
    );
    assert_eq!(app.test_undo_history().undo_label(), Some("solo take"));

    send(
        &mut app,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 2,
        },
    );
    assert_eq!(app.test_undo_history().undo_label(), Some("delete take"));

    assert_eq!(
        app.test_undo_history().test_undo_entries().len(),
        4,
        "four edits, four atomic entries"
    );
}
