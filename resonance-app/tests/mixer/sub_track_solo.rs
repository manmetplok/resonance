//! The S button on a sub-track strip (BUG-02).
//!
//! A kit tap's solo is a real solo now: the engine counts it and plays
//! only the soloed taps (`resonance-audio/tests/mixer/sub_track_solo.rs`).
//! The app's half is to hand the engine the tap's own flag — `ToggleSolo`
//! on a sub-track id must flip that sub-track and send `SetTrackSolo` for
//! it, leaving the parent alone.

use resonance_app::message::{Message, TrackMessage};
use resonance_app::state::{SubTrackLink, TrackState};
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, TrackType};

const KIT: u64 = 1;
const SNARE: u64 = 900;

fn soloed(app: &Resonance, id: u64) -> bool {
    app.test_registry().tracks.iter().find(|t| t.id == id).unwrap().soloed
}

#[test]
fn toggling_a_sub_track_solo_sends_the_taps_own_flag() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_add_track(KIT, TrackType::Instrument);
    let mut snare = TrackState::new_instrument(SNARE, 1);
    snare.sub_track = Some(SubTrackLink {
        parent_track_id: KIT,
        output_port_index: 1,
    });
    app.test_push_track(snare);
    while rx.try_recv().is_ok() {}

    let _ = app.update(Message::Track(TrackMessage::ToggleSolo(SNARE)));
    assert!(soloed(&app, SNARE), "the tap's own flag flips");
    assert!(!soloed(&app, KIT), "the parent's does not");
    let sent: Vec<_> = rx
        .try_iter()
        .filter_map(|cmd| match cmd {
            AudioCommand::SetTrackSolo { track_id, soloed } => Some((track_id, soloed)),
            _ => None,
        })
        .collect();
    assert_eq!(sent, vec![(SNARE, true)], "the engine is told about the tap alone");

    let _ = app.update(Message::Track(TrackMessage::ToggleSolo(SNARE)));
    assert!(!soloed(&app, SNARE));
    assert!(
        rx.try_iter()
            .any(|cmd| matches!(cmd, AudioCommand::SetTrackSolo { track_id: SNARE, soloed: false })),
        "un-soloing the tap reaches the engine too"
    );
}
