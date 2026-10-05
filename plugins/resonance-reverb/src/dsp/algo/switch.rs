//! The engine bank and the algorithm switch (reverb-algorithms.md §4.2).
//!
//! Every engine is built up front, one slot each; the bank owns them and
//! runs one, or two while a switch is fading. A switch never allocates.
//!
//! **The fade.** For `SWITCH_FADE_MS` the outgoing engine's input *and*
//! output ramp linearly from 1 to 0 while the incoming engine, cleared
//! and configured on the spot, gets its input ramped from 0 to 1. Both
//! ramps matter: a delay network fed a step (a sustained signal switched
//! on into empty lines, or cut off) emits that step again at every tap,
//! which no output gain short of zero hides. Ramping the inputs makes
//! every tap's onset and release continuous; ramping the outgoing output
//! retires its tail on time. A switch requested while one is fading is
//! queued, newest wins, and starts when the running one completes — the
//! pattern `er.rs` and the pre-delay use.
//!
//! **Freeze defers a switch.** A frozen engine holds its tail with its
//! input muted, so an incoming engine would start frozen and empty and
//! the fade would retire the held sound into silence. While Freeze is on
//! a request is queued instead (newest wins), and it starts with the
//! normal fade the moment Freeze is released.
//!
//! Outside a fade the active engine is fed the input untouched, so a bank
//! that never switches renders bit-identically to the engine alone.

use super::{Algorithm, Engine, Extras, Wet};

/// Algorithm switch crossfade length.
pub(crate) const SWITCH_FADE_MS: f32 = 50.0;

/// Every parameter an engine reads, as last set. `None` until the first
/// set, so configuring a freshly activated engine replays exactly the
/// calls the running one received — nothing invented.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EngineConfig {
    pub size: Option<f32>,
    pub decay: Option<f32>,
    pub freeze: Option<bool>,
    pub damping: Option<f32>,
    pub er_level: Option<f32>,
    pub er_time: Option<f32>,
    pub mod_rate: Option<f32>,
    pub mod_depth: Option<f32>,
    /// `(low_decay_mult, low_xover_hz, high_decay_mult)`.
    pub decay_shape: Option<(f32, f32, f32)>,
    pub build: Option<f32>,
    pub extras: Option<Extras>,
}

impl EngineConfig {
    /// Apply every set value to `e`, in the order the plugin's block loop
    /// sets them (size before decay: Classic derives its loop gain from
    /// the size; freeze after decay, as freeze overrides it).
    fn apply(&self, e: &mut Engine) {
        if let Some(v) = self.size {
            e.set_size(v);
        }
        if let Some(v) = self.decay {
            e.set_decay(v);
        }
        if let Some(v) = self.freeze {
            e.set_freeze(v);
        }
        if let Some(v) = self.damping {
            e.set_damping(v);
        }
        if let Some(v) = self.er_level {
            e.set_er_level(v);
        }
        if let Some(v) = self.er_time {
            e.set_er_time(v);
        }
        if let Some(v) = self.mod_rate {
            e.set_mod_rate(v);
        }
        if let Some(v) = self.mod_depth {
            e.set_mod_depth(v);
        }
        if let Some((lo, xover, hi)) = self.decay_shape {
            e.set_decay_shape(lo, xover, hi);
        }
        if let Some(v) = self.build {
            e.set_build(v);
        }
        if let Some(x) = self.extras {
            e.set_extras(&x);
        }
    }
}

pub(crate) struct EngineBank {
    /// One engine per slot, built at construction.
    engines: Vec<Engine>,
    /// What each engine was last told (replayed onto an incoming one).
    pub(crate) cfg: EngineConfig,
    /// The slot receiving input.
    active: usize,
    /// The slot fading out while a switch runs.
    outgoing: Option<usize>,
    /// Remaining fade samples; 0 means no fade is running.
    fade_left: u32,
    fade_total: u32,
    /// Newest slot requested while a fade was running.
    pending: Option<usize>,
    /// Last slot requested — dedupes the per-block requests.
    requested: usize,
    /// False until the first processed sample. While false a switch snaps
    /// (a fresh or reset instance has nothing audible to fade from).
    primed: bool,
}

impl EngineBank {
    /// A bank with one slot per entry of `slots`, starting on slot 0.
    pub(crate) fn new(slots: &[Algorithm], sample_rate: f32) -> Self {
        assert!(!slots.is_empty(), "an engine bank needs at least one engine");
        Self {
            engines: slots.iter().map(|&a| Engine::new(a, sample_rate)).collect(),
            cfg: EngineConfig::default(),
            active: 0,
            outgoing: None,
            fade_left: 0,
            fade_total: ((SWITCH_FADE_MS * 0.001 * sample_rate) as u32).max(1),
            pending: None,
            requested: 0,
            primed: false,
        }
    }

    /// The first slot holding `algorithm`, if the bank has one.
    pub(crate) fn slot_of(&self, algorithm: Algorithm) -> Option<usize> {
        self.engines.iter().position(|e| e.algorithm() == algorithm)
    }

    pub(crate) fn slot_count(&self) -> usize {
        self.engines.len()
    }

    pub(crate) fn active_slot(&self) -> usize {
        self.active
    }

    pub(crate) fn switching(&self) -> bool {
        self.fade_left > 0
    }

    fn frozen(&self) -> bool {
        self.cfg.freeze == Some(true)
    }

    /// Freeze or release every live engine. Releasing starts a switch
    /// that was deferred while frozen.
    pub(crate) fn set_freeze(&mut self, freeze: bool) {
        self.cfg.freeze = Some(freeze);
        self.for_live(|e| e.set_freeze(freeze));
        if !freeze && self.fade_left == 0 {
            self.start_pending();
        }
    }

    fn start_pending(&mut self) {
        if let Some(next) = self.pending.take() {
            if next != self.active {
                self.begin(next);
            }
        }
    }

    pub(crate) fn active(&self) -> &Engine {
        &self.engines[self.active]
    }

    /// Run `f` on every engine that is producing sound: the active one,
    /// and the outgoing one while it fades. Idle engines are configured
    /// when they become active instead.
    pub(crate) fn for_live(&mut self, mut f: impl FnMut(&mut Engine)) {
        f(&mut self.engines[self.active]);
        if let Some(o) = self.outgoing {
            f(&mut self.engines[o]);
        }
    }

    /// Ask for `slot` to be the active engine. Out-of-range slots are
    /// ignored.
    pub(crate) fn request(&mut self, slot: usize) {
        if slot >= self.engines.len() || slot == self.requested {
            return;
        }
        self.requested = slot;
        if !self.primed {
            self.active = slot;
            self.activate(slot);
        } else if self.fade_left > 0 || self.frozen() {
            self.pending = Some(slot);
        } else if slot != self.active {
            self.begin(slot);
        }
    }

    /// Clear `slot` and replay the config onto it.
    fn activate(&mut self, slot: usize) {
        let cfg = self.cfg;
        let e = &mut self.engines[slot];
        e.clear();
        cfg.apply(e);
    }

    fn begin(&mut self, slot: usize) {
        self.outgoing = Some(self.active);
        self.active = slot;
        self.activate(slot);
        self.fade_left = self.fade_total;
    }

    #[inline]
    pub(crate) fn process(&mut self, l: f32, r: f32, diffusion: f32) -> Wet {
        self.primed = true;
        let Some(out) = self.outgoing else {
            return self.engines[self.active].process(l, r, diffusion);
        };
        self.fade_left -= 1;
        let x = (self.fade_total - self.fade_left) as f32 / self.fade_total as f32;
        let g = 1.0 - x;
        let a = self.engines[self.active].process(l * x, r * x, diffusion);
        let o = self.engines[out].process(l * g, r * g, diffusion);
        if self.fade_left == 0 {
            self.outgoing = None;
            if !self.frozen() {
                self.start_pending();
            }
        }
        Wet {
            er_l: a.er_l + o.er_l * g,
            er_r: a.er_r + o.er_r * g,
            late_l: a.late_l + o.late_l * g,
            late_r: a.late_r + o.late_r * g,
        }
    }

    /// Forget all audio. Lands on the newest requested slot with no fade,
    /// the state a fresh bank configured once is in.
    pub(crate) fn clear(&mut self) {
        // Only the engines that have run since their activation hold
        // audio: an idle engine is cleared by `activate` when it is next
        // switched to, so clearing it here too would be a wasted memset
        // of its every buffer on the audio thread.
        self.fade_left = 0;
        self.pending = None;
        if let Some(o) = self.outgoing.take() {
            self.engines[o].clear();
        }
        self.engines[self.active].clear();
        if self.active != self.requested {
            // A reset mid-fade: the newest request wins, configured
            // exactly as a snap on a fresh bank would configure it.
            self.active = self.requested;
            self.activate(self.active);
        }
        self.primed = false;
    }
}
