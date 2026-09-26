//! `track.set_plugin_param` — one parameter on one plugin, and the
//! f32-bound tolerance every plugin-param setter in the control API
//! shares.

use super::{ack, find_track, frozen_reject, instance_for, not_found_track, reject};
use crate::message::{Message, PluginMessage};
use crate::update::control::{run_via_update, view_model};
use crate::Resonance;
use iced::Task;
use resonance_control::methods::track;
use resonance_control::{Request, Response, RpcError};

/// Set one parameter on one plugin (ba doc #272 V-3).
///
/// Routes the same `PluginMessage::SetPluginParam` the GUI's plugin
/// panel sends, so the edit reaches the engine and is undoable.
/// Each call is its own undo entry and one `revision` bump — successive
/// sets of the same parameter do not coalesce the way a GUI knob drag
/// does, so one `edit.undo` takes back exactly one call (code review
/// CTL-03).
///
/// Addressing mirrors `track.plugin_params`: the CLAP id `song.tracks`
/// reports, plus `occurrence` for a track carrying the same plugin
/// twice.
pub(super) fn set_plugin_param(
    app: &mut Resonance,
    request: &Request,
) -> (Response, Task<Message>) {
    let params: track::SetPluginParamParams = match request.params() {
        Ok(p) => p,
        Err(e) => return reject(request, e),
    };
    let Some(t) = find_track(app, params.track_id.0).cloned() else {
        return not_found_track(request, params.track_id.0);
    };
    // `SetPluginParam` is a frozen-input edit (gates.rs); reject rather
    // than ack an edit the gate would swallow.
    if let Some(e) = frozen_reject(app, t.id) {
        return reject(request, e);
    }

    // Resolve the plugin: an explicit id, else the track's instrument —
    // the common case for "make this synth sound different".
    let entries = view_model::plugin_entries(app, &t);
    let occurrence = params.occurrence.unwrap_or(0);
    let entry = match &params.plugin_id {
        Some(id) => entries
            .iter()
            .find(|e| &e.plugin_id == id && e.occurrence == occurrence),
        None => entries
            .iter()
            .find(|e| e.kind == track::PluginKind::Instrument),
    };
    let Some(entry) = entry else {
        let error = match &params.plugin_id {
            Some(id) => view_model::unknown_plugin_on_track(app, &t, id, occurrence),
            None => RpcError::invalid_params(format!(
                "track {} has no instrument; name a plugin_id (it carries: [{}])",
                t.id,
                entries
                    .iter()
                    .map(|e| e.plugin_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
        return reject(request, error);
    };

    // The one window `track.add_effect`'s synchronous commit (todo
    // #1234) cannot close: the slot exists, but its parameter list only
    // arrives with the engine's `PluginAdded` echo. Say that, rather
    // than falling through to "plugin X has no parameter Y (has: [])" —
    // which is what made a working add look like a failed one.
    if entry.params.is_empty() {
        return reject(
            request,
            RpcError::busy(format!(
                "plugin {:?} is on track {} but is still initializing — its parameter list \
                 arrives with the engine echo, usually within a frame. Retry, or read \
                 track.plugin_params until its params array is non-empty. (A plugin that \
                 genuinely exposes no parameters reports the same empty list.)",
                entry.plugin_id, t.id
            )),
        );
    }

    // Resolve the parameter by name first, then by numeric CLAP id — a
    // client reading `track.plugin_params` has both, and a name is what
    // it will usually have to hand.
    let wanted = params.param.trim();
    let param = entry
        .params
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            wanted
                .parse::<u32>()
                .ok()
                .and_then(|id| entry.params.iter().find(|p| p.id == id))
        });
    let Some(param) = param else {
        let known: Vec<&str> = entry.params.iter().map(|p| p.name.as_str()).collect();
        return reject(
            request,
            RpcError::not_found(format!(
                "plugin {:?} has no parameter {wanted:?} (has: [{}])",
                entry.plugin_id,
                known.join(", ")
            )),
        );
    };

    // Reject rather than clamp: silently moving a value the caller asked
    // for is how a mix ends up subtly wrong with nothing to point at.
    //
    // ...but the bounds themselves are f64 renderings of f32 plugin
    // declarations, so an exact comparison rejects the very number this
    // API just reported. nih-plug declares the compressor's attack as
    // `FloatRange::Skewed { min: 0.1, .. }` in f32; widened to f64 that
    // is 0.10000000149011612, which is strictly greater than the f64 0.1
    // a caller types. Every f32-declared bound without an exact binary
    // representation has this, on every plugin (ba doc #273, todo
    // #1235). So compare with an f32-precision tolerance and CLAMP what
    // lands inside the band — the DSP never sees a value below the
    // plugin's real minimum, and a genuinely out-of-range request is
    // still refused with the same message it always got.
    //
    // `resolve_param_value` also turns a choice label into its step, so
    // a caller can send what the parameter calls itself.
    let value = match resolve_param_value(param, &params.value) {
        Ok(value) => value,
        Err(e) => return reject(request, e),
    };

    // The instance id is the engine's handle; it is not on the wire, so
    // recover it from the same chain position the entry came from.
    let Some(instance_id) = instance_for(&t, &entry.plugin_id, entry.occurrence) else {
        return reject(
            request,
            RpcError::not_found(format!(
                "plugin {:?} vanished from track {} between lookup and set",
                entry.plugin_id, t.id
            )),
        );
    };

    let task = run_via_update(
        app,
        Message::Plugin(PluginMessage::SetPluginParam(instance_id, param.id, value)),
    );
    (ack(app, request), task)
}

/// The number a `set_plugin_param` request means for one parameter, or
/// the error explaining why it means none (ba todo #1290).
///
/// Three things happen here, in the order a caller's mistake is easiest
/// to describe:
///
/// 1. a choice LABEL becomes the step it names — `"Low-pass"` is the
///    value the parameter reports as its text, so it must also be a
///    value it accepts, and a wrong label comes back with the ones that
///    would have worked rather than as an out-of-range number;
/// 2. a non-finite number is refused, because NaN compares false against
///    every bound and would sail through the range check;
/// 3. the f32-declared-bounds tolerance ([`clamp_within_tolerance`]) is
///    applied.
///
/// Shared by the track, bus and master setters so one label resolves the
/// same way wherever the plugin sits.
pub(crate) fn resolve_param_value(
    param: &track::PluginParamView,
    requested: &track::ParamValue,
) -> Result<f64, RpcError> {
    let value = requested
        .resolve(param)
        .map_err(|e| RpcError::invalid_params(format!("{}: {e}", param.name)))?;
    if !value.is_finite() {
        return Err(RpcError::invalid_params(format!(
            "value must be finite (got {value})"
        )));
    }
    clamp_within_tolerance(value, param.min, param.max).ok_or_else(|| {
        let choices = if param.choices.is_empty() {
            String::new()
        } else {
            format!(" — its choices are [{}]", param.choices.join(", "))
        };
        RpcError::invalid_params(format!(
            "{} must be within {}..={} (got {value}){choices}",
            param.name, param.min, param.max
        ))
    })
}

/// `value` clamped into `min..=max`, or `None` when it lies genuinely
/// outside the range (ba doc #273, todo #1235).
///
/// "Genuinely" is the whole point: `min`/`max` reach the wire as f64
/// widenings of f32 plugin declarations, so the exact f64 a caller reads
/// out of `track.plugin_params` round-trips, but the tidy decimal it
/// *means* (`0.1`) sits a few ULPs outside. The tolerance scales with the
/// magnitude of the range rather than being a fixed absolute epsilon —
/// on a 20..20000 Hz parameter an absolute 1e-7 would be meaningless,
/// and on a 0..1 parameter it would be far too generous.
///
/// A request beyond the tolerance band is still `None`, so
/// `set_plugin_param` keeps its reject-don't-clamp promise for values
/// the caller really did get wrong.
pub(crate) fn clamp_within_tolerance(value: f64, min: f64, max: f64) -> Option<f64> {
    if !(min <= max) {
        // A plugin declaring an inverted range is a plugin bug; take the
        // value as-is rather than rejecting everything it exposes.
        //
        // Written as `!(min <= max)` rather than `min > max` so a NaN
        // bound lands here too: NaN fails every comparison, so `min > max`
        // would wave it through to `value.clamp(min, max)`, and
        // `f64::clamp` *asserts* `min <= max` — a third-party CLAP plugin
        // reporting a NaN bound would panic the update loop. `ParamInfo`
        // takes `min_value`/`max_value` straight from the plugin with no
        // sanitisation (`resonance-audio/src/clap_host/instance.rs`), so
        // that is reachable from outside the project.
        return Some(value);
    }
    let tol = (max - min)
        .abs()
        .max(max.abs())
        .max(min.abs())
        .max(1.0)
        * f32::EPSILON as f64;
    if value < min - tol || value > max + tol {
        return None;
    }
    Some(value.clamp(min, max))
}
