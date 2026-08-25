//! Deleting a take, end to end: the app's gesture → the `AudioCommand`s it
//! emits → the **real** engine handlers → samples (ba todo #1401, epic #15,
//! docs #165 / #292).
//!
//! # Why these cases render audio
//!
//! `take_comp_edits.rs` pins what the app-side mirror becomes and what the
//! undo history did. That is not enough for a deletion, for the reason ba
//! doc #292 spells out: a take's recording is an ordinary `AudioClip` in
//! the engine's shared clip list, and it is inaudible only because the
//! published comp table marks it *governed*. A structural test sees a tidy
//! lane and a comp with no dangling segment in every one of the following
//! failure modes:
//!
//! - the app refuses the gesture (the pre-#1401 last-take behaviour) and
//!   the "deleted" take keeps playing;
//! - the app sends a `SetTakeComp` instead of `RemoveTake`, so the engine
//!   still holds the take and nothing parks its recording;
//! - an undo restores the take to the lane but never un-parks its clip, so
//!   it comes back on screen and silent;
//! - a redo drops the take again but leaves the clip registered and
//!   un-governed, so it plays raw at full gain.
//!
//! Every case here therefore replays what the app actually told the engine
//! onto [`EngineHandlerHarness`] and asks `render_track` what the user
//! hears. Each take's recording is DC at a level of its own, so a probe
//! says not merely *whether* something plays but *which pass* it is.
//!
//! The engine-side handlers have their own coverage in
//! `resonance-audio/tests/take_removal.rs`. What is new here is the seam
//! those cases cannot reach: whether the app sends the commands at all.

use resonance_app::message::{Message, TakeMessage};
use resonance_app::Resonance;
use resonance_audio::__test_support::{EngineHandlerHarness, Receiver};
use resonance_audio::types::*;
use resonance_common::{TakeContent, TimelineRange};

const TRACK: u64 = 7;
const GROUP: u64 = 1;

/// Short enough to render three times a test, long enough that the 96-frame
/// edge declick and the ±128-frame seam crossfade leave wide stretches of
/// untouched, full-gain audio to probe.
const SLOT: TimelineRange = TimelineRange {
    start: 0,
    length: 2000,
};
const MID: u64 = SLOT.length / 2;
/// Anti-click ramp at the comp's outer edges (`CLIP_DECLICK_FRAMES`).
const DECLICK: usize = 96;
/// Half a seam crossfade (`COMP_XFADE_FRAMES / 2`).
const SEAM_HALF: usize = 128;

/// Take *n*'s recording is DC at `LEVELS[n]` — three levels far enough
/// apart that a probe identifies the pass, and no two of them sum to a
/// third.
const LEVELS: [f32; 3] = [1.0, 0.5, 0.25];

fn clip_ref_of(take_id: u64) -> ClipId {
    5_000 + take_id
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A clip whose stereo PCM is the constant `value` over the whole slot.
fn const_clip(id: ClipId, value: f32) -> AudioClip {
    AudioClip {
        id,
        track_id: TRACK,
        start_sample: SLOT.start,
        source: ClipSource::Memory(vec![value; SLOT.length as usize * 2]),
        name: "take".into(),
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

/// An app with a saved project (the undo gate needs a path), `n` captured
/// audio passes on one lane, and a command-capturing engine — plus a real
/// engine harness holding the same lane and the same recordings.
///
/// The harness is seeded through `RestoreTakeGroups` from the app's own
/// mirror, which is exactly how a project load puts a lane into the engine
/// (todo #1394), so the two start out agreeing by construction rather than
/// by a hand-written fixture that could drift from what the app believes.
fn app_and_engine(n: u64) -> (Resonance, Receiver<AudioCommand>, EngineHandlerHarness) {
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(std::path::PathBuf::from("/tmp/resonance-take-removal-test"));
    app.test_add_track(TRACK, TrackType::Audio);
    let rx = app.test_capture_engine();

    let mut h = EngineHandlerHarness::new();
    for take_id in 0..n {
        app.test_apply_engine_event(AudioEvent::TakeCaptured {
            group_id: GROUP,
            take_id,
            track_id: TRACK,
            slot: SLOT,
            pass_index: take_id as u32,
            extent: SLOT,
            content: TakeContent::Audio {
                clip_ref: clip_ref_of(take_id),
            },
        });
        h.push_clip(const_clip(clip_ref_of(take_id), LEVELS[take_id as usize]));
    }
    h.restore_take_groups(app.test_take_groups().to_vec());
    drain(&rx);
    (app, rx, h)
}

fn drain(rx: &Receiver<AudioCommand>) -> Vec<AudioCommand> {
    let mut cmds = Vec::new();
    while let Ok(cmd) = rx.try_recv() {
        cmds.push(cmd);
    }
    cmds
}

/// Run every take-lane command the app emitted through the engine's own
/// handlers, in the order it emitted them.
///
/// Unrelated traffic is ignored rather than rejected: an undo drives a
/// whole diff replay past the receiver (`Stop`, fader pushes, …) and none
/// of it bears on what a take lane plays. Every command that *does* is
/// listed, so a future variant the app starts sending is a compile error
/// here rather than a silently skipped step.
fn replay(h: &mut EngineHandlerHarness, cmds: &[AudioCommand]) {
    for cmd in cmds {
        match cmd {
            AudioCommand::SetTakeComp { group_id, segments } => {
                h.set_take_comp(*group_id, segments.clone())
            }
            AudioCommand::SetActiveTake { group_id, take_id } => {
                h.set_active_take(*group_id, *take_id)
            }
            AudioCommand::RemoveTake { group_id, take_id } => h.remove_take(*group_id, *take_id),
            AudioCommand::RemoveTakeGroup { group_id } => h.remove_take_group(*group_id),
            AudioCommand::RestoreTakeGroups { groups } => h.restore_take_groups(groups.clone()),
            _ => {}
        }
    }
}

/// Dispatch `m`, then push everything it made the app say to the engine.
fn send(
    app: &mut Resonance,
    rx: &Receiver<AudioCommand>,
    h: &mut EngineHandlerHarness,
    m: TakeMessage,
) {
    let _ = app.update(Message::Take(m));
    replay(h, &drain(rx));
}

/// Dispatch a history step, then push its engine traffic through too.
fn history(
    app: &mut Resonance,
    rx: &Receiver<AudioCommand>,
    h: &mut EngineHandlerHarness,
    m: Message,
) {
    let _ = app.update(m);
    replay(h, &drain(rx));
}

/// What the track plays across the whole slot.
fn render(h: &EngineHandlerHarness) -> Vec<f32> {
    h.render_track(TRACK, SLOT.start, SLOT.length as usize)
}

fn peak(out: &[f32]) -> f32 {
    out.iter().fold(0.0f32, |acc, s| acc.max(s.abs()))
}

/// Assert `out` is silence — nothing anywhere in the block.
///
/// A peak check rather than a probe: an un-parked recording plays on the
/// ordinary clip path, which is bound to no comp segment at all, so it can
/// surface anywhere in the block.
fn assert_silent(out: &[f32], what: &str) {
    let p = peak(out);
    assert!(p < 1e-4, "{what}: expected silence, peak was {p}");
}

/// Assert the block plays `level` at `frame`, well clear of any ramp.
fn assert_plays(out: &[f32], frame: usize, level: f32, what: &str) {
    assert!(
        (out[frame] - level).abs() < 1e-6,
        "{what}: frame {frame} should be {level}, got {}",
        out[frame]
    );
}

/// Assert the block never rises above `ceiling` — the level of the loudest
/// thing that is *supposed* to be playing.
///
/// A ceiling rather than "no sample equals the removed take's level",
/// because an un-parked recording does not replace the comp, it **sums**
/// with it on the ordinary clip path: the symptom doc #292 reproduced is a
/// deleted take making the lane *louder* (peak 1.25 against 1.0). A ceiling
/// catches that wherever in the block it surfaces, and catches a removed
/// take that is somehow still selected too.
fn assert_nothing_louder_than(out: &[f32], ceiling: f32, what: &str) {
    let p = peak(out);
    assert!(
        p <= ceiling + 1e-4,
        "{what}: peak {p} is above the {ceiling} the survivors should be playing — \
         a removed take is still in the render"
    );
}

/// The take ids the app's mirror holds for the lane, or `None` when the
/// lane is gone.
fn mirrored_takes(app: &Resonance) -> Option<Vec<u64>> {
    let group = app.test_take_groups().iter().find(|g| g.id == GROUP)?;
    Some(group.takes.iter().map(|t| t.id).collect())
}

/// The take ids the *engine* holds for the lane, or `None` when the lane is
/// gone from its store.
fn engine_takes(h: &EngineHandlerHarness) -> Option<Vec<u64>> {
    Some(h.take_group(GROUP)?.takes.iter().map(|t| t.id).collect())
}

// ---------------------------------------------------------------------------
// The acceptance case: a group's last take
// ---------------------------------------------------------------------------

/// **Deleting a group's last take removes the lane and leaves nothing
/// audible.**
///
/// This is the case the app refused outright before todo #1401, and the
/// refusal is exactly what the render catches: a lane the user deleted that
/// carries on playing its only pass.
#[test]
fn deleting_a_lone_take_removes_the_lane_and_silences_it() {
    let (mut app, rx, mut h) = app_and_engine(1);

    assert_plays(
        &render(&h),
        DECLICK + 100,
        LEVELS[0],
        "fixture check: the lone take should be playing",
    );

    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    assert_silent(&render(&h), "the deleted lane");
    assert_eq!(
        mirrored_takes(&app),
        None,
        "the lane is gone from the mirror"
    );
    assert_eq!(engine_takes(&h), None, "and from the engine's store");
}

/// Undo brings the lane back **audibly**, not merely visibly.
///
/// The take's recording was parked out of the render, so restoring the lane
/// has to un-park it. An undo that only rebuilt the mirror would draw the
/// card and play silence under it — which is why this asserts a level
/// rather than a card.
#[test]
fn undoing_the_deletion_brings_the_lane_back_audibly() {
    let (mut app, rx, mut h) = app_and_engine(1);
    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );
    assert_silent(&render(&h), "the deleted lane");

    history(&mut app, &rx, &mut h, Message::Undo);

    assert_eq!(
        mirrored_takes(&app),
        Some(vec![0]),
        "the take is back on screen"
    );
    assert_eq!(engine_takes(&h), Some(vec![0]), "and back in the engine");
    assert_plays(
        &render(&h),
        DECLICK + 100,
        LEVELS[0],
        "the restored take must be audible, not a silent card",
    );
}

/// And redo removes them again — the half that is easy to miss, because
/// `RestoreTakeGroups` has to *re*-park a recording it previously un-parked
/// or the redone deletion plays the take raw at full gain.
#[test]
fn redoing_the_deletion_silences_the_lane_again() {
    let (mut app, rx, mut h) = app_and_engine(1);
    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );
    history(&mut app, &rx, &mut h, Message::Undo);
    assert_plays(
        &render(&h),
        DECLICK + 100,
        LEVELS[0],
        "fixture check: undone",
    );

    history(&mut app, &rx, &mut h, Message::Redo);

    assert_eq!(mirrored_takes(&app), None, "the lane is gone again");
    assert_silent(&render(&h), "the redone deletion");
}

// ---------------------------------------------------------------------------
// Deleting down to nothing
// ---------------------------------------------------------------------------

/// Every take deleted in turn: at each step the survivors are audible at
/// their own level, the deleted passes are gone, and the last one takes the
/// lane with it.
///
/// The most user-real shape of the gesture, and the one that catches a
/// removal that empties the group without dropping it: an empty group falls
/// back to its most recent pass, so the take the user just deleted would
/// carry on playing.
#[test]
fn deleting_every_take_in_turn_ends_in_silence() {
    let (mut app, rx, mut h) = app_and_engine(3);
    let probe = DECLICK + 100;

    // Take 2 is the newest pass, so it is what covers the un-comped slot.
    assert_plays(&render(&h), probe, LEVELS[2], "fixture check");

    for (take_id, survivor) in [(2, LEVELS[1]), (1, LEVELS[0])] {
        send(
            &mut app,
            &rx,
            &mut h,
            TakeMessage::DeleteTake {
                group_id: GROUP,
                take_id,
            },
        );
        let out = render(&h);
        assert_plays(
            &out,
            probe,
            survivor,
            &format!("after deleting take {take_id}, the next-newest pass covers the slot"),
        );
        assert_nothing_louder_than(&out, survivor, &format!("after deleting take {take_id}"));
    }

    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    assert_silent(&render(&h), "the emptied lane");
    assert_eq!(mirrored_takes(&app), None);
    assert_eq!(engine_takes(&h), None);
}

// ---------------------------------------------------------------------------
// Removing one take of several
// ---------------------------------------------------------------------------

/// A non-last removal reaches the engine as a removal, not as a comp edit.
///
/// The audible symptom is not the point here — a take the engine still
/// holds stays governed, and therefore stays silent, which is exactly why
/// the pre-#1401 `SetTakeComp` papered over the bug. What the render *does*
/// guard is the trap under it: once the take is genuinely gone, nothing
/// governs its recording, and it plays raw at full gain unless the engine
/// parks it. That is why the app has to send the command rather than a
/// comp.
#[test]
fn removing_one_take_of_several_takes_its_recording_out_of_the_render() {
    let (mut app, rx, mut h) = app_and_engine(3);
    // Promote take 0 over the first half, so it is genuinely audible there
    // and the removal has a hole to re-cover.
    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, MID),
        },
    );
    assert_plays(
        &render(&h),
        DECLICK + 100,
        LEVELS[0],
        "fixture check: take 0 covers the first half",
    );

    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    assert_eq!(
        engine_takes(&h),
        Some(vec![1, 2]),
        "the engine drops the take — a comp edit would have left it in the store"
    );
    assert!(
        !h.clip_ids().contains(&clip_ref_of(0)),
        "and its recording leaves the render's input"
    );
    assert!(
        h.parked_clip_ids().contains(&clip_ref_of(0)),
        "parked, not deleted — the undo has to bring the audio back"
    );

    let out = render(&h);
    // The survivor the cover now falls back to inherits take 0's half...
    for probe in [DECLICK + 100, MID as usize + SEAM_HALF] {
        assert_plays(&out, probe, LEVELS[2], "the survivor covers the whole slot");
    }
    // ...and take 0's recording is nowhere in the block. Un-parked it would
    // sum onto the comp at 1.0, which is the doc #292 reproduction: the
    // deleted take coming back *louder* than it was.
    assert_nothing_louder_than(&out, LEVELS[2], "after the removal");
}

/// A comp flattened to `(start, end, take)` per segment.
type FlatComp = Vec<(u64, u64, u64)>;

/// The mirror's comp beside the engine's, both flattened for comparison.
fn comps(app: &Resonance, h: &EngineHandlerHarness) -> (FlatComp, FlatComp) {
    let flatten = |segs: &[resonance_common::CompSegment]| {
        segs.iter()
            .map(|s| (s.range.start, s.range.end(), s.take_id))
            .collect::<Vec<_>>()
    };
    (
        flatten(&app.test_take_groups()[0].comp.segments),
        flatten(
            &h.take_group(GROUP)
                .expect("the lane survives")
                .comp
                .segments,
        ),
    )
}

/// The mirror and the engine hold the **same** re-covered comp after a
/// removal — the whole point of the app delegating to the shared
/// `TakeGroup::remove_take` instead of re-deriving the cover.
///
/// The deleted take's half goes to the take the cover now falls back to,
/// on both sides. A mirror that merely dropped the take would leave a
/// segment naming it and draw a hole where the engine plays the survivor.
#[test]
fn the_mirror_and_the_engine_agree_on_a_re_covered_comp() {
    let (mut app, rx, mut h) = app_and_engine(3);
    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, MID),
        },
    );

    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    let (mirrored, engine) = comps(&app, &h);
    assert_eq!(
        mirrored, engine,
        "one definition of a removal, or the lane draws a comp the engine is not playing"
    );
    assert_eq!(
        mirrored,
        vec![(SLOT.start, SLOT.end(), 2)],
        "the survivor inherits the hole, merged into one span"
    );
}

/// A lane that was never comped does **not** acquire a comp by being
/// deleted from.
///
/// The sharp end of the delegation. The app's removed `cover_without`
/// started from `effective_comp()`, so it materialized a full explicit
/// cover here — segments every UI draws as *"you promoted this"* — while
/// the engine's group stayed un-comped. Both are equally audible (tier 3
/// covers the slot with the same take either way), which is exactly why
/// only a comparison against the engine catches it.
#[test]
fn deleting_from_an_un_comped_lane_leaves_it_un_comped() {
    let (mut app, rx, mut h) = app_and_engine(3);

    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 2,
        },
    );

    let (mirrored, engine) = comps(&app, &h);
    assert_eq!(mirrored, engine);
    assert!(
        mirrored.is_empty(),
        "deleting a take is not a promotion of the rest: {mirrored:?}"
    );
}

/// Deleting the soloed take clears the solo on both sides, and the lane
/// falls back to the survivors rather than going silent.
#[test]
fn deleting_the_soloed_take_clears_the_solo_in_the_engine_too() {
    let (mut app, rx, mut h) = app_and_engine(3);
    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::SetActiveTake {
            group_id: GROUP,
            take_id: Some(0),
        },
    );
    assert_plays(
        &render(&h),
        DECLICK + 100,
        LEVELS[0],
        "fixture check: take 0 is soloed across the slot",
    );

    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    assert_eq!(app.test_take_groups()[0].active_take, None);
    assert_eq!(
        h.take_group(GROUP).expect("the lane survives").active_take,
        None,
        "an engine still soloing a take it no longer holds draws a solo that solos nothing"
    );
    let out = render(&h);
    assert_plays(
        &out,
        DECLICK + 100,
        LEVELS[2],
        "the lane plays its newest surviving pass, not silence",
    );
    assert_nothing_louder_than(&out, LEVELS[2], "after deleting the soloed take");
}

/// Undo restores a non-last take's **audio**, not just its card.
#[test]
fn undoing_a_single_take_removal_restores_its_audio() {
    let (mut app, rx, mut h) = app_and_engine(3);
    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::PromoteTakeSegment {
            group_id: GROUP,
            take_id: 0,
            range: TimelineRange::from_bounds(SLOT.start, MID),
        },
    );
    send(
        &mut app,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );
    assert_nothing_louder_than(&render(&h), LEVELS[2], "fixture check: take 0 is gone");

    history(&mut app, &rx, &mut h, Message::Undo);

    assert_eq!(mirrored_takes(&app), Some(vec![0, 1, 2]));
    assert!(
        h.clip_ids().contains(&clip_ref_of(0)),
        "the parked recording is back in the render's input"
    );
    assert_plays(
        &render(&h),
        DECLICK + 100,
        LEVELS[0],
        "and the restored take is audible where it was promoted",
    );
}

// ---------------------------------------------------------------------------
// A known-open defect this todo makes reachable (see the follow-up in ba)
// ---------------------------------------------------------------------------

/// **`#[ignore]`d because it fails: it reproduces an open engine-side
/// defect, not a regression in this todo.** Un-ignore it when the fix
/// lands; it is written to go green on the fix and on nothing else.
///
/// ```text
/// cargo test -p resonance-app --test timeline -- --ignored take_clip_load
/// ```
///
/// # The race
///
/// Todo #1402 made a project load restore a take's *audio* as well as its
/// group: `replay_take_groups` sends `RestoreTakeGroups` and then one
/// `LoadTakeClipFromWav` per audio take. That load is **asynchronous** —
/// the engine handler only submits it to a worker pool, and the
/// `AudioClip` reaches `ctx.clips` some milliseconds later.
///
/// `park_take_clip` is what makes a removal silent, and it works by taking
/// the clip *out of* `ctx.clips`. A `RemoveTake` handled while the load is
/// still in flight therefore finds nothing to park and parks nothing — and
/// the worker then publishes an `AudioClip` that no comp table governs,
/// because the take it belonged to is gone. It plays raw, at full gain, on
/// the ordinary clip path, on top of the comp: the exact "deleting a take
/// makes it **louder**" shape ba doc #292 records at peak 1.25 against 1.0.
///
/// # Why the app cannot fix it, and this is filed rather than worked around
///
/// The obvious app-side answer — refuse a removal while a load is
/// outstanding — cannot be written, because **the app has no way to know
/// one is outstanding**. `LoadTakeClipFromWav` echoes nothing back by
/// design (#1402's `ClipLoadEcho::SilentTake`: "the take-group restore is
/// sender and mirror both"), so no counter the app could keep would ever
/// be decremented. `io.loading` is false long before the workers finish —
/// it is cleared when the app's replay returns, which is when the loads
/// are *submitted*. Nothing else in `gates_message` knows about clip loads
/// either. A heuristic time window would refuse legitimate gestures and
/// still miss slow ones.
///
/// The fix has to be where the knowledge is: the publish step in
/// `submit_clip_load` has to be able to see that the take was removed
/// (a tombstone the park leaves, or a claim check shared with the worker).
/// That is `resonance-audio`, and it is filed there.
///
/// # Why this test is here rather than in the engine's suite
///
/// It is *this todo* that makes the race reachable — before it, nothing in
/// the app sent `RemoveTake` at all — so the reproduction drives the app's
/// real message path: a real save, a real reload, and a `DeleteTake`
/// dispatched through `update` the instant the lane appears, with nothing
/// waited on in between. That is not a contrived interleaving; it is a
/// user right-clicking a take card as soon as they can see it.
#[test]
#[ignore = "reproduces an open engine-side defect: a removal racing a take-clip load parks nothing"]
fn a_removal_racing_a_take_clip_load_leaves_the_take_audible() {
    let dir = std::env::temp_dir().join(format!(
        "resonance-take-removal-race-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create project dir");

    // An authored project: two audio takes over one slot, each backed by a
    // real DC-valued WAV, saved to disk.
    let (mut app, _task) = Resonance::new_for_test();
    app.test_set_active_project(true);
    app.test_set_project_path(dir.clone());
    app.test_add_track(TRACK, TrackType::Audio);
    let audio_dir = dir.join("audio");
    std::fs::create_dir_all(&audio_dir).expect("create audio dir");
    for take_id in 0..2u64 {
        let level = LEVELS[take_id as usize];
        resonance_audio::transcode_to_wav(
            &audio_dir.join(format!("clip_{}.wav", clip_ref_of(take_id))),
            &vec![level; SLOT.length as usize * 2],
            48_000,
        )
        .expect("write take wav");
        app.test_apply_engine_event(AudioEvent::TakeCaptured {
            group_id: GROUP,
            take_id,
            track_id: TRACK,
            slot: SLOT,
            pass_index: take_id as u32,
            extent: SLOT,
            content: TakeContent::Audio {
                clip_ref: clip_ref_of(take_id),
            },
        });
    }
    let file = app.test_build_project_file();
    resonance_app::project::save_project(&dir, &file, &[], &[]).expect("save project");
    let loaded = resonance_app::project::load_project(&dir).expect("load project");

    // Reopen it. The replay emits `RestoreTakeGroups` and then a
    // `LoadTakeClipFromWav` per take; the loads are submitted, not done.
    let (mut reopened, _task) = Resonance::new_for_test();
    reopened.test_set_active_project(true);
    reopened.test_set_project_path(dir.clone());
    reopened.test_add_track(TRACK, TrackType::Audio);
    let rx = reopened.test_capture_engine();
    reopened.test_replay_loaded_project_from(loaded);

    let mut h = EngineHandlerHarness::new();
    let load_cmds = drain(&rx);
    assert!(
        load_cmds
            .iter()
            .any(|c| matches!(c, AudioCommand::LoadTakeClipFromWav { .. })),
        "fixture check: the reload must ask for the take clips"
    );
    for cmd in &load_cmds {
        h.replay_take_lane_command(cmd);
    }

    // The user right-clicks take 0's card the instant the lane is drawn.
    // Nothing is waited on: the workers are still reading their WAVs, and
    // this is the whole point.
    send(
        &mut reopened,
        &rx,
        &mut h,
        TakeMessage::DeleteTake {
            group_id: GROUP,
            take_id: 0,
        },
    );

    // Now let the loads land.
    assert!(
        h.wait_for_clips(1, std::time::Duration::from_secs(5)),
        "the take clips never loaded at all"
    );
    std::thread::sleep(std::time::Duration::from_millis(300));

    let out = h.render_track(TRACK, SLOT.start, SLOT.length as usize);
    // The audible assertion first — it is the one that states the defect.
    // Take 1 is the only survivor, so nothing may exceed its level; the
    // leaked pass sums on top of it.
    assert_nothing_louder_than(&out, LEVELS[1], "a removal that raced the take-clip load");
    // And the mechanism, as a diagnostic for whoever fixes it.
    assert!(
        !h.clip_ids().contains(&clip_ref_of(0)),
        "the removed take's recording is back in the render's input, un-parked \
         and ungoverned: clips={:?} parked={:?}",
        h.clip_ids(),
        h.parked_clip_ids()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
