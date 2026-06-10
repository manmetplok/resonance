//! Plugin-delay compensation (PDC).
//!
//! Model: each plugin's latency is read from its `clap.latency`
//! extension once, right after activation (the host vtable doesn't
//! implement `clap_host_latency`, so latency changes after activation
//! are not tracked). A track's *chain latency* is the sum of its own
//! plugin latencies, plus — for sub-tracks — the parent's instrument
//! (first plugin), plus the chain of the bus the track routes to
//! (bus processing happens downstream of the track, so it contributes
//! to when that track's audio reaches master). Every track is then
//! delayed by `max_chain_latency − its_chain_latency` right after its
//! plugin chain and before fader/pan/routing, so all paths arrive at
//! master with the same total latency.
//!
//! Compensated: track chains, sub-track chains (incl. the shared
//! parent instrument), and bus chains (via the per-track delay).
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
use crate::types::{Bus, BusId, PluginInstanceId, Track, TrackId, TrackOutput};

/// Total chain latency per track (see the module doc for what counts
/// toward a chain). Returns one entry per track, sub-tracks included.
/// `plugin_latency` resolves one plugin instance's latency in samples.
pub fn chain_latencies(
    tracks: &IndexMap<TrackId, Track>,
    busses: &IndexMap<BusId, Bus>,
    plugin_latency: impl Fn(PluginInstanceId) -> u64,
) -> Vec<(TrackId, u64)> {
    let bus_latency: HashMap<BusId, u64> = busses
        .iter()
        .map(|(&id, bus)| {
            (
                id,
                bus.plugin_ids.iter().map(|&p| plugin_latency(p)).sum(),
            )
        })
        .collect();
    tracks
        .values()
        .map(|track| {
            let own: u64 = track.plugins().iter().map(|&p| plugin_latency(p)).sum();
            // Sub-tracks are fed by the parent's instrument (first
            // plugin), so they inherit its latency on top of their own
            // FX chain.
            let parent_instrument = track
                .sub_track_of
                .and_then(|(parent_id, _)| tracks.get(&parent_id))
                .and_then(|parent| parent.plugins().first().copied())
                .map(&plugin_latency)
                .unwrap_or(0);
            let bus = match track.output() {
                TrackOutput::Bus(bus_id) => bus_latency.get(&bus_id).copied().unwrap_or(0),
                TrackOutput::Master => 0,
            };
            (track.id, own + parent_instrument + bus)
        })
        .collect()
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

/// Published per-track delay lines. Built off the audio thread
/// ([`LatencyComp::new`] allocates); the audio thread only looks up
/// tracks and streams through pre-allocated [`DelayLine`]s.
pub struct LatencyComp {
    max_latency: u64,
    tracks: HashMap<TrackId, TrackComp>,
}

impl LatencyComp {
    /// A comp table with no delays — the startup / no-latency-plugins
    /// state. `apply` is a single failed HashMap lookup per track.
    pub fn empty() -> Self {
        Self {
            max_latency: 0,
            tracks: HashMap::new(),
        }
    }

    /// Build delay lines for every track with a non-zero delay.
    /// Allocates — never call on the audio thread.
    pub fn new(max_latency: u64, delays: &[(TrackId, u64)]) -> Self {
        let tracks = delays
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
            .collect();
        Self {
            max_latency,
            tracks,
        }
    }

    /// The whole pipeline's latency in samples (max chain latency).
    pub fn max_latency(&self) -> u64 {
        self.max_latency
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    /// The delay this comp table applies to `track_id` (0 if none).
    pub fn delay_for(&self, track_id: TrackId) -> u64 {
        self.tracks
            .get(&track_id)
            .map(|t| t.delay as u64)
            .unwrap_or(0)
    }

    /// True when the non-zero entries of `delays` match this table
    /// exactly — used by the engine thread to skip republishing (and
    /// thereby resetting every delay line) on topology edits that don't
    /// change any compensation amount.
    pub fn delays_match(&self, delays: &[(TrackId, u64)]) -> bool {
        let nonzero = delays.iter().filter(|&&(_, d)| d > 0);
        nonzero.clone().count() == self.tracks.len()
            && nonzero
                .clone()
                .all(|&(id, d)| self.tracks.get(&id).map(|t| t.delay as u64) == Some(d))
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
}
