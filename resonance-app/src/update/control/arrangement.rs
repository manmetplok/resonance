//! `arrangement.*` control handlers (ba doc #275 P2): insert / remove
//! bars, moving everything after the cut in one undoable edit.
//!
//! The thinking behind the edit lives in `update::arrangement`; this file
//! is the wire boundary — validate the bar range, refuse a destructive
//! removal without `confirm`, dispatch the edit through `update()` so it
//! records exactly one undo entry, and report what actually moved.

use crate::message::{ArrangementMessage, Message};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::arrangement::{
    self as proto, InsertBarsParams, RemoveBarsParams, ShiftResult,
};
use resonance_control::{Request, Response, RpcError};

use super::reply::reject;

/// Handle an `arrangement.*` request, or `None` when `method` belongs to
/// another namespace.
pub(super) fn try_handle(
    app: &mut Resonance,
    request: &Request,
) -> Option<(Response, Task<Message>)> {
    let handled = match request.method.as_str() {
        proto::INSERT_BARS => insert_bars(app, request),
        proto::REMOVE_BARS => remove_bars(app, request),
        _ => return None,
    };
    Some(handled)
}

/// Bars are 1-based on the wire and a shift of zero bars is a no-op the
/// caller almost certainly did not mean, so both are rejected rather than
/// silently clamped. The whole span must also end within
/// [`resonance_control::MAX_BARS`]: the bar arithmetic behind the edit is
/// plain `u32`, and a wrapped span skipped `remove_bars`' confirm gate
/// (CTL-05).
fn check_range(at_bar: u32, count: u32) -> Result<(), RpcError> {
    if at_bar == 0 {
        return Err(RpcError::invalid_params(
            "bars are 1-based; at_bar must be at least 1",
        ));
    }
    if count == 0 {
        return Err(RpcError::invalid_params("count must be at least 1"));
    }
    resonance_control::check_max_bars("at_bar", at_bar)?;
    resonance_control::check_max_bars("count", count)?;
    // Both are bounded, so the sum cannot overflow.
    resonance_control::check_max_bars("the span's last bar", at_bar + count - 1)?;
    Ok(())
}

/// Run the edit through `update()` and report the tally it left behind.
fn run(
    app: &mut Resonance,
    request: &Request,
    message: ArrangementMessage,
    at_bar: u32,
    count: u32,
) -> (Response, Task<Message>) {
    app.last_arrangement_shift = None;
    let task = super::run_via_update(app, Message::Arrangement(message));
    let outcome = app.last_arrangement_shift.take().unwrap_or_default();
    let result = ShiftResult {
        at_bar,
        count,
        shift_samples: outcome.shift_samples,
        audio_clips_moved: outcome.audio_clips_moved,
        midi_clips_moved: outcome.midi_clips_moved,
        placements_moved: outcome.placements_moved,
        markers_moved: outcome.markers_moved,
        automation_points_moved: outcome.automation_points_moved,
        tempo_events_moved: outcome.tempo_events_moved,
        signature_events_moved: outcome.signature_events_moved,
        tempo_events_removed: outcome.tempo_events_removed,
        signature_events_removed: outcome.signature_events_removed,
        clips_deleted: outcome
            .clips_deleted
            .iter()
            .map(|id| resonance_control::ids::ClipId(*id))
            .collect(),
        placements_deleted: outcome.placements_deleted,
        revision: app.revision(),
    };
    (super::success(request, &result), task)
}

fn insert_bars(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: InsertBarsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if let Err(e) = check_range(params.at_bar, params.count) {
        return reject(request, e);
    }
    run(
        app,
        request,
        ArrangementMessage::InsertBars {
            at_bar: params.at_bar,
            count: params.count,
        },
        params.at_bar,
        params.count,
    )
}

fn remove_bars(app: &mut Resonance, request: &Request) -> (Response, Task<Message>) {
    let params: RemoveBarsParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    if let Err(e) = check_range(params.at_bar, params.count) {
        return reject(request, e);
    }

    // Anything that STARTS inside the removed span goes with it. That is
    // data loss, so it follows the same confirm convention as
    // `track.delete`: say exactly what would go, then require the caller
    // to ask again.
    let casualties =
        crate::update::arrangement::removal_casualties(app, params.at_bar, params.count);
    if !params.confirm && !casualties.is_empty() {
        return reject(
            request,
            RpcError::needs_confirmation(format!(
                "removing bars {}..{} deletes {} audio clip(s), {} MIDI clip(s) and {} section \
                 placement(s) that start inside them; re-send with \"confirm\": true",
                params.at_bar,
                params.at_bar.saturating_add(params.count - 1),
                casualties.audio_clips.len(),
                casualties.midi_clips.len(),
                casualties.placements.len(),
            )),
        );
    }

    run(
        app,
        request,
        ArrangementMessage::RemoveBars {
            at_bar: params.at_bar,
            count: params.count,
        },
        params.at_bar,
        params.count,
    )
}
