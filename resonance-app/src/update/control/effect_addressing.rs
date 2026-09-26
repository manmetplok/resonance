//! The slot-or-(plugin_id, occurrence) effect-addressing state machine
//! shared by `track/bus/master.remove_effect` / `move_effect` /
//! `replace_effect`.
//!
//! It used to be three copies — `resolve_chain_effect` in
//! `track/chain.rs`, `resolve_bus_effect` in `bus.rs`,
//! `resolve_master_effect` in `master.rs` — each with its own
//! `chain_description`. [`ParamValue::resolve`]'s doc comment
//! (resonance-control/src/methods/track.rs:713-719) makes exactly this
//! argument about the neighbouring problem, choice-label resolution:
//! "three copies of a lookup is how they [stop agreeing]". These three
//! already had: track's "plugin vanished" wording didn't match bus's,
//! and only track's not-found-by-id listing carried a stray colon
//! (`"carries: ["` vs `"carries ["`) that nobody decided on — it just
//! drifted, silently, because nothing made the three agree.
//!
//! [`ParamValue::resolve`]: resonance_control::methods::track::ParamValue::resolve
//!
//! What's genuinely shared: given a chain's wire entries plus the raw
//! `(slot, plugin_id, occurrence)` triple, there are exactly four
//! outcomes (both given / neither given / named and missing / named and
//! found), plus an instrument check and a final "did it vanish"
//! check on the way out — see [`resolve_effect`]. What's genuinely NOT
//! shared is what the error SAYS: a track names itself `"track 5"`, a
//! bus `"bus 3"`, the master `"the master chain"` with no id at all.
//! That wording is deliberately left unglued from the state machine via
//! [`ChainWording`], and this pass does not unify the strings — track's
//! not-found-by-id message still lists occurrences instead of slots,
//! and still keeps its colon. Doing that too would touch every test
//! that pins one surface's current wording, for a change nobody asked
//! for; it's a candidate for a later, deliberate pass.

use crate::state::PluginSlotState;
use resonance_audio::types::PluginInstanceId;
use resonance_control::methods::track::{self, PluginKind};
use resonance_control::RpcError;

/// One surface's wording for the four ways [`resolve_effect`] can fail.
/// The resolution order and the conditions themselves are identical
/// across track/bus/master (that's what [`resolve_effect`] owns); this
/// is only the text, which the three said differently before this
/// module existed and keep saying differently now.
pub(super) trait ChainWording {
    /// Neither `slot` nor `plugin_id` was given.
    fn no_address(&self, verb: &str, listing: &str) -> String;

    /// `slot` was given and no entry sits there.
    fn slot_not_found(&self, slot: u32, listing: &str) -> String;

    /// `plugin_id` (+ `occurrence`) was given and no entry matches.
    /// Takes the shared `listing` so bus/master can use it directly;
    /// track's implementation ignores it and defers to
    /// [`view_model::unknown_plugin_on_track`](super::view_model::unknown_plugin_on_track),
    /// which lists occurrences instead of slots and predates this
    /// module — a drift this pass preserves rather than papers over.
    fn id_not_found(&self, plugin_id: &str, occurrence: u32, listing: &str) -> RpcError;

    /// The resolved entry is the surface's instrument slot and `verb`
    /// refuses to touch it — `None` when the surface has no instrument
    /// concept at all (bus, master; their entries are always
    /// `PluginKind::Effect`) or `verb` is exempt (track's `replace`).
    fn instrument_refusal(&self, entry: &track::PluginParamsEntry, verb: &str) -> Option<String>;

    /// The entry resolved a moment ago is no longer in the chain by the
    /// time its instance id is looked up. Structurally unreachable
    /// through today's callers — `entries` and `instance_of` both read
    /// the same snapshot within one request, with no await between
    /// them — but kept as the defensive backstop all three original
    /// call sites always had.
    fn vanished(&self, plugin_id: &str, verb: &str) -> String;
}

/// Resolve one [`track::PluginParamsEntry`] plus its engine instance id
/// from the slot-or-(plugin_id, occurrence) pair every chain-editing
/// method accepts.
///
/// `entries` is the surface's already-built wire listing (track's tags
/// one slot `Instrument`; bus's and master's never do). `instance_of`
/// turns a resolved `(plugin_id, occurrence)` back into an engine
/// instance id — a track, a bus and the master each hold their chain in
/// a different collection ([`crate::state::TrackState::plugins`],
/// [`crate::state::BusState::plugins`], `Resonance::master.plugins`),
/// so this is the one piece of surface-specific *behaviour* (not just
/// wording) left as a closure rather than data. [`instance_at`] is the
/// shared body for it; track keeps its own copy
/// (`track::instance_for`) because it isn't this module's file to
/// touch.
pub(super) fn resolve_effect(
    entries: &[track::PluginParamsEntry],
    slot: Option<u32>,
    plugin_id: Option<&str>,
    occurrence: Option<u32>,
    verb: &str,
    wording: &dyn ChainWording,
    instance_of: impl Fn(&str, u32) -> Option<PluginInstanceId>,
) -> Result<(track::PluginParamsEntry, PluginInstanceId), RpcError> {
    let entry = match (slot, plugin_id) {
        (Some(_), Some(_)) => {
            return Err(RpcError::invalid_params(
                "address the effect by slot OR by plugin_id (+ occurrence), not both",
            ))
        }
        (None, None) => {
            return Err(RpcError::invalid_params(
                wording.no_address(verb, &chain_description(entries)),
            ))
        }
        (Some(slot), None) => entries
            .iter()
            .find(|e| e.slot == slot)
            .cloned()
            .ok_or_else(|| {
                RpcError::not_found(wording.slot_not_found(slot, &chain_description(entries)))
            })?,
        (None, Some(id)) => {
            let occurrence = occurrence.unwrap_or(0);
            entries
                .iter()
                .find(|e| e.plugin_id == id && e.occurrence == occurrence)
                .cloned()
                .ok_or_else(|| {
                    wording.id_not_found(id, occurrence, &chain_description(entries))
                })?
        }
    };

    if entry.kind == PluginKind::Instrument {
        if let Some(refusal) = wording.instrument_refusal(&entry, verb) {
            return Err(RpcError::invalid_params(refusal));
        }
    }

    let instance_id = instance_of(&entry.plugin_id, entry.occurrence)
        .ok_or_else(|| RpcError::not_found(wording.vanished(&entry.plugin_id, verb)))?;
    Ok((entry, instance_id))
}

/// The chain as `slot:plugin_id` pairs, for error messages that let the
/// caller correct an address rather than guess again.
///
/// Shared verbatim by bus and master (and by track everywhere except
/// its not-found-by-id message, see [`ChainWording::id_not_found`]).
pub(super) fn chain_description(entries: &[track::PluginParamsEntry]) -> String {
    entries
        .iter()
        .map(|e| format!("{}:{}", e.slot, e.plugin_id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The engine instance id of the `occurrence`-th plugin with this CLAP
/// id in `chain`.
///
/// Bus and master share this rather than each hand-rolling the same
/// `filter().nth()`; track keeps its own copy (`track::instance_for`,
/// over `TrackState::plugins`) since `track/mod.rs` isn't this module's
/// file to touch, but it is the identical lookup over the identical
/// element type.
pub(super) fn instance_at(
    chain: &[PluginSlotState],
    plugin_id: &str,
    occurrence: u32,
) -> Option<PluginInstanceId> {
    chain
        .iter()
        .filter(|p| p.clap_plugin_id == plugin_id)
        .nth(occurrence as usize)
        .map(|p| p.instance_id)
}
