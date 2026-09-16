//! External sidechain (key) routing: feeding one track's or bus's audio
//! into another track's plugin as a detection signal.
//!
//! A plugin that declares a key port (`resonance-plugin`'s
//! `SIDECHAIN_INPUT` — the compressor and the gate do) gets handed a
//! second, non-main CLAP input port. What arrives there is decided here:
//! a [`SidechainRoute`] names the plugin instance and the source whose
//! audio should key it.
//!
//! # Why the key is one block old
//!
//! The obvious implementation taps the source as it renders and hands it
//! straight to the destination. That only works when the source happens
//! to render first, which depends on track order — so "duck the pad from
//! the kick" would behave differently depending on which lane the user
//! dragged where, and routing a track from something downstream of itself
//! would deadlock the ordering entirely.
//!
//! Instead, [`SidechainTaps`] double-buffers: each block writes the
//! current audio into one bank and every key reads the *other*, holding
//! the previous block. The result is deterministic regardless of track
//! order, immune to cycles by construction (a track may legally key off
//! itself), and costs one block of latency on the detector — 2.7 ms at
//! the 128-frame quantum this engine runs, which is below the attack time
//! of any usable ducker and is what the sidechain path in most DAWs does
//! for exactly this reason.
//!
//! The tap is taken **post-FX, pre-fader**: post-FX because you want the
//! processed kick, and pre-fader so riding the source's fader doesn't
//! silently change how hard it ducks something else.

use crate::types::{BusId, PluginInstanceId, SendSource, TrackId};

/// Most distinct key sources one project can have active at once. Bounded
/// because the tap buffers are pre-allocated at mixer construction — the
/// audio thread never allocates, so it cannot grow this on demand.
/// Routes naming sources beyond the cap are ignored rather than
/// misrouted; the engine reports them when the route is set.
pub const MAX_SIDECHAIN_SOURCES: usize = 8;

/// One key routing: `plugin`'s sidechain input reads `source`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SidechainRoute {
    /// The plugin instance being keyed. It must declare a sidechain port;
    /// a route onto a plugin without one is inert.
    pub plugin: PluginInstanceId,
    /// Whose audio feeds the key.
    pub source: SendSource,
    /// A disabled route keeps its configuration but delivers no key, so
    /// the plugin falls back to keying off its own input.
    pub enabled: bool,
}

impl SidechainRoute {
    pub fn new(plugin: PluginInstanceId, source: SendSource) -> Self {
        Self {
            plugin,
            source,
            enabled: true,
        }
    }
}

/// The source a plugin's key should read this block, or `None` when the
/// plugin has no enabled route.
pub fn route_source(routes: &[SidechainRoute], plugin: PluginInstanceId) -> Option<SendSource> {
    routes
        .iter()
        .find(|r| r.plugin == plugin && r.enabled)
        .map(|r| r.source)
}

/// The distinct sources named by the enabled routes: a fixed-capacity,
/// stack-held set of at most [`MAX_SIDECHAIN_SOURCES`] entries. Built
/// fresh every block by [`SidechainTaps::begin_block`], so it must not
/// heap-allocate — the audio thread never allocates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActiveSources {
    /// Only the first `len` entries are meaningful; the tail keeps a
    /// filler value so the array is fully initialized.
    sources: [SendSource; MAX_SIDECHAIN_SOURCES],
    len: usize,
}

impl ActiveSources {
    /// The collected sources, in first-seen order.
    pub fn as_slice(&self) -> &[SendSource] {
        &self.sources[..self.len]
    }

    pub fn iter(&self) -> impl Iterator<Item = SendSource> + '_ {
        self.as_slice().iter().copied()
    }

    pub fn contains(&self, source: SendSource) -> bool {
        self.as_slice().contains(&source)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Every distinct source named by an enabled route, in first-seen order,
/// capped at [`MAX_SIDECHAIN_SOURCES`]. Allocation-free (the result
/// lives on the caller's stack) because the playing branch calls this
/// once per audio callback. Pure so the slot assignment can be tested
/// without a mixer.
pub fn active_sources(routes: &[SidechainRoute]) -> ActiveSources {
    let mut out = ActiveSources {
        sources: [SendSource::Track(0); MAX_SIDECHAIN_SOURCES],
        len: 0,
    };
    for route in routes.iter().filter(|r| r.enabled) {
        if !out.contains(route.source) && out.len < MAX_SIDECHAIN_SOURCES {
            out.sources[out.len] = route.source;
            out.len += 1;
        }
    }
    out
}

/// One source's captured audio, double-buffered.
struct TapSlot {
    source: Option<SendSource>,
    /// `[bank][channel]`. One bank is written this block while the other
    /// is read; they swap at the block boundary.
    banks: [(Vec<f32>, Vec<f32>); 2],
    /// Whether the read bank actually received audio last block. A source
    /// that produced nothing keys as silence rather than as stale audio.
    written: [bool; 2],
}

/// Per-source capture buffers owned by the mixer (audio thread).
///
/// Allocation happens once, in [`SidechainTaps::new`]. Everything on the
/// audio thread is a linear scan over at most [`MAX_SIDECHAIN_SOURCES`]
/// slots plus a `copy_from_slice`.
pub struct SidechainTaps {
    slots: Vec<TapSlot>,
    /// Which bank the current block WRITES into; the other is read.
    write_bank: usize,
    max_frames: usize,
}

impl SidechainTaps {
    /// Pre-allocate every slot for a maximum block size.
    pub fn new(max_frames: usize) -> Self {
        let max_frames = max_frames.max(1);
        let slots = (0..MAX_SIDECHAIN_SOURCES)
            .map(|_| TapSlot {
                source: None,
                banks: [
                    (vec![0.0; max_frames], vec![0.0; max_frames]),
                    (vec![0.0; max_frames], vec![0.0; max_frames]),
                ],
                written: [false; 2],
            })
            .collect();
        Self {
            slots,
            write_bank: 0,
            max_frames,
        }
    }

    /// Start a block: swap banks (so this block reads what the last one
    /// captured) and re-assign slots to the currently-routed sources.
    ///
    /// Slot assignment is stable — a source that keeps its slot keeps its
    /// captured audio across the change, so enabling an unrelated route
    /// doesn't drop a key for one block.
    pub fn begin_block(&mut self, routes: &[SidechainRoute]) {
        self.write_bank ^= 1;
        let wanted = active_sources(routes);

        // Release slots whose source is no longer routed.
        for slot in self.slots.iter_mut() {
            if let Some(src) = slot.source {
                if !wanted.contains(src) {
                    slot.source = None;
                    slot.written = [false; 2];
                }
            }
        }
        // Assign the new ones into free slots.
        for src in wanted.iter() {
            if self.slots.iter().any(|s| s.source == Some(src)) {
                continue;
            }
            if let Some(free) = self.slots.iter_mut().find(|s| s.source.is_none()) {
                free.source = Some(src);
                free.written = [false; 2];
            }
        }
        // Nothing captured yet this block.
        let bank = self.write_bank;
        for slot in self.slots.iter_mut() {
            slot.written[bank] = false;
        }
    }

    /// True when `source` is being used as a key by some enabled route —
    /// the mixer checks this before paying for a copy.
    pub fn is_tapped(&self, source: SendSource) -> bool {
        self.slots.iter().any(|s| s.source == Some(source))
    }

    /// Capture `source`'s audio for the NEXT block's keys.
    pub fn capture(&mut self, source: SendSource, left: &[f32], right: &[f32], frames: usize) {
        let bank = self.write_bank;
        let max = self.max_frames;
        let Some(slot) = self.slots.iter_mut().find(|s| s.source == Some(source)) else {
            return;
        };
        let n = frames.min(max).min(left.len()).min(right.len());
        let (l, r) = &mut slot.banks[bank];
        l[..n].copy_from_slice(&left[..n]);
        r[..n].copy_from_slice(&right[..n]);
        // A short block leaves stale audio in the tail; zero it so the
        // key never reads samples from a longer previous block.
        l[n..].fill(0.0);
        r[n..].fill(0.0);
        slot.written[bank] = true;
    }

    /// The key signal for `source` this block — the previous block's
    /// capture. `None` when the source isn't tapped or produced nothing,
    /// which leaves the plugin keying off its own input.
    pub fn key(&self, source: SendSource) -> Option<(&[f32], &[f32])> {
        let bank = self.write_bank ^ 1;
        let slot = self.slots.iter().find(|s| s.source == Some(source))?;
        if !slot.written[bank] {
            return None;
        }
        let (l, r) = &slot.banks[bank];
        Some((l.as_slice(), r.as_slice()))
    }

    /// The key signal for `plugin`, resolving its route in one step.
    pub fn key_for(
        &self,
        routes: &[SidechainRoute],
        plugin: PluginInstanceId,
    ) -> Option<(&[f32], &[f32])> {
        self.key(route_source(routes, plugin)?)
    }

    /// Drop every captured signal (transport stop / plugin reload), so a
    /// key can't carry audio across a discontinuity.
    pub fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            slot.written = [false; 2];
        }
    }
}

/// Convenience: a route sourced from a track.
pub fn from_track(plugin: PluginInstanceId, track: TrackId) -> SidechainRoute {
    SidechainRoute::new(plugin, SendSource::Track(track))
}

/// Convenience: a route sourced from a bus.
pub fn from_bus(plugin: PluginInstanceId, bus: BusId) -> SidechainRoute {
    SidechainRoute::new(plugin, SendSource::Bus(bus))
}
