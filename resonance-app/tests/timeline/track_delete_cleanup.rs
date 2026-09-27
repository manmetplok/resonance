//! Deleting a track takes everything that names it (code review STATE-05).
//!
//! `TrackRemoved` pruned sends, key routes, audio clips and sub-tracks,
//! but left the track's MIDI clips (the engine's `RemoveTrack` does not
//! drop them either), its automation lanes and its group memberships.
//! All of that was saved, and after a reload the engine hands the deleted
//! (highest) id to the next new track — which then inherited the old
//! notes, fader automation and group mute.

use resonance_app::message::{Message, TrackMessage};
use resonance_app::state::MidiClipState;
use resonance_app::Resonance;
use resonance_audio::types::{AudioCommand, AudioEvent, TrackType};
use resonance_common::group_identity::GroupIdentityColor;
use resonance_common::{AutomationLane, AutomationTarget, Breakpoint, CurveKind};

const KEEP: u64 = 1;
const GONE: u64 = 5;
const GROUP: u64 = 1_000_000_000;

#[test]
fn a_removed_track_leaves_no_clip_lane_or_group_member_behind() {
    let (mut app, _task, rx) = Resonance::new_for_test_with_capture();
    app.test_set_active_project(true);
    app.test_add_track(KEEP, TrackType::Instrument);
    app.test_add_track(GONE, TrackType::Instrument);
    app.test_push_midi_clip(MidiClipState {
        id: 11,
        track_id: GONE,
        start_sample: 0,
        duration_ticks: 3840,
        name: "gone".into(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    app.test_push_midi_clip(MidiClipState {
        id: 12,
        track_id: KEEP,
        start_sample: 0,
        duration_ticks: 3840,
        name: "kept".into(),
        notes: Vec::new().into(),
        trim_start_ticks: 0,
        trim_end_ticks: 0,
    });
    for (id, target) in [
        (1, AutomationTarget::TrackGain(GONE)),
        (2, AutomationTarget::TrackGain(KEEP)),
    ] {
        app.test_apply_engine_event(AudioEvent::AutomationLaneChanged {
            lane: AutomationLane::new(id, target, vec![Breakpoint::new(0, 0.5, CurveKind::Linear)]),
        });
    }
    app.test_track_groups_mut()
        .add_group_new(GROUP, "Group", GroupIdentityColor::all()[0]);
    app.test_track_groups_mut().add_member(GROUP, KEEP);
    app.test_track_groups_mut().add_member(GROUP, GONE);
    while rx.try_recv().is_ok() {}

    let _ = app.update(Message::Track(TrackMessage::RequestRemoveTrack(GONE)));
    let _ = app.update(Message::Track(TrackMessage::ConfirmRemoveTrack));
    app.test_apply_engine_event(AudioEvent::TrackRemoved { track_id: GONE });

    let file = app.test_build_project_file();
    assert!(
        file.midi_clips.iter().all(|c| c.track_id != GONE),
        "no MIDI clip of the deleted track is saved"
    );
    assert!(file.midi_clips.iter().any(|c| c.track_id == KEEP), "other clips stay");
    assert!(
        !file
            .automation_lanes
            .iter()
            .any(|l| l.target == AutomationTarget::TrackGain(GONE)),
        "no lane of the deleted track is saved"
    );
    assert!(
        file.automation_lanes
            .iter()
            .any(|l| l.target == AutomationTarget::TrackGain(KEEP)),
        "other lanes stay"
    );
    let group = file.track_groups.iter().find(|g| g.id == GROUP).expect("group");
    assert_eq!(group.ordered_members, vec![KEEP], "the deleted id leaves its group");

    let sent: Vec<AudioCommand> = rx.try_iter().collect();
    assert!(
        sent.iter()
            .any(|c| matches!(c, AudioCommand::DeleteMidiClip { clip_id: 11 })),
        "the engine keeps MIDI clips on RemoveTrack, so they are deleted explicitly"
    );
    assert!(
        sent.iter().any(|c| matches!(
            c,
            AudioCommand::ClearAutomationLane { target: AutomationTarget::TrackGain(t) } if *t == GONE
        )),
        "the engine's lane is cleared too"
    );
}
