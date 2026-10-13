//! `vocal.refit_melody` — move a vocal lane's pitches onto its section's
//! current chords, keeping rhythm, note count, lyrics and holds
//! (vocal-melody-on-chord-change, BUG-70). The GUI's lane badge sends the
//! same `VocalMelodyMsg::Refit`.

use crate::compose::vocal_melody::VocalMelodyMsg;
use crate::compose::ComposeMessage;
use crate::message::Message;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::vocal as proto;
use resonance_control::{Request, Response, RpcError};

use super::super::reply::reject;
use super::super::view_model;

pub(super) fn refit_melody(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: proto::RefitMelodyParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let track_id: u64 = params.track_id.into();
    let definition_id = match super::resolve_vocal_lane(app, track_id, params.section_id) {
        Ok(id) => id,
        Err(e) => return reject(request, e),
    };
    let note_count = view_model::lane_note_count(app, definition_id, track_id);
    if note_count == 0 {
        return reject(
            request,
            RpcError::invalid_params(
                "the lane has no notes to re-fit; generate them with vocal.generate or write \
                 them with notes.*",
            ),
        );
    }
    let before = crate::update::compose::vocal_melody::lane_pitches(app, definition_id, track_id);
    let task = super::super::run_via_update(
        app,
        Message::Compose(ComposeMessage::VocalMelody(VocalMelodyMsg::Refit {
            definition_id,
            track_id,
        })),
    );
    let after = crate::update::compose::vocal_melody::lane_pitches(app, definition_id, track_id);
    let result = proto::RefitMelodyResult {
        revision: app.revision(),
        section_id: definition_id.into(),
        note_count,
        notes_changed: before.iter().zip(&after).filter(|(a, b)| a != b).count(),
    };
    (super::super::success(request, &result), task)
}
