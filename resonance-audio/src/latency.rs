//! Plugin-delay compensation (PDC).
//!
//! Model: each plugin's latency is read from its `clap.latency`
//! extension once, right after activation (the host vtable doesn't
//! implement `clap_host_latency`, so latency changes after activation
//! are not tracked). Compensation runs in two stages so aux sends stay
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
//! lines. The offline bounce renderer builds its own instance per run
//! and additionally trims the leading `max_latency` frames so bounced
//! audio lands exactly on the timeline.

use std::collections::HashMap;

use indexmap::IndexMap;
use parking_lot::Mutex;
use resonance_dsp::DelayLine;

use crate::limits::MAX_COMP_LATENCY;
use crate::types::{Bus, BusId, PluginInstanceId, Track, TrackId, TrackType};

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
///   only way to bypass a latency-carrying plugin without re-publishing
///   (and thereby resetting) every delay line.
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
    tracks: &IndexMap<TrackId, Track>,
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
pub fn bus_chain_latencies(
    busses: &IndexMap<BusId, Bus>,
    plugin_latency: impl Fn(PluginInstanceId) -> u64,
) -> Vec<(BusId, u64)> {
    busses
        .iter()
        .map(|(&id, bus)| {
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
    /// A mismatch (seek, loop wrap, a track that was skipped while
    /// muted) clears the lines so stale audio doesn't replay.
    next_playhead: Option<u64>,
}

struct TrackComp {
    delay: usize,
    state: Mutex<DelayState>,
}

/// Build the delay-line table for one compensation stage (tracks or
/// busses): one entry per non-zero delay.
fn build_comp_map(delays: &[(u64, u64)]) -> HashMap<u64, TrackComp> {
    delays
        .iter()
        .filter(|&&(_, d)| d > 0)
        .map(|&(id, d)| {
            let delay = d.min(MAX_COMP_LATENCY) as usize;
            (
                id,
                TrackComp {
                    delay,
                    // +1 so `tap(delay)` stays within capacity even
                    // when `delay` is itself a power of two.
                    state: Mutex::new(DelayState {
                        line_l: DelayLine::new(delay + 1),
                        line_r: DelayLine::new(delay + 1),
                        next_playhead: None,
                    }),
                },
            )
        })
        .collect()
}

fn stage_matches(map: &HashMap<u64, TrackComp>, delays: &[(u64, u64)]) -> bool {
    let nonzero = delays.iter().filter(|&&(_, d)| d > 0);
    nonzero.clone().count() == map.len()
        && nonzero
            .clone()
            .all(|&(id, d)| map.get(&id).map(|t| t.delay as u64) == Some(d))
}

fn apply_comp(tc: &TrackComp, left: &mut [f32], right: &mut [f32], playhead: u64) -> bool {
    let Some(mut st) = tc.state.try_lock() else {
        return false;
    };
    let frames = left.len().min(right.len());
    if st.next_playhead != Some(playhead) {
        st.line_l.clear();
        st.line_r.clear();
    }
    st.next_playhead = Some(playhead + frames as u64);
    for f in 0..frames {
        st.line_l.push(left[f]);
        left[f] = st.line_l.tap(tc.delay);
        st.line_r.push(right[f]);
        right[f] = st.line_r.tap(tc.delay);
    }
    true
}

/// Published delay lines for both compensation stages (module doc):
/// per-track lines (stage 1), per-bus lines plus the shared dry line for
/// master-direct signals (stage 2). Built off the audio thread
/// ([`LatencyComp::new`] allocates); the audio thread only looks up
/// entries and streams through pre-allocated [`DelayLine`]s.
pub struct LatencyComp {
    /// Total pipeline latency: `max_track_chain + max_bus_chain`.
    max_latency: u64,
    /// Bus-stage latency every routed path picks up (`max_bus_chain`).
    bus_stage: u64,
    tracks: HashMap<TrackId, TrackComp>,
    busses: HashMap<BusId, TrackComp>,
    /// Shared delay for the master-direct sum, `bus_stage` samples, so
    /// dry paths arrive together with signals that traversed a bus.
    /// `None` when the bus stage is latency-free.
    dry: Option<TrackComp>,
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
        }
    }

    /// Build delay lines for every track / bus with a non-zero delay,
    /// plus the shared dry line when the bus stage carries latency.
    /// `track_max`/`track_delays` and `bus_max`/`bus_delays` come from
    /// [`compensation_delays`] over [`chain_latencies`] and
    /// [`bus_chain_latencies`] respectively.
    /// Allocates — never call on the audio thread.
    pub fn new(
        track_max: u64,
        track_delays: &[(TrackId, u64)],
        bus_max: u64,
        bus_delays: &[(BusId, u64)],
    ) -> Self {
        let bus_stage = bus_max.min(MAX_COMP_LATENCY);
        let dry = (bus_stage > 0).then(|| {
            let delay = bus_stage as usize;
            TrackComp {
                delay,
                state: Mutex::new(DelayState {
                    line_l: DelayLine::new(delay + 1),
                    line_r: DelayLine::new(delay + 1),
                    next_playhead: None,
                }),
            }
        });
        Self {
            max_latency: track_max.min(MAX_COMP_LATENCY) + bus_stage,
            bus_stage,
            tracks: build_comp_map(track_delays),
            busses: build_comp_map(bus_delays),
            dry,
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

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty() && self.busses.is_empty() && self.dry.is_none()
    }

    /// The delay this comp table applies to `track_id` (0 if none).
    pub fn delay_for(&self, track_id: TrackId) -> u64 {
        self.tracks
            .get(&track_id)
            .map(|t| t.delay as u64)
            .unwrap_or(0)
    }

    /// True when the non-zero entries of both stages match this table
    /// exactly — used by the engine thread to skip republishing (and
    /// thereby resetting every delay line) on topology edits that don't
    /// change any compensation amount.
    pub fn delays_match(
        &self,
        track_delays: &[(TrackId, u64)],
        bus_delays: &[(BusId, u64)],
        bus_max: u64,
    ) -> bool {
        self.bus_stage == bus_max.min(MAX_COMP_LATENCY)
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
        apply_comp(tc, left, right, playhead)
    }

    /// Delay `bus_id`'s summing buffers in place (bus stage, applied
    /// post-chain so every bus exit is `bus_stage` samples after the
    /// routing plane). Same contract as [`apply`](Self::apply).
    pub fn apply_bus(&self, bus_id: BusId, left: &mut [f32], right: &mut [f32], playhead: u64) -> bool {
        let Some(tc) = self.busses.get(&bus_id) else {
            return false;
        };
        apply_comp(tc, left, right, playhead)
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
        let Some(mut st) = tc.state.try_lock() else {
            return false;
        };
        if st.next_playhead != Some(playhead) {
            st.line_l.clear();
            st.line_r.clear();
        }
        st.next_playhead = Some(playhead + frames as u64);
        for f in 0..frames {
            let idx = f * channels;
            st.line_l.push(data[idx]);
            data[idx] = st.line_l.tap(tc.delay);
            if channels >= 2 {
                st.line_r.push(data[idx + 1]);
                data[idx + 1] = st.line_r.tap(tc.delay);
            }
        }
        true
    }
}
