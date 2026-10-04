//! `track.set_plugin_param` — one parameter on one plugin, and the
//! f32-bound tolerance every plugin-param setter in the control API
//! shares.

use super::{ack, find_track, frozen_reject, not_found_track, reject};
use crate::message::{Message, PluginMessage};
use crate::update::control::plugin_target::{resolve_plugin_param, ChainOwner, PluginTarget};
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
    let value = match resolve_param_value_on(app, request, &target, &param, &params.value) {
        Ok(ParamValueOutcome::Value(value)) => value,
        Ok(ParamValueOutcome::Deferred) => return deferred_reply(request),
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

/// How long a deferred label waits for the plugin's answer before the
/// request is refused. Nothing blocks meanwhile — the update loop and the
/// engine both carry on — so this is only a bound on a lost answer.
pub(crate) const LABEL_ANSWER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(3);

// Resonance Amp's CLAP id and its model selector's string key.
use resonance_plugin::first_party::{amp::FILE_SELECT as AMP_MODEL_SELECT, AMP as AMP_ID};

/// What a `set_plugin_param` value came to.
pub(crate) enum ParamValueOutcome {
    /// The number to set.
    Value(f64),
    /// Only the plugin can say what the label means: the question went to
    /// the engine and the reply is deferred ([`label_resolved`]).
    Deferred,
}

/// [`resolve_param_value`], and when that fails on a label for a parameter
/// with no enumerated `choices`, one more way to read the label
/// (nam-model-library.md §9.2):
///
/// - on Resonance Amp's `Model Select`, the app resolves the model name (or
///   id prefix) itself, from the same library the plugin reads — no round
///   trip;
/// - on any other plugin the question goes to the plugin (CLAP
///   `text_to_value`) through the engine, **without blocking the update
///   loop**: the request's reply is deferred until the engine's
///   `PluginParamTextResolved` answer, which only carries a value when the
///   plugin's own display of it round-trips to the label (so a lenient
///   plugin that parses `"loud"` as 0 is not believed).
///
/// A deferred request is re-run with the number when the answer comes, so
/// it still costs one call, one undo entry and one revision.
pub(crate) fn resolve_param_value_on(
    app: &mut Resonance,
    request: &Request,
    target: &PluginTarget,
    param: &track::PluginParamView,
    requested: &track::ParamValue,
) -> Result<ParamValueOutcome, RpcError> {
    // An output only the plugin writes: the plugin would drop the value,
    // so acking it (and recording an undo step and mirroring a number
    // that never lands) would be a lie.
    if param.read_only {
        return Err(RpcError::invalid_params(format!(
            "{} is read-only: the plugin reports it (read it from plugin_params) and \
             ignores writes",
            param.name
        )));
    }
    let first = resolve_param_value(param, requested);
    let track::ParamValue::Label(text) = requested else {
        return first.map(ParamValueOutcome::Value);
    };
    let text = text.trim();
    if first.is_ok() || !param.choices.is_empty() || text.parse::<f64>().is_ok() {
        return first.map(ParamValueOutcome::Value);
    }
    let base = first.err().map(|e| e.message).unwrap_or_default();

    if target.entry.plugin_id == AMP_ID && param.id == resonance_plugin::stable_hash(AMP_MODEL_SELECT) {
        return match super::super::amp_models::slot_for_label(app, text) {
            Ok(slot) => resolve_param_value(param, &track::ParamValue::Number(f64::from(slot)))
                .map(ParamValueOutcome::Value),
            Err(why) => Err(RpcError::invalid_params(format!("{}: {why}", param.name))),
        };
    }

    let Some(reply) = app.control.current_reply.clone() else {
        // No reply channel to answer later on (an in-process caller): the
        // choice error is the answer.
        return Err(RpcError::invalid_params(base));
    };
    let token = app.control.next_label_token;
    app.control.next_label_token = token.wrapping_add(1);
    let sent = app
        .engine
        .send(resonance_audio::types::AudioCommand::ResolvePluginParamText {
            instance_id: target.instance_id,
            param_id: param.id,
            text: text.to_string(),
            token,
        });
    if sent.is_err() {
        return Err(RpcError::invalid_params(base));
    }
    app.control.pending_labels.insert(
        token,
        crate::state::PendingLabel {
            conn: app.control.current_conn.unwrap_or_default(),
            request: request.clone(),
            reply,
            deadline: std::time::Instant::now() + LABEL_ANSWER_DEADLINE,
            refusal: format!(
                "{base}; nor does the plugin recognise {text:?} as one of its displayed values \
                 (read them back as `text` from plugin_params)"
            ),
        },
    );
    app.control.deferred = true;
    Ok(ParamValueOutcome::Deferred)
}

/// The placeholder a handler returns once it deferred its reply: never
/// sent (the request's reply goes out from [`label_resolved`]).
pub(crate) fn deferred_reply(request: &Request) -> (Response, Task<Message>) {
    (
        Response::failure(
            Some(request.id.clone()),
            RpcError::internal("deferred: the plugin is resolving the label"),
        ),
        Task::none(),
    )
}

/// The engine answered a deferred label: re-run the request with the value
/// (through the full dispatch — gates, compound undo, one revision) and
/// reply, or refuse it.
pub(crate) fn label_resolved(app: &mut Resonance, token: u64, value: Option<f64>) -> Task<Message> {
    let Some(pending) = app.control.pending_labels.remove(&token) else {
        return Task::none();
    };
    let Some(value) = value else {
        pending
            .reply
            .send(Response::failure(Some(pending.request.id.clone()), RpcError::invalid_params(pending.refusal)));
        return Task::none();
    };
    let mut request = pending.request.clone();
    if let Some(obj) = request.params.as_mut().and_then(|p| p.as_object_mut()) {
        obj.insert("value".into(), serde_json::json!(value));
    }
    let (response, task) = super::super::execute(app, pending.conn, &request);
    pending.reply.send(response);
    task
}

/// Refuse every deferred label whose answer is overdue (called from the
/// tick; `now` is injectable for tests).
pub(crate) fn expire_pending_labels(app: &mut Resonance, now: std::time::Instant) {
    let overdue: Vec<u64> = app
        .control
        .pending_labels
        .iter()
        .filter(|(_, p)| p.deadline <= now)
        .map(|(t, _)| *t)
        .collect();
    for token in overdue {
        if let Some(p) = app.control.pending_labels.remove(&token) {
            p.reply.send(Response::failure(
                Some(p.request.id.clone()),
                RpcError::invalid_params(format!(
                    "{} (the plugin was asked but did not answer in time)",
                    p.refusal
                        .split("; nor does")
                        .next()
                        .unwrap_or(&p.refusal)
                )),
            ));
        }
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
