//! Plugin-delay compensation (PDC).
//!
//! Model: each plugin's latency is read from its `clap.latency`
//! extension right after activation. The host vtable does implement
//! `clap_host_latency` (`clap_host/mod.rs`): a plugin calling
//! `changed()` latches a flag on its `HostData`, and the engine thread's
//! next poll (`take_host_restart_requests`) deactivates,
//! re-activates and re-reads the instance at a safe point, then
//! recomputes the chains below (doc #260 finding #10). Compensation
//! runs in two stages so aux sends stay
//! aligned with main outputs (a signal can leave one track through
//! *several* paths — main output plus any number of sends — and a
//! single per-track delay cannot equalize divergent downstream chains):
//!
//! 1. **Track stage.** A track's *chain latency* is the sum of its own
//!    effective plugin latencies plus — for sub-tracks — the parent's
//!    instrument (first plugin). Every track is delayed by
//!    `max_track_chain − its_chain` right after its plugin chain and
//!    before fader/routing/send taps, so all tracks present their
//!    signal to the routing stage (bus inputs, send taps, master)
//!    at the same moment.
//! 2. **Bus stage.** Every bus is delayed post-chain by
//!    `max_bus_chain − its_chain`, so any signal that traverses *any*
//!    bus — via its main output or an aux send — picks up exactly
//!    `max_bus_chain` samples on the way to master. Signals routed
//!    straight to master pick up the same `max_bus_chain` through one
//!    shared "dry" delay applied to the master-direct sum, keeping dry
//!    paths aligned with wet returns.
//!
//! The total pipeline latency is `max_track_chain + max_bus_chain`.
//! Known limitation: a bus→bus aux send crosses the bus stage twice, so
//! its wet contribution arrives `max_bus_chain` late; equalizing that
//! would need per-depth staging (not worth it for the current UI).
//!
//! Compensated: track chains, sub-track chains (incl. the shared
//! parent instrument), bus chains — including send/return busses — via
//! the bus-stage delays + shared dry delay, and the manual round-trip
//! offset of external-instrument tracks (their hardware audio return
//! arrives late, so the rest of the mix is delayed to meet it — see
//! [`add_external_offsets`]).
//! Not compensated:
//! - the master FX chain — it delays every path equally, shifting the
//!   whole output without misaligning tracks;
//! - the metronome click and live-input monitoring, which are rendered
//!   at the raw playhead and therefore lead plugin-delayed material by
//!   the maximum chain latency;
//! - latency changes a plugin makes after activation (state load,
//!   lookahead parameter edits) — re-activation would be needed.
//!
//! Live playback uses a [`LatencyComp`] published through an `ArcSwap`
//! by the engine thread whenever the track/bus/plugin topology changes;
//! the audio callback only ever loads it and runs pre-allocated delay
//! lines. A republished table is built with [`LatencyComp::following`]:
//! it keeps the lines (and their history) of the table it replaces, and
//! crossfades any delay that changed instead of restarting it from
//! silence (code review RT-04). The offline bounce renderer builds its own instance per run
//! and additionally trims the leading `max_latency` frames so bounced
//! audio lands exactly on the timeline.

use std::collections::HashMap;
use std::sync::Arc;

use indexmap::IndexMap;
use parking_lot::Mutex;
use resonance_dsp::DelayLine;

use crate::limits::MAX_COMP_LATENCY;
use crate::types::{Bus, BusId, PluginInstanceId, TrackId, TrackMap, TrackType};

/// The latency one chain slot actually contributes, given the plugin's
/// reported latency and whether the mixer skips it (ba doc #275 finding
/// X3).
///
/// Per-slot bypass has two shapes, and only one of them moves the comp
/// table:
///
/// - **Host bypass** — the plugin declares no bypass parameter, so the
///   mixer stops calling it. Nothing runs, nothing delays: the slot
///   contributes 0 and the rest of the mix re-aligns around it, exactly
///   as it already does for a whole bypassed chain.
/// - **The plugin's own bypass parameter** — the plugin keeps running
///   and keeps reporting the same latency, so its contribution is
///   unchanged and the comp table does not move at all. This is why the
///   host prefers a plugin's own bypass when it declares one: it is the
///   only way to bypass a latency-carrying plugin without re-aligning
///   every other track around it (a time shift, code review RT-04).
///
/// `host_bypassed` is `PluginSlot::host_bypassed()`.
#[inline]
pub fn slot_latency(reported: u64, host_bypassed: bool) -> u64 {
    if host_bypassed {
        0
    } else {
        reported
    }
}

/// Track-stage chain latency per track (see the module doc). Returns
/// one entry per track, sub-tracks included. `plugin_latency` resolves
/// one plugin instance's latency in samples. Bus chains are *not*
/// included — the bus stage is equalized separately (see
/// [`bus_chain_latencies`]), so a track's entry covers exactly the
/// processing between its sources and the routing stage.
///
/// Only *effective* processing counts — plugins the mixer will actually
/// run this pass (mirroring `mixer::render_core`):
/// - a frozen track plays its latency-trimmed cache instead of the
///   instrument + FX chain, so its own chain contributes 0;
/// - FX bypass skips a track's effect chain (on instrument tracks the
///   instrument itself still runs and still counts; sub-track chains
///   are all FX);
/// - an individually bypassed *slot* drops out of its chain's sum — the
///   caller's `plugin_latency` closure resolves that through
///   [`slot_latency`], so this function needs no per-slot knowledge;
/// - a frozen parent skips its instrument fan-out, so its sub-tracks
///   don't inherit the parent-instrument latency.
pub fn chain_latencies(
    tracks: &TrackMap,
    plugin_latency: impl Fn(PluginInstanceId) -> u64,
) -> Vec<(TrackId, u64)> {
    tracks
        .values()
        .map(|track| {
            let is_sub = track.sub_track_of.is_some();
            let own: u64 = if !is_sub && track.frozen_source.load().is_some() {
                // Frozen: audio comes from the pre-trimmed cache; the
                // idle instrument + FX chain adds no latency. (Sub-tracks
                // are always rendered live off the parent's fan-out, so
                // a frozen source on one is ignored — as in the mixer.)
                0
            } else {
                let plugins = track.plugins();
                if !is_sub && track.track_type == TrackType::Instrument {
                    // The instrument (first plugin) runs even under FX
                    // bypass; only the downstream effects are skipped.
                    let instrument =
                        plugins.first().map(|&p| plugin_latency(p)).unwrap_or(0);
                    let fx: u64 = if track.fx_bypassed() {
                        0
                    } else {
                        plugins.iter().skip(1).map(|&p| plugin_latency(p)).sum()
                    };
                    instrument + fx
                } else if track.fx_bypassed() {
                    0
                } else {
                    plugins.iter().map(|&p| plugin_latency(p)).sum()
                }
            };
            // Sub-tracks are fed by the parent's instrument (first
            // plugin), so they inherit its latency on top of their own
            // FX chain — unless the parent is frozen, in which case the
            // fan-out never runs.
            let parent_instrument = track
                .sub_track_of
                .and_then(|(parent_id, _)| tracks.get(&parent_id))
                .filter(|parent| parent.frozen_source.load().is_none())
                .and_then(|parent| parent.plugins().first().copied())
                .map(&plugin_latency)
                .unwrap_or(0);
            (track.id, own + parent_instrument)
        })
        .collect()
}

/// Bus-stage chain latency per bus: the sum of the bus's effective
/// plugin latencies (0 while the bus's FX are bypassed — the mixer
/// skips the whole chain). One entry per bus; [`compensation_delays`]
/// over this list yields the bus-stage delays and `max_bus_chain`.
///
/// Generic over the element so the render graph's `Arc<Bus>` map and a
/// plain `Bus` map both work.
pub fn bus_chain_latencies<B: std::borrow::Borrow<Bus>>(
    busses: &IndexMap<BusId, B>,
    plugin_latency: impl Fn(PluginInstanceId) -> u64,
) -> Vec<(BusId, u64)> {
    busses
        .iter()
        .map(|(&id, bus)| {
            let bus: &Bus = bus.borrow();
            let lat = if bus.fx_bypassed() {
                0
            } else {
                bus.plugin_ids.iter().map(|&p| plugin_latency(p)).sum()
            };
            (id, lat)
        })
        .collect()
}

/// True when any chain exceeds [`MAX_COMP_LATENCY`] — i.e. the clamp in
/// [`compensation_delays`] is actually engaging and alignment for that
/// chain is silently degraded. The engine surfaces a warning when this
/// flips on (doc #260 finding #20). Pure; unit-tested.
pub fn comp_latency_clamped(chains: &[(u64, u64)]) -> bool {
    chains.iter().any(|&(_, l)| l > MAX_COMP_LATENCY)
}

/// Effective master-chain latency: the sum of the master plugins'
/// reported latencies, 0 while the master FX are bypassed (the render
/// paths skip the whole chain). Live playback deliberately leaves this
/// uncompensated — it delays every path equally — but *offline export*
/// pre-rolls/trims by it on top of the track/bus comp so the file
/// starts at t=0 and keeps its full tail (doc #260 finding #8).
pub fn master_chain_latency(
    plugin_ids: &[PluginInstanceId],
    bypassed: bool,
    plugin_latency: impl Fn(PluginInstanceId) -> u64,
) -> u64 {
    if bypassed {
        0
    } else {
        plugin_ids.iter().map(|&p| plugin_latency(p)).sum()
    }
}

/// Fold manual external-instrument latency offsets into per-track chain
/// latencies, in place. An external instrument's audio return arrives a
/// round-trip late (MIDI out → hardware synth → audio in); its positive
/// `latency_offset_samples` models that delay, so the track is treated as
/// that much more latent and [`compensation_delays`] delays the rest of
/// the mix to meet it. This keeps the live return — and the realtime
/// bounce that records it through the same mix loop — aligned with the
/// timeline. A non-positive offset can't advance a live stream and is
/// ignored; tracks with no offset (`offset_for` returns 0) are untouched.
pub fn add_external_offsets(chains: &mut [(TrackId, u64)], offset_for: impl Fn(TrackId) -> i64) {
    for (id, latency) in chains.iter_mut() {
        let offset = offset_for(*id);
        if offset > 0 {
            *latency = latency.saturating_add(offset as u64);
        }
    }
}

/// Pure compensation math: given per-track chain latencies, return the
/// maximum (the whole mix's pipeline latency) and the delay each track
/// must add so that `delay + chain == max` for every track. Chain
/// latencies are clamped to [`MAX_COMP_LATENCY`].
pub fn compensation_delays(chains: &[(TrackId, u64)]) -> (u64, Vec<(TrackId, u64)>) {
    let max = chains
        .iter()
        .map(|&(_, l)| l.min(MAX_COMP_LATENCY))
        .max()
        .unwrap_or(0);
    let delays = chains
        .iter()
        .map(|&(id, l)| (id, max - l.min(MAX_COMP_LATENCY)))
        .collect();
    (max, delays)
}

struct DelayState {
    line_l: DelayLine,
    line_r: DelayLine,
    /// Timeline frame the next `apply` call is expected to start at.
    /// A mismatch (seek, a track that was skipped while muted)
    /// invalidates the lines so stale audio doesn't replay. A loop wrap
    /// is not one: the seam rebases this via
    /// [`LatencyComp::continue_across_loop_wrap`].
    next_playhead: Option<u64>,
    /// Samples pushed since the last discontinuity (saturating). A tap of
    /// `d` reads real audio only while `d < filled`; anything older is a
    /// stale tail from before the discontinuity and reads as 0.0 —
    /// exactly what a freshly cleared line would return.
    ///
    /// Invalidation is lazy: eagerly `clear()`ing both lines would memset
    /// up to [`MAX_COMP_LATENCY`] samples each, per compensated
    /// track/bus, inside the first callback after every seek. Resetting
    /// this counter instead costs nothing, while fresh pushes displace
    /// the stale tail in place, so it is never read again. Continuous
    /// playback only ever grows it.
    filled: usize,
    /// The delay this line's output is read at (the target of an
    /// in-flight delay change). A republished table that asks for a
    /// different delay starts a crossfade from this one (code review
    /// RT-04).
    read_delay: usize,
    /// The delay an in-flight change fades *from*.
    xfade_from: usize,
    /// Frames of the delay-change crossfade done so far; it is in flight
    /// while `xfade_pos < xfade_len`.
    xfade_pos: usize,
    /// Length of the delay-change crossfade in flight (0 = none).
    xfade_len: usize,
    /// Whether this line has looked at its entry's predecessor yet (see
    /// [`TrackComp::predecessor`]). Set on the first `apply`.
    adopted: bool,
}

/// One delay line pair plus its immutable capacity, shared between
/// successive comp tables when an entry's delay still fits (code review
/// RT-04): the line — and its history — survive a republish untouched.
struct DelayCell {
    /// The largest delay the lines hold (`capacity − 1`). Fixed for the
    /// cell's life, so the engine thread can read it without the lock.
    max_tap: usize,
    state: Mutex<DelayState>,
}

impl DelayCell {
    /// Allocates — engine thread only.
    fn new(delay: usize) -> Arc<Self> {
        // +1 so `tap(delay)` stays within capacity even when `delay` is
        // itself a power of two. `DelayLine` rounds up to a power of two,
        // which is the headroom a later, longer delay can reuse.
        let line_l = DelayLine::new(delay + 1);
        let line_r = DelayLine::new(delay + 1);
        Arc::new(Self {
            max_tap: (delay + 1).max(2).next_power_of_two() - 1,
            state: Mutex::new(DelayState {
                line_l,
                line_r,
                next_playhead: None,
                filled: 0,
                read_delay: delay,
                xfade_from: delay,
                xfade_pos: 0,
                xfade_len: 0,
                adopted: false,
            }),
        })
    }
}

/// The most history a grown line copies from the one it replaces, per
/// channel, on the audio thread (code review RT-04). 2^16 frames is 1.4 s
/// at 48 kHz — beyond any real plugin's latency — and a few hundred
/// microseconds of copying, once. A longer line warms up from silence
/// instead, as every line did before.
const ADOPT_COPY_LIMIT: usize = 1 << 16;

struct TrackComp {
    delay: usize,
    cell: Arc<DelayCell>,
    /// When a republished table needs a longer line than this entry had,
    /// the old line: on its first `apply` the new line copies the old
    /// one's history (bounded by [`ADOPT_COPY_LIMIT`]), so the delay
    /// change is a time shift instead of `delay` samples of silence. Held
    /// for this table's life so the audio thread never drops the last
    /// reference to it.
    predecessor: Option<Arc<DelayCell>>,
}

impl TrackComp {
    /// A new entry with an empty line — the offline / first-publish case.
    fn fresh(delay: usize) -> Self {
        Self {
            delay,
            cell: DelayCell::new(delay),
            predecessor: None,
        }
    }

    /// The entry a republished table uses for an id `old` already had:
    /// the same line when `delay` fits it (history and all), else a
    /// bigger line that adopts the old one's history on first use.
    fn follow(old: &TrackComp, delay: usize) -> Self {
        if delay <= old.cell.max_tap {
            Self {
                delay,
                cell: Arc::clone(&old.cell),
                // A line that never ran yet may still owe an adoption.
                predecessor: old.predecessor.clone(),
            }
        } else {
            Self {
                delay,
                cell: DelayCell::new(delay),
                predecessor: Some(Arc::clone(&old.cell)),
            }
        }
    }
}

/// Build the delay-line table for one compensation stage (tracks or
/// busses): one entry per non-zero delay.
fn build_comp_map(delays: &[(u64, u64)]) -> HashMap<u64, TrackComp> {
    delays
        .iter()
        .filter(|&&(_, d)| d > 0)
        .map(|&(id, d)| (id, TrackComp::fresh(d.min(MAX_COMP_LATENCY) as usize)))
        .collect()
}

/// [`build_comp_map`] for a table that replaces `prev` during playback
/// (code review RT-04). Every id `prev` had a line for keeps one — even
/// at delay 0, where the line is a pass-through that keeps recording
/// history — so a later delay increase replays real audio rather than
/// warming up from silence. An id gone from `delays` (track / bus
/// removed) drops its line.
fn follow_comp_map(prev: &HashMap<u64, TrackComp>, delays: &[(u64, u64)]) -> HashMap<u64, TrackComp> {
    delays
        .iter()
        .filter_map(|&(id, d)| {
            let delay = d.min(MAX_COMP_LATENCY) as usize;
            match prev.get(&id) {
                Some(old) => Some((id, TrackComp::follow(old, delay))),
                None if delay > 0 => Some((id, TrackComp::fresh(delay))),
                None => None,
            }
        })
        .collect()
}

fn stage_matches(map: &HashMap<u64, TrackComp>, delays: &[(u64, u64)]) -> bool {
    let nonzero = delays.iter().filter(|&&(_, d)| d > 0);
    nonzero.clone().count() == map.values().filter(|t| t.delay > 0).count()
        && nonzero
            .clone()
            .all(|&(id, d)| map.get(&id).map(|t| t.delay as u64) == Some(d))
}

/// Copy `pred`'s history into the fresh line `st` (see
/// [`TrackComp::predecessor`]). Only when `pred` ran right up to
/// `playhead` — otherwise its audio is stale anyway and the line warms up
/// from silence. Allocation-free; one `try_lock`.
fn adopt(st: &mut DelayState, max_tap: usize, pred: &DelayCell, playhead: u64) {
    let Some(p) = pred.state.try_lock() else {
        return;
    };
    if p.next_playhead != Some(playhead) {
        return;
    }
    let n = p.filled.min(pred.max_tap + 1).min(max_tap + 1);
    if n > ADOPT_COPY_LIMIT {
        return;
    }
    // Oldest first, so the newest sample lands at tap 0.
    for k in (0..n).rev() {
        st.line_l.push(p.line_l.tap(k));
        st.line_r.push(p.line_r.tap(k));
    }
    st.filled = n;
    st.next_playhead = p.next_playhead;
    st.read_delay = p.read_delay;
    st.xfade_from = p.xfade_from;
    st.xfade_pos = p.xfade_pos;
    st.xfade_len = p.xfade_len;
}

/// Bring `st` up to this block: adopt a predecessor's history once,
/// invalidate on a discontinuity, and start a crossfade when the table's
/// delay differs from the one the line was read at. Returns whether the
/// block's output can carry audio the input doesn't (a delayed tail or
/// a delay change in flight).
fn begin_block(tc: &TrackComp, st: &mut DelayState, playhead: u64, transition: usize) -> bool {
    if !st.adopted {
        st.adopted = true;
        if let Some(pred) = &tc.predecessor {
            adopt(st, tc.cell.max_tap, pred, playhead);
        }
    }
    if st.next_playhead != Some(playhead) {
        // Lazy invalidation — see `DelayState::filled`. No memset here.
        st.filled = 0;
        st.read_delay = tc.delay;
        st.xfade_pos = 0;
        st.xfade_len = 0;
    } else if st.read_delay != tc.delay {
        // A republished table moved this line's delay. Fade the read
        // position from the old delay to the new one: both taps come out
        // of the same history, so the change is a smooth time shift.
        // Re-targeted mid-fade, it restarts from whichever delay
        // dominates the output right now.
        let in_flight = st.xfade_pos < st.xfade_len;
        let from = if in_flight && st.xfade_pos * 2 < st.xfade_len {
            st.xfade_from
        } else {
            st.read_delay
        };
        st.read_delay = tc.delay;
        st.xfade_from = from;
        st.xfade_pos = 0;
        st.xfade_len = if from != tc.delay { transition } else { 0 };
    }
    st.read_delay > 0 || st.xfade_pos < st.xfade_len
}

#[inline(always)]
fn tap_or_zero(line: &DelayLine, delay: usize, filled: usize, max_tap: usize) -> f32 {
    if delay < filled && delay <= max_tap {
        line.tap(delay)
    } else {
        0.0
    }
}

/// Push one stereo frame and read the delayed one back.
#[inline(always)]
fn step(st: &mut DelayState, max_tap: usize, l: f32, r: f32) -> (f32, f32) {
    st.line_l.push(l);
    st.line_r.push(r);
    st.filled = st.filled.saturating_add(1);
    if st.xfade_pos < st.xfade_len {
        st.xfade_pos += 1;
        let w = crate::bypass::fade_weight(st.xfade_pos as f32 / st.xfade_len as f32);
        let (from, to, filled) = (st.xfade_from, st.read_delay, st.filled);
        let a_l = tap_or_zero(&st.line_l, from, filled, max_tap);
        let a_r = tap_or_zero(&st.line_r, from, filled, max_tap);
        let b_l = tap_or_zero(&st.line_l, to, filled, max_tap);
        let b_r = tap_or_zero(&st.line_r, to, filled, max_tap);
        (a_l * (1.0 - w) + b_l * w, a_r * (1.0 - w) + b_r * w)
    } else {
        let (d, filled) = (st.read_delay, st.filled);
        (
            tap_or_zero(&st.line_l, d, filled, max_tap),
            tap_or_zero(&st.line_r, d, filled, max_tap),
        )
    }
}

fn apply_comp(
    tc: &TrackComp,
    left: &mut [f32],
    right: &mut [f32],
    playhead: u64,
    transition: usize,
) -> bool {
    let Some(mut st) = tc.cell.state.try_lock() else {
        return false;
    };
    let st = &mut *st;
    let frames = left.len().min(right.len());
    let carrying = begin_block(tc, st, playhead, transition);
    st.next_playhead = Some(playhead + frames as u64);
    let max_tap = tc.cell.max_tap;
    for f in 0..frames {
        (left[f], right[f]) = step(st, max_tap, left[f], right[f]);
    }
    carrying
}

/// Published delay lines for both compensation stages (module doc):
/// per-track lines (stage 1), per-bus lines plus the shared dry line for
/// master-direct signals (stage 2). Built off the audio thread
/// ([`LatencyComp::new`] / [`LatencyComp::following`] allocate); the
/// audio thread only looks up entries and streams through pre-allocated
/// [`DelayLine`]s.
pub struct LatencyComp {
    /// Total pipeline latency: `max_track_chain + max_bus_chain`.
    max_latency: u64,
    /// Bus-stage latency every routed path picks up (`max_bus_chain`).
    bus_stage: u64,
    tracks: HashMap<TrackId, TrackComp>,
    busses: HashMap<BusId, TrackComp>,
    /// Shared delay for the master-direct sum, `bus_stage` samples, so
    /// dry paths arrive together with signals that traversed a bus.
    /// `None` when the bus stage is latency-free (and never was, for a
    /// table built by [`LatencyComp::following`]).
    dry: Option<TrackComp>,
    /// Frames a line whose delay this table changed crossfades over (0 =
    /// switch on the sample). Only [`LatencyComp::following`] sets it.
    transition: usize,
}

impl LatencyComp {
    /// A comp table with no delays — the startup / no-latency-plugins
    /// state. `apply` is a single failed HashMap lookup per track.
    pub fn empty() -> Self {
        Self {
            max_latency: 0,
            bus_stage: 0,
            tracks: HashMap::new(),
            busses: HashMap::new(),
            dry: None,
            transition: 0,
        }
    }

    /// Build delay lines for every track / bus with a non-zero delay,
    /// plus the shared dry line when the bus stage carries latency.
    /// `track_max`/`track_delays` and `bus_max`/`bus_delays` come from
    /// [`compensation_delays`] over [`chain_latencies`] and
    /// [`bus_chain_latencies`] respectively.
    ///
    /// Every line starts empty — right for an offline render, which
    /// builds its own table per run. A table replacing one the live
    /// callback is playing through is [`LatencyComp::following`].
    /// Allocates — never call on the audio thread.
    pub fn new(
        track_max: u64,
        track_delays: &[(TrackId, u64)],
        bus_max: u64,
        bus_delays: &[(BusId, u64)],
    ) -> Self {
        let bus_stage = bus_max.min(MAX_COMP_LATENCY);
        Self {
            max_latency: track_max.min(MAX_COMP_LATENCY) + bus_stage,
            bus_stage,
            tracks: build_comp_map(track_delays),
            busses: build_comp_map(bus_delays),
            dry: (bus_stage > 0).then(|| TrackComp::fresh(bus_stage as usize)),
            transition: 0,
        }
    }

    /// The table that replaces `prev` while the callback plays through it
    /// (code review RT-04). Arguments as [`LatencyComp::new`], plus the
    /// crossfade length for a changed delay.
    ///
    /// Before, every latency-affecting edit during playback published a
    /// table of empty lines: every compensated track and bus warmed up
    /// from `delay` samples of silence, even the ones whose delay had not
    /// changed. Now:
    ///
    /// - an entry whose delay is unchanged keeps `prev`'s line — its
    ///   output is sample-for-sample what it would have been;
    /// - an entry whose delay changed keeps the line if the new delay
    ///   fits (else a bigger line copies the old history on first use),
    ///   and its output crossfades from the old delay to the new one over
    ///   `transition_frames`. Both taps read the same history, so the
    ///   change is a time shift with no dropout and no step. The engine
    ///   passes the bypass fade length: a chain bypass toggle is what
    ///   usually moves the table, and the whole mix then fades between
    ///   the old alignment and the new one over the same few
    ///   milliseconds the bypassed chain fades between wet and dry;
    /// - an entry whose delay fell to 0 keeps its line as a pass-through
    ///   that still records history, so toggling back replays real audio
    ///   instead of silence.
    ///
    /// Allocates — never call on the audio thread.
    pub fn following(
        prev: &LatencyComp,
        track_max: u64,
        track_delays: &[(TrackId, u64)],
        bus_max: u64,
        bus_delays: &[(BusId, u64)],
        transition_frames: usize,
    ) -> Self {
        let bus_stage = bus_max.min(MAX_COMP_LATENCY);
        let dry = match &prev.dry {
            Some(old) => Some(TrackComp::follow(old, bus_stage as usize)),
            None => (bus_stage > 0).then(|| TrackComp::fresh(bus_stage as usize)),
        };
        Self {
            max_latency: track_max.min(MAX_COMP_LATENCY) + bus_stage,
            bus_stage,
            tracks: follow_comp_map(&prev.tracks, track_delays),
            busses: follow_comp_map(&prev.busses, bus_delays),
            dry,
            transition: transition_frames,
        }
    }

    /// The whole pipeline's latency in samples
    /// (`max_track_chain + max_bus_chain`).
    pub fn max_latency(&self) -> u64 {
        self.max_latency
    }

    /// The bus-stage latency (`max_bus_chain`) every routed signal picks
    /// up between the track stage and master.
    pub fn bus_stage(&self) -> u64 {
        self.bus_stage
    }

    /// The track-stage latency (`max_track_chain`): how late — relative
    /// to its timeline position — a track's audio leaves the per-track
    /// delay lines. Post-PDC parameters (fader/pan/mute automation)
    /// must be evaluated this many samples behind the raw playhead to
    /// act on the audio they were drawn against (doc #260 finding #9).
    pub fn track_stage(&self) -> u64 {
        self.max_latency - self.bus_stage
    }

    /// True when this table delays nothing. (A table built by
    /// [`LatencyComp::following`] may still hold zero-delay pass-through
    /// lines that keep history.)
    pub fn is_empty(&self) -> bool {
        self.tracks.values().all(|t| t.delay == 0)
            && self.busses.values().all(|t| t.delay == 0)
            && self.dry.as_ref().is_none_or(|t| t.delay == 0)
    }

    /// The delay this comp table applies to `track_id` (0 if none).
    pub fn delay_for(&self, track_id: TrackId) -> u64 {
        self.tracks
            .get(&track_id)
            .map(|t| t.delay as u64)
            .unwrap_or(0)
    }

    /// True when both stage maxima and the non-zero entries of both
    /// stages match this table exactly — used by the engine thread to
    /// skip republishing on topology edits that don't change any
    /// compensation amount. The maxima must be compared too, not just
    /// the per-id delays: a topology change can shift every chain
    /// latency equally (single track, all tracks equal, a multi-output
    /// parent's instrument), leaving all *relative* delays identical
    /// while `max_latency` / `track_stage()` move — those feed post-PDC
    /// automation timing and must not go stale. Arguments mirror
    /// [`LatencyComp::new`].
    pub fn delays_match(
        &self,
        track_max: u64,
        track_delays: &[(TrackId, u64)],
        bus_max: u64,
        bus_delays: &[(BusId, u64)],
    ) -> bool {
        self.bus_stage == bus_max.min(MAX_COMP_LATENCY)
            && self.max_latency == track_max.min(MAX_COMP_LATENCY) + self.bus_stage
            && stage_matches(&self.tracks, track_delays)
            && stage_matches(&self.busses, bus_delays)
    }

    /// Delay `track_id`'s buffers in place. `left`/`right` hold exactly
    /// the block being rendered and `playhead` is the timeline frame of
    /// its first sample. Returns true when a delay was applied (the
    /// caller must then treat the block as carrying audio, since a
    /// delayed tail can outlive the track's own sources). Allocation-
    /// free; the per-track mutex is uncontended by construction (one
    /// consumer per comp instance) and skipped defensively if not.
    pub fn apply(&self, track_id: TrackId, left: &mut [f32], right: &mut [f32], playhead: u64) -> bool {
        let Some(tc) = self.tracks.get(&track_id) else {
            return false;
        };
        apply_comp(tc, left, right, playhead, self.transition)
    }

    /// Delay `bus_id`'s summing buffers in place (bus stage, applied
    /// post-chain so every bus exit is `bus_stage` samples after the
    /// routing plane). Same contract as [`apply`](Self::apply).
    pub fn apply_bus(&self, bus_id: BusId, left: &mut [f32], right: &mut [f32], playhead: u64) -> bool {
        let Some(tc) = self.busses.get(&bus_id) else {
            return false;
        };
        apply_comp(tc, left, right, playhead, self.transition)
    }

    /// Tell every delay line that the next block, starting at `loop_in`,
    /// continues the one that just ended at `loop_out` (code review
    /// MIX-03). Called by the loop seam between its two sub-blocks.
    ///
    /// A wrap is a jump on the timeline but not on the output: the audio
    /// in a line at the seam is the last `delay` samples before
    /// `loop_out`, which is exactly what must be heard next (everything
    /// reaches master `max_latency` late, and the latent track's own
    /// plugin plays its pre-seam tail regardless). Without this the
    /// continuity check read the wrap as a seek and every compensated
    /// track/bus dropped to `delay` samples of silence on each pass.
    ///
    /// Only a line that ended exactly on `loop_out` is rebased — one that
    /// skipped the head sub-block (muted, say) stays discontinuous and
    /// warms up as before. Allocation-free and lock-free (`try_lock`, as
    /// in `apply`); a no-op for an empty table.
    pub fn continue_across_loop_wrap(&self, loop_out: u64, loop_in: u64) {
        let rebase_cell = |cell: &DelayCell| {
            if let Some(mut st) = cell.state.try_lock() {
                if st.next_playhead == Some(loop_out) {
                    st.next_playhead = Some(loop_in);
                }
            }
        };
        let rebase = |tc: &TrackComp| {
            rebase_cell(&tc.cell);
            // A line still to adopt its predecessor's history checks the
            // predecessor's continuity, so it rides the wrap too.
            if let Some(pred) = &tc.predecessor {
                rebase_cell(pred);
            }
        };
        self.tracks.values().for_each(rebase);
        self.busses.values().for_each(rebase);
        if let Some(dry) = &self.dry {
            rebase(dry);
        }
    }

    /// Delay the master-direct (dry) sum in place by `bus_stage`
    /// samples, so signals routed straight to master arrive together
    /// with signals that traversed a bus chain. `data` is the
    /// interleaved output holding exactly the master-direct
    /// contributions of this block (the caller runs this after the
    /// track pass and before any bus sums into the output). Only the
    /// first two channels are delayed — the mixer never writes beyond
    /// them. No-op (returns false) while the bus stage is latency-free,
    /// which keeps the common no-bus-latency path byte-identical.
    pub fn apply_dry(&self, data: &mut [f32], channels: usize, frames: usize, playhead: u64) -> bool {
        let Some(tc) = &self.dry else {
            return false;
        };
        let Some(mut st) = tc.cell.state.try_lock() else {
            return false;
        };
        let st = &mut *st;
        let carrying = begin_block(tc, st, playhead, self.transition);
        st.next_playhead = Some(playhead + frames as u64);
        let max_tap = tc.cell.max_tap;
        for f in 0..frames {
            let idx = f * channels;
            let r_in = if channels >= 2 { data[idx + 1] } else { 0.0 };
            let (l, r) = step(st, max_tap, data[idx], r_in);
            data[idx] = l;
            if channels >= 2 {
                data[idx + 1] = r;
            }
        }
        carrying
    }
}
