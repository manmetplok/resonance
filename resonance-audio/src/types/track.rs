//! Engine-side Track and Bus, with atomic hot-path accessors for the
//! audio callback.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use arc_swap::{ArcSwap, ArcSwapOption, Guard};

use super::{BusId, PluginInstanceId, TrackId, TrackOutput, TrackType};

/// Sentinel value used in `Track::output_bus_bits` to encode
/// `TrackOutput::Master` (so the enum can live in a single AtomicU64
/// for lock-free reads on the audio thread).
const TRACK_OUTPUT_MASTER: u64 = u64::MAX;

/// A track containing audio clips or MIDI clips.
///
/// Hot-path fields (volume, muted, monitor_enabled, record_armed) are atomic
/// so the audio callback can read them without taking a write lock.
#[derive(Debug)]
pub struct Track {
    pub id: TrackId,
    pub track_type: TrackType,
    volume_bits: AtomicU32,
    pan_bits: AtomicU32,
    muted: AtomicBool,
    soloed: AtomicBool,
    /// When true, the mixer skips every effect plugin on this track.
    /// Instrument plugins (the first slot on instrument tracks) still
    /// play — only the effects chain after them is bypassed.
    fx_bypassed: AtomicBool,
    pub name: String,
    record_armed: AtomicBool,
    monitor_enabled: AtomicBool,
    /// If true, track captures a single input channel (duplicated to both L/R).
    /// If false, track captures a stereo pair.
    mono: AtomicBool,
    /// Post-fader peak level for left channel (for VU meters).
    peak_l_bits: AtomicU32,
    /// Post-fader peak level for right channel (for VU meters).
    peak_r_bits: AtomicU32,
    /// Output destination, encoded as `u64::MAX` for `Master` or a bus id.
    /// Stored as an atomic so the audio thread can read the routing
    /// without taking a write lock while the UI edits it.
    output_bus_bits: AtomicU64,
    /// Hardware capture device the track records / monitors from.
    /// Stored in an `ArcSwapOption` so the engine thread can edit it
    /// from a `tracks.read()` guard — write-locking the tracks map
    /// silenced the audio callback for whatever block straddled the
    /// edit because the mixer's own `try_read` would fail.
    pub input_device_name: ArcSwapOption<String>,
    /// 0-indexed starting input channel on the track's input device. For
    /// mono tracks this is the single channel captured and duplicated to
    /// L/R; for stereo tracks it's the L channel and `port_index + 1` is
    /// used as R. Defaults to 0 (first channel pair).
    input_port_bits: AtomicU32,
    /// Ordered list of plugin instance IDs forming the insert chain.
    /// For instrument tracks, the first plugin is the instrument; the
    /// rest are effects.
    ///
    /// Wrapped in `ArcSwap` so the audio thread can load the chain
    /// without ever blocking on a `tracks.write()` guard the UI is
    /// holding to add/remove/reorder plugins. Mutations build a new
    /// `Vec` and publish it with a single atomic store; readers see
    /// either the pre-edit or post-edit chain, never a torn one.
    /// Access via `plugins()` / `push_plugin()` / `retain_plugins()` /
    /// `clear_plugins()` / `set_plugin_chain()`; the field is private
    /// so the `&mut Vec` mutation pattern can no longer compile.
    plugin_chain: ArcSwap<Vec<PluginInstanceId>>,
    /// When set, this track is a sub-track fed by a non-main output port
    /// of `parent_track_id`'s instrument plugin. Sub-tracks never run
    /// their own plugin chain or receive MIDI events — the mixer drives
    /// them entirely from the parent plugin's `process_multi` output.
    /// The tuple is `(parent_track_id, output_port_index)` where index 0
    /// is reserved for the parent's own main output.
    pub sub_track_of: Option<(TrackId, u32)>,
    /// Hardware MIDI input device name. The engine control thread
    /// reads this when applying `SetTrackMidiInput`; the audio callback
    /// never touches it.
    pub midi_input_device: Option<String>,
    /// Channel filter for hardware MIDI input. `None` = omni.
    pub midi_input_channel: Option<u8>,
    /// Hardware MIDI output device name. Read on the audio thread to
    /// decide whether timeline notes should also be ferried to the
    /// engine thread for hardware send-out — kept in an
    /// `ArcSwapOption<String>` so the audio thread reads are cheap
    /// and edits never touch a mutex.
    pub midi_output_device: ArcSwapOption<String>,
    /// Channel that hardware MIDI output uses. None = channel 1.
    /// Only read on the engine control thread.
    pub midi_output_channel: Option<u8>,
}

impl Track {
    pub fn new(id: TrackId, name: String) -> Self {
        Self::with_type(id, name, TrackType::Audio)
    }

    pub fn with_type(id: TrackId, name: String, track_type: TrackType) -> Self {
        Self {
            id,
            track_type,
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            pan_bits: AtomicU32::new(0.0f32.to_bits()),
            muted: AtomicBool::new(false),
            soloed: AtomicBool::new(false),
            fx_bypassed: AtomicBool::new(false),
            name,
            record_armed: AtomicBool::new(false),
            monitor_enabled: AtomicBool::new(false),
            mono: AtomicBool::new(true),
            peak_l_bits: AtomicU32::new(0),
            peak_r_bits: AtomicU32::new(0),
            output_bus_bits: AtomicU64::new(TRACK_OUTPUT_MASTER),
            input_device_name: ArcSwapOption::const_empty(),
            input_port_bits: AtomicU32::new(0),
            plugin_chain: ArcSwap::from_pointee(Vec::new()),
            sub_track_of: None,
            midi_input_device: None,
            midi_input_channel: None,
            midi_output_device: ArcSwapOption::const_empty(),
            midi_output_channel: None,
        }
    }

    /// The track's 0-indexed starting input channel.
    pub fn input_port(&self) -> u16 {
        (self.input_port_bits.load(Ordering::Relaxed) & 0xFFFF) as u16
    }

    pub fn set_input_port(&self, port: u16) {
        self.input_port_bits.store(port as u32, Ordering::Relaxed);
    }

    /// Construct a sub-track feeding from `parent_track_id`'s output port
    /// index `output_port_index`. Starts muted-friendly (volume 1.0,
    /// pan 0.0) and routed to master; the app layer pushes user edits
    /// via the normal `SetTrackVolume` / `SetTrackOutput` / etc. commands.
    pub fn new_sub_track(
        id: TrackId,
        name: String,
        parent_track_id: TrackId,
        output_port_index: u32,
    ) -> Self {
        let mut t = Self::with_type(id, name, TrackType::Instrument);
        t.sub_track_of = Some((parent_track_id, output_port_index));
        t
    }

    pub fn output(&self) -> TrackOutput {
        match self.output_bus_bits.load(Ordering::Relaxed) {
            TRACK_OUTPUT_MASTER => TrackOutput::Master,
            bus_id => TrackOutput::Bus(bus_id),
        }
    }

    pub fn set_output(&self, output: TrackOutput) {
        let encoded = match output {
            TrackOutput::Master => TRACK_OUTPUT_MASTER,
            TrackOutput::Bus(id) => id,
        };
        self.output_bus_bits.store(encoded, Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, v: f32) {
        self.volume_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan_bits.load(Ordering::Relaxed))
    }

    pub fn set_pan(&self, v: f32) {
        self.pan_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, v: bool) {
        self.muted.store(v, Ordering::Relaxed);
    }

    pub fn soloed(&self) -> bool {
        self.soloed.load(Ordering::Relaxed)
    }

    pub fn set_soloed(&self, v: bool) {
        self.soloed.store(v, Ordering::Relaxed);
    }

    pub fn fx_bypassed(&self) -> bool {
        self.fx_bypassed.load(Ordering::Relaxed)
    }

    pub fn set_fx_bypassed(&self, v: bool) {
        self.fx_bypassed.store(v, Ordering::Relaxed);
    }

    pub fn record_armed(&self) -> bool {
        self.record_armed.load(Ordering::Relaxed)
    }

    pub fn set_record_armed(&self, v: bool) {
        self.record_armed.store(v, Ordering::Relaxed);
    }

    pub fn monitor_enabled(&self) -> bool {
        self.monitor_enabled.load(Ordering::Relaxed)
    }

    pub fn set_monitor_enabled(&self, v: bool) {
        self.monitor_enabled.store(v, Ordering::Relaxed);
    }

    pub fn mono(&self) -> bool {
        self.mono.load(Ordering::Relaxed)
    }

    pub fn set_mono(&self, v: bool) {
        self.mono.store(v, Ordering::Relaxed);
    }

    /// Borrow the current plugin chain. The returned [`Guard`] derefs to
    /// `&Vec<PluginInstanceId>`, so call sites can `for &id in track.plugins().iter()`.
    /// Holding the guard does not block writers — `ArcSwap` snapshots
    /// the chain via a single atomic load, so a concurrent mutation
    /// just publishes a new chain that future loads will see.
    pub fn plugins(&self) -> Guard<Arc<Vec<PluginInstanceId>>> {
        self.plugin_chain.load()
    }

    /// Cheap `Arc` clone of the current plugin chain. Useful when the
    /// caller wants to hand the chain off to another scope (e.g. the
    /// engine-thread "collect plugin ids before draining" pattern) and
    /// outlive any borrow of `&self`.
    pub fn plugin_chain_snapshot(&self) -> Arc<Vec<PluginInstanceId>> {
        self.plugin_chain.load_full()
    }

    /// Append `id` to the chain. Copy-on-write: clones the current
    /// chain, pushes, and publishes the new chain. Concurrent readers
    /// keep using the pre-push chain until they reload.
    pub fn push_plugin(&self, id: PluginInstanceId) {
        let current = self.plugin_chain.load_full();
        let mut next = (*current).clone();
        next.push(id);
        self.plugin_chain.store(Arc::new(next));
    }

    /// Drop every plugin id where `pred` returns false. Copy-on-write
    /// like [`push_plugin`](Self::push_plugin).
    pub fn retain_plugins(&self, mut pred: impl FnMut(&PluginInstanceId) -> bool) {
        let current = self.plugin_chain.load_full();
        let mut next = (*current).clone();
        next.retain(|id| pred(id));
        self.plugin_chain.store(Arc::new(next));
    }

    /// Replace the chain wholesale with `ids`. Used by project-load
    /// replay and by the plugin-scan path that clears every track's
    /// chain before re-instantiating the saved instances.
    pub fn set_plugin_chain(&self, ids: Vec<PluginInstanceId>) {
        self.plugin_chain.store(Arc::new(ids));
    }

    /// Empty the chain. Convenience wrapper over
    /// [`set_plugin_chain`](Self::set_plugin_chain) for the common
    /// "wipe all FX" path.
    pub fn clear_plugins(&self) {
        self.plugin_chain.store(Arc::new(Vec::new()));
    }

    /// Atomically update peak L to the max of the current and new value.
    ///
    /// Uses `fetch_max` on bit-punned `AtomicU32`. This works because `v`
    /// is always non-negative (`.abs()` applied at call sites), and IEEE 754
    /// binary32 bit ordering matches u32 ordering for non-negative values.
    ///
    /// `AcqRel` synchronises with the engine-thread `swap` reader so the
    /// reader observes a coherent peak value rather than racing against
    /// concurrent block updates from the audio callback.
    pub fn update_peak_l(&self, v: f32) {
        self.peak_l_bits.fetch_max(v.to_bits(), Ordering::AcqRel);
    }

    /// Atomically update peak R to the max of the current and new value.
    /// See [`update_peak_l`](Self::update_peak_l) for the non-negative invariant.
    pub fn update_peak_r(&self, v: f32) {
        self.peak_r_bits.fetch_max(v.to_bits(), Ordering::AcqRel);
    }

    /// Read and clear peak L, returning the peak since last call.
    pub fn swap_peak_l(&self) -> f32 {
        f32::from_bits(self.peak_l_bits.swap(0, Ordering::AcqRel))
    }

    /// Read and clear peak R, returning the peak since last call.
    pub fn swap_peak_r(&self) -> f32 {
        f32::from_bits(self.peak_r_bits.swap(0, Ordering::AcqRel))
    }
}

/// An audio bus: an intermediate summing point with its own plugin
/// chain, fader, pan, mute, and meters. Busses live between tracks and
/// master — tracks can route their post-fader audio to a bus, the bus
/// processes the sum through its plugin chain, then the bus sums into
/// master.
#[derive(Debug)]
pub struct Bus {
    pub id: BusId,
    volume_bits: AtomicU32,
    pan_bits: AtomicU32,
    muted: AtomicBool,
    /// When true, the mixer skips every plugin in this bus's FX chain.
    fx_bypassed: AtomicBool,
    pub name: String,
    peak_l_bits: AtomicU32,
    peak_r_bits: AtomicU32,
    /// Ordered list of plugin instance IDs forming the insert chain.
    pub plugin_ids: Vec<PluginInstanceId>,
}

impl Bus {
    pub fn new(id: BusId, name: String) -> Self {
        Self {
            id,
            volume_bits: AtomicU32::new(1.0f32.to_bits()),
            pan_bits: AtomicU32::new(0.0f32.to_bits()),
            muted: AtomicBool::new(false),
            fx_bypassed: AtomicBool::new(false),
            name,
            peak_l_bits: AtomicU32::new(0),
            peak_r_bits: AtomicU32::new(0),
            plugin_ids: Vec::new(),
        }
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }

    pub fn set_volume(&self, v: f32) {
        self.volume_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn pan(&self) -> f32 {
        f32::from_bits(self.pan_bits.load(Ordering::Relaxed))
    }

    pub fn set_pan(&self, v: f32) {
        self.pan_bits.store(v.to_bits(), Ordering::Relaxed);
    }

    pub fn muted(&self) -> bool {
        self.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, v: bool) {
        self.muted.store(v, Ordering::Relaxed);
    }

    pub fn fx_bypassed(&self) -> bool {
        self.fx_bypassed.load(Ordering::Relaxed)
    }

    pub fn set_fx_bypassed(&self, v: bool) {
        self.fx_bypassed.store(v, Ordering::Relaxed);
    }

    /// See [`Track::update_peak_l`] for the non-negative invariant and the
    /// `AcqRel` ordering rationale.
    pub fn update_peak_l(&self, v: f32) {
        self.peak_l_bits.fetch_max(v.to_bits(), Ordering::AcqRel);
    }

    /// See [`Track::update_peak_l`] for the non-negative invariant.
    pub fn update_peak_r(&self, v: f32) {
        self.peak_r_bits.fetch_max(v.to_bits(), Ordering::AcqRel);
    }

    pub fn swap_peak_l(&self) -> f32 {
        f32::from_bits(self.peak_l_bits.swap(0, Ordering::AcqRel))
    }

    pub fn swap_peak_r(&self) -> f32 {
        f32::from_bits(self.peak_r_bits.swap(0, Ordering::AcqRel))
    }
}

/// The global master bus. Holds the post-bus-sum FX chain that runs
/// after every track and bus has been summed into the master output,
/// right before the master volume / clip / peak pass.
#[derive(Debug, Default)]
pub struct MasterBus {
    /// Ordered list of plugin instance IDs forming the master insert chain.
    pub plugin_ids: Vec<PluginInstanceId>,
}

impl MasterBus {
    pub fn new() -> Self {
        Self::default()
    }
}

