//! `track.set_plugin_param` — one parameter on one plugin, and the
//! f32-bound tolerance every plugin-param setter in the control API
//! shares.

use super::{ack, find_track, frozen_reject, not_found_track, reject};
use crate::message::{Message, PluginMessage};
use crate::update::control::plugin_target::{resolve_plugin_param, ChainOwner};
use crate::update::control::run_via_update;
use crate::Resonance;
use iced::Task;
use resonance_control::methods::track;
use resonance_control::{Request, Response, RpcError};

/// Resolve a `param` string against a plugin's parameter list: by display
/// name (case-insensitive) first — what a client reading `plugin_params`
/// usually has to hand — then by numeric CLAP id, then by a first-party
/// plugin's string key (`"lim_on"`), whose CLAP id is
/// [`resonance_plugin::stable_hash`] of the key (FU-M5c). Keys survive a
/// display rename, so skills can pin them. Shared by the track, bus and
/// master setters.
pub(crate) fn find_param<'a>(
    params: &'a [track::PluginParamView],
    wanted: &str,
) -> Option<&'a track::PluginParamView> {
    params
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            let id = wanted.parse::<u32>().ok()?;
            params.iter().find(|p| p.id == id)
        })
        .or_else(|| {
            let id = resonance_plugin::stable_hash(wanted);
            params.iter().find(|p| p.id == id)
        })
}

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

    // Resolve the plugin — an explicit id, else the track's instrument,
    // the common case for "make this synth sound different" — and the
    // parameter on it. The addressing (and every miss's wording) is the
    // one `bus.*`, `master.*` and `automation.*` share; it also answers
    // `busy` for the window `track.add_effect`'s synchronous commit
    // (todo #1234) cannot close: the slot exists, but its parameter list
    // only arrives with the engine's `PluginAdded` echo.
    let (target, param) = match resolve_plugin_param(
        app,
        ChainOwner::Track(t.id),
        params.plugin_id.as_deref(),
        params.occurrence,
        &params.param,
    ) {
        Ok(found) => found,
        Err(e) => return reject(request, e),
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
    let value = match resolve_param_value_on(app, target.instance_id, &param, &params.value) {
        Ok(value) => value,
        Err(e) => return reject(request, e),
    };
    let instance_id = target.instance_id;

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

/// How long a setter waits for the plugin to answer a label
/// (`AudioEngine::param_from_text`): an engine-thread round trip, normally
/// well under a millisecond.
const PARAM_TEXT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// [`resolve_param_value`], and when that fails on a label for a parameter
/// with no enumerated `choices`, ask the plugin itself what the label
/// means (CLAP `text_to_value`, nam-model-library.md §9.2). This is how
/// `"Friedman BE-100"` picks a model on the amp's 1000-slot selector, and
/// `"-6 dB"` a gain, on any plugin that implements `text_to_value`.
///
/// The answer goes through the same finite and range checks a number
/// does. Shared by the track, bus and master setters.
pub(crate) fn resolve_param_value_on(
    app: &Resonance,
    instance_id: resonance_audio::PluginInstanceId,
    param: &track::PluginParamView,
    requested: &track::ParamValue,
) -> Result<f64, RpcError> {
    let first = resolve_param_value(param, requested);
    let track::ParamValue::Label(text) = requested else {
        return first;
    };
    let text = text.trim();
    if first.is_ok() || !param.choices.is_empty() || text.parse::<f64>().is_ok() {
        return first;
    }
    // Keep the choice-resolution error as the base: it already tells the
    // caller to send a number in range; say what the plugin answered too.
    let base = first.err().map(|e| e.message).unwrap_or_default();
    match app
        .engine
        .param_from_text(instance_id, param.id, text, PARAM_TEXT_TIMEOUT)
    {
        Ok(Some(value)) => resolve_param_value(param, &track::ParamValue::Number(value)),
        Ok(None) => Err(RpcError::invalid_params(format!(
            "{base}; nor does the plugin recognise {text:?} as one of its displayed values \
             (read them back as `text` from plugin_params)"
        ))),
        Err(()) => Err(RpcError::invalid_params(format!(
            "{base} (the plugin was asked what {text:?} means but did not answer in time)"
        ))),
    }
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
