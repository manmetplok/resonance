//! The `vocal_melody` choice on every control call that can change a
//! section's chords or length (`harmony.*`, `section.resize`), and the
//! `vocal_lanes_affected` its reply carries (vocal-melody-on-chord-change).

use crate::compose::vocal_melody::AffectedVocalLane;
use crate::compose::VocalMelodyChoice;
use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::{VocalLaneAffected, VocalMelodyChoice as WireChoice};

/// The app's choice for a wire `vocal_melody`.
pub(in crate::update::control) fn from_wire(choice: WireChoice) -> VocalMelodyChoice {
    match choice {
        WireChoice::Keep => VocalMelodyChoice::Keep,
        WireChoice::Refit => VocalMelodyChoice::Refit,
        WireChoice::Regenerate => VocalMelodyChoice::Regenerate,
    }
}

fn to_wire(choice: VocalMelodyChoice) -> WireChoice {
    match choice {
        VocalMelodyChoice::Keep => WireChoice::Keep,
        VocalMelodyChoice::Refit => WireChoice::Refit,
        VocalMelodyChoice::Regenerate => WireChoice::Regenerate,
    }
}

fn affected_to_wire(lanes: Vec<AffectedVocalLane>) -> Vec<VocalLaneAffected> {
    let mut wire: Vec<VocalLaneAffected> = Vec::with_capacity(lanes.len());
    for lane in lanes {
        // A call that edits twice (none today) reports each lane once,
        // with its last outcome.
        wire.retain(|w| {
            !(u64::from(w.definition_id) == lane.definition_id
                && u64::from(w.track_id) == lane.track_id)
        });
        wire.push(VocalLaneAffected {
            definition_id: lane.definition_id.into(),
            track_id: lane.track_id.into(),
            outcome: to_wire(lane.outcome),
        });
    }
    wire
}

/// Dispatch `message` through the full update path with `choice` as what
/// an affected vocal lane gets (`None`: the project's remembered choice,
/// else keep), and report what each affected lane got.
pub(in crate::update::control) fn run_with_vocal_melody(
    app: &mut Resonance,
    choice: Option<WireChoice>,
    message: Message,
) -> (Task<Message>, Vec<VocalLaneAffected>) {
    let (task, affected) = crate::update::compose::vocal_melody::with_requested_choice(
        app,
        choice.map(from_wire),
        |app| super::super::run_via_update(app, message),
    );
    (task, affected_to_wire(affected))
}
