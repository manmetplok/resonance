//! `track.set_output` — where a track's post-fader audio goes.

use super::{ack, find_track, not_found_track, reject};
use crate::message::{Message, TrackMessage};
use crate::update::control::run_via_update;
use crate::Resonance;
use iced::Task;
use resonance_audio::types::TrackOutput;
use resonance_control::methods::track;
use resonance_control::{Request, Response, RpcError, TrackOutput as WireTrackOutput};

/// `track.set_output` — send a track's post-fader audio to master, or
/// through a group bus first (ba doc #273, todo #1228).
///
/// `song.summary` / `song.tracks` report the current destination in the
/// same shape, so read and write share one vocabulary.
///
/// Busses share the track id space, so "route a bus somewhere" is
/// expressible — but the engine models bus -> master only, and
/// [`find_track`] searches `registry.tracks`, which never holds a bus.
/// A bus id therefore falls out below as `not_found` before any routing
/// is attempted. There used to be an explicit "this is a bus" branch
/// after that check; it was unreachable, so it is gone (ba doc #273,
/// todo #1238 item 2).
pub(super) fn set_output(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: track::SetOutputParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0) else {
        return not_found_track(request, params.track_id.0);
    };

    let output = match params.output {
        WireTrackOutput::Master => TrackOutput::Master,
        WireTrackOutput::Bus(bus_id) => {
            if !app.registry.busses.iter().any(|b| b.id == bus_id.0) {
                let known: Vec<String> = app
                    .registry
                    .busses
                    .iter()
                    .map(|b| format!("{} ({})", b.id, b.name))
                    .collect();
                return reject(
                    request,
                    RpcError::not_found(format!(
                        "no bus with id {bus_id}; create one with bus.create. Existing busses: \
                         [{}]",
                        known.join(", ")
                    )),
                );
            }
            TrackOutput::Bus(bus_id.0)
        }
    };

    // Idempotent: re-routing somewhere it already goes records no undo
    // entry and sends no engine command.
    if t.output == output {
        return (ack(app, request), Task::none());
    }
    let task = run_via_update(
        app,
        Message::Track(TrackMessage::SetTrackOutput(params.track_id.0, output)),
    );
    (ack(app, request), task)
}
