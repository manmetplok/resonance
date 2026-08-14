//! Domain rules for a track's plugin chain (ba todo #1261).
//!
//! A track's chain is an ordered `Vec` where the index *is* the
//! processing order. On an instrument track one slot is structural: it
//! is what receives MIDI and what every sub-track inherits its latency
//! from. Effects sit after it. That rule is a property of the chain, not
//! of any one caller, so it lives here and every path that reorders a
//! chain applies it — the control API, and any future mixer
//! drag-to-reorder.

use crate::state::TrackState;
use crate::Resonance;
use resonance_audio::types::TrackType;

/// Which chain index holds the track's instrument, if it has one.
///
/// Classification comes from the plugin scanner. When the scanner does
/// not know the plugin in slot 0 at all — a project whose instrument is
/// no longer installed — that slot still counts as the instrument, so
/// the chain keeps its shape across a missing plugin. A plugin the
/// scanner positively classified as an *effect* never does.
pub(crate) fn instrument_slot(app: &Resonance, t: &TrackState) -> Option<usize> {
    if t.track_type != TrackType::Instrument {
        return None;
    }
    let scanned = |id: &str| app.available_plugins.iter().find(|p| p.clap_plugin_id == id);
    if let Some(i) = t
        .plugins
        .iter()
        .position(|p| scanned(&p.clap_plugin_id).is_some_and(|s| s.is_instrument))
    {
        return Some(i);
    }
    match t.plugins.first() {
        Some(p) if scanned(&p.clap_plugin_id).is_none() => Some(0),
        _ => None,
    }
}

/// The lowest chain index an **effect** may occupy once `moving` has
/// been lifted out of the chain.
///
/// A move is `remove(from)` then `insert(to)` (`TrackState::move_plugin`),
/// so when the mover sits BELOW the instrument, removing it shifts the
/// instrument down one and landing at `instrument_slot` already puts the
/// effect after it. Adding one unconditionally is off by one in that
/// direction and refuses a legal move: on `[eq, instrument]` it makes
/// the floor 2 while the last slot is 1, so no destination whatsoever
/// can reorder that chain.
fn effect_slot_floor_for(app: &Resonance, t: &TrackState, moving: u32) -> u32 {
    match instrument_slot(app, t) {
        None => 0,
        Some(instrument) => {
            let instrument = instrument as u32;
            if moving < instrument {
                instrument
            } else {
                instrument + 1
            }
        }
    }
}

/// Where a requested move of an effect to `to_index` actually lands.
///
/// `Ok(slot)` is the destination to use — a request past the end means
/// the end, clamped rather than refused so "move it last" works without
/// the caller counting the chain (the engine clamps identically, so the
/// mirrored order agrees with what it does).
///
/// `Err(floor)` means the move would displace the track's sound source.
/// Callers refuse: the control API turns `floor` into a wire error, the
/// GUI path drops the move.
///
/// `moving` is the chain index of the plugin being moved, and it is
/// required: the rule is not "no effect may land below the floor" but
/// "the instrument stays put and effects stay after it", and those are
/// different guarantees. Without it the guard refuses an effect moved
/// onto slot 0 while happily letting the INSTRUMENT walk down its own
/// chain — `[instrument, eq, comp]` with the instrument sent to slot 2
/// passes a floor of 1, lands as `[eq, comp, instrument]`, and is then
/// stuck.
///
/// The clamp is applied BEFORE the floor check, and that ordering is
/// load-bearing: the control layer validates the raw `to_slot` and then
/// dispatches the clamped one, so checking the raw value let it ack a
/// move that the pre-dispatch gate — which only ever sees the clamped
/// value — then silently refused. Clamping first makes both callers ask
/// the same question and get the same answer.
pub(crate) fn resolve_effect_move(
    app: &Resonance,
    t: &TrackState,
    moving: u32,
    to_index: u32,
) -> Result<u32, u32> {
    let floor = effect_slot_floor_for(app, t, moving);
    // The instrument itself is structural: it does not move at all.
    if let Some(instrument) = instrument_slot(app, t) {
        if moving == instrument as u32 {
            return Err(floor);
        }
    }
    let last = t.plugins.len().saturating_sub(1) as u32;
    let dest = to_index.min(last);
    if dest < floor {
        return Err(floor);
    }
    Ok(dest)
}
