//! FU-D4a: the app's startup default track ("Track 1") must reserve its
//! id synchronously, so the user's first "Add Track" can never race it.
//!
//! Before this fix the engine thread created a default track itself,
//! unprompted, always as literal id 1, before its command loop ever read
//! anything. The app's own `next_track_id` counter also starts at 1
//! (`state/tracks.rs`), and nothing in `engine_events::tracks::added`
//! bumps it — the mirror only pushes onto `registry.tracks`, which the
//! in-use scan behind `allocate_track_id` reads. So if the GUI's first
//! "Add Track" ran before the app had drained and mirrored that
//! unprompted `TrackAdded` echo, `allocate_track_id` saw an empty
//! `tracks` list, handed out id 1 again, and the engine refused the
//! resulting `AddTrack` as a collision with the track it had already
//! silently created (`EngineErrorKind::Internal`) — a click that
//! visibly did nothing but raise an error banner.
//!
//! Since ARCH-04 D-4 the app is the only track-id allocator
//! (`state/ids.rs`), so the fix routes the startup default track through
//! that same allocator (`Resonance::send_startup_default_track`, called
//! from `Resonance::new` — see its doc comment). This closes the window
//! outright rather than narrowing it: the allocation happens
//! synchronously, before iced's event loop can ever deliver a second
//! message, so nothing can race it for id 1 — proven below by driving a
//! GUI "Add Track" immediately after the startup send, before either
//! `TrackAdded` echo has been fed back at all.
//!
//! `new_for_test*` is hermetic (no real engine thread), so it never runs
//! `Resonance::new`'s startup send either; `test_send_startup_default_track`
//! drives it explicitly. `resonance-audio/tests/engine/startup_no_default_track.rs`
//! is the engine-side half: the real `engine_thread` no longer creates a
//! track unprompted at all.

use resonance_app::message::{Message, TrackMessage};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent};

#[test]
fn startup_default_track_reserves_id_1_before_any_other_add_can_run() {
    let (mut app, _task, cmd_rx) = Resonance::new_for_test_with_capture();

    app.test_send_startup_default_track();

    let sent: Vec<AudioCommand> = cmd_rx.try_iter().collect();
    assert_eq!(sent.len(), 1, "exactly one command is sent: {sent:?}");
    assert!(
        matches!(
            &sent[0],
            AudioCommand::AddTrack { id: 1, name: None }
        ),
        "the startup default track is added for id 1: {sent:?}"
    );

    // The counter is already advanced past id 1 — before the `TrackAdded`
    // echo for it has even been fed back, let alone mirrored into
    // `registry.tracks`. This is what makes the race impossible rather
    // than just unlikely: the in-use scan behind `allocate_track_id`
    // can't yet see the startup track in `tracks`, but the counter has
    // already moved past its id regardless.
    assert_eq!(app.test_registry().next_track_id, 2);
    assert!(
        app.test_registry().tracks.is_empty(),
        "the mirror only happens on the TrackAdded echo, not yet fed back"
    );

    // The exact race this bug was: the user's very first "Add Track"
    // click, dispatched before the startup track's echo has arrived.
    app.test_dispatch(Message::Track(TrackMessage::AddTrack));

    let second_sent: Vec<AudioCommand> = cmd_rx.try_iter().collect();
    assert_eq!(second_sent.len(), 1, "exactly one command is sent: {second_sent:?}");
    assert!(
        matches!(
            &second_sent[0],
            AudioCommand::AddTrack { id: 2, name: None }
        ),
        "the click must not collide with the startup track's id: {second_sent:?}"
    );

    // Feed both echoes back (order doesn't matter) and check the mirror
    // lands cleanly, with no dropped add and no duplicate-id warning path
    // taken.
    app.test_apply_engine_event(AudioEvent::TrackAdded { track_id: 1 });
    app.test_apply_engine_event(AudioEvent::TrackAdded { track_id: 2 });

    let mut ids: Vec<u64> = app.test_registry().tracks.iter().map(|t| t.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![1, 2], "both the startup track and the click's track landed");
}
