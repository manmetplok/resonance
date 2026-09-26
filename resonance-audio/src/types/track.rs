//! Engine-side Track and Bus, with atomic hot-path accessors for the
//! audio callback.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use arc_swap::{ArcSwap, ArcSwapOption, Guard};
use resonance_common::{DeviceParam, PlaybackSource};

use crate::bypass::BypassFade;

use super::{BusId, FrozenSource, PluginInstanceId, TrackId, TrackOutput, TrackType};

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
    /// When bypassed, the mixer skips every effect plugin on this track.
    /// Instrument plugins (the first slot on instrument tracks) still
    /// play — only the effects chain after them is bypassed. Toggling it
    /// crossfades over a few milliseconds rather than switching the path
    /// on a sample boundary (see [`BypassFade`]).
    fx_bypass: BypassFade,
    pub name: String,
    record_armed: AtomicBool,
    monitor_enabled: AtomicBool,
    /// External-instrument playback source (doc #257): `false` = `Live`
    /// (default, exactly the pre-mode behaviour), `true` = `Recorded` —
    /// recorded takes gate the MIDI-out and monitor mix over the spans they
    /// cover. Stored as an atomic bool so both the audio callback (monitor
    /// gate) and the engine control thread (outbound MIDI gate) read it
    /// lock-free; accessed through the typed
    /// [`playback_source`](Self::playback_source) /
    /// [`set_playback_source`](Self::set_playback_source) pair.
    playback_source_recorded: AtomicBool,
    /// True when this track is in external-instrument mode (doc #169): its
    /// "instrument" is outboard hardware reached over MIDI, so it has no
    /// instrument plugin to render and its audio arrives on the return
    /// input — live while monitoring, or from a recorded take. Such a
    /// track is created as [`TrackType::Instrument`] but has to take the
    /// mixer's *audio* path (clips + monitor + all-plugins-are-FX); this
    /// flag is what tells the audio callback so, since the authoritative
    /// `ExternalInstruments` map is engine-control-thread-local and not
    /// reachable from the callback. Mirrored by
    /// `set_external_instrument_in_place` / `clear_…` so it cannot drift.
    external: AtomicBool,
    /// If true, track captures a single input channel (duplicated to both L/R).
    /// If false, track captures a stereo pair.
    mono: AtomicBool,
    /// Post-fader peak level for left channel (for VU meters).
    peak_l_bits: AtomicU32,
    /// Post-fader peak level for right channel (for VU meters).
    peak_r_bits: AtomicU32,
    /// Effective stereo gains applied at the end of the previous audio
    /// block (0.0 while muted / solo-suppressed). Written only by the
    /// audio thread; the mixer ramps from these to the current block's
    /// gains per sample to avoid zipper noise on fader/pan/mute changes.
    last_gain_l_bits: AtomicU32,
    last_gain_r_bits: AtomicU32,
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
    /// Automatable device parameters of the device preset selected on this
    /// external-instrument track, keyed by [`DeviceParam::id`]. Set via
    /// `AudioCommand::SetTrackDeviceParams` when a preset is selected
    /// (architecture doc #201 §4, epic #40). A `DeviceParam` automation
    /// lane resolves its target through this map at render time (epic #40
    /// E3) to find the bound CC/NRPN + value range, so the engine never
    /// reaches back across the command/event boundary for a definition.
    ///
    /// Wrapped in `ArcSwap` like [`plugin_chain`](Self::plugin_chain) so
    /// the audio thread reads the map lock-free while the control thread
    /// swaps in a fresh one — readers see either the pre- or post-edit
    /// map, never a torn one. Empty when no device is selected.
    device_params: ArcSwap<HashMap<String, DeviceParam>>,
    /// Optional frozen source buffer for this track. When set, the mixer
    /// plays the cached audio instead of running the live synth/FX chain.
    /// Wrapped in `ArcSwapOption` so the audio thread can read without
    /// blocking on the engine control thread's edits.
    pub frozen_source: ArcSwapOption<FrozenSource>,
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
            fx_bypass: BypassFade::new(),
            name,
            record_armed: AtomicBool::new(false),
            monitor_enabled: AtomicBool::new(false),
            playback_source_recorded: AtomicBool::new(false),
            external: AtomicBool::new(false),
            mono: AtomicBool::new(true),
            peak_l_bits: AtomicU32::new(0),
            peak_r_bits: AtomicU32::new(0),
            last_gain_l_bits: AtomicU32::new(0),
            last_gain_r_bits: AtomicU32::new(0),
            output_bus_bits: AtomicU64::new(TRACK_OUTPUT_MASTER),
            input_device_name: ArcSwapOption::const_empty(),
            input_port_bits: AtomicU32::new(0),
            plugin_chain: ArcSwap::from_pointee(Vec::new()),
            sub_track_of: None,
            midi_input_device: None,
            midi_input_channel: None,
            midi_output_device: ArcSwapOption::const_empty(),
            midi_output_channel: None,
            device_params: ArcSwap::from_pointee(HashMap::new()),
            frozen_source: ArcSwapOption::const_empty(),
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
        self.fx_bypass.bypassed()
    }

    /// Ask for the chain to be bypassed (or re-engaged). The mixer
    /// crossfades to the new state over [`crate::bypass::BYPASS_FADE_MS`];
    /// nothing switches on this call.
    pub fn set_fx_bypassed(&self, v: bool) {
        self.fx_bypass.set_bypassed(v);
    }

    /// The chain-level bypass crossfade, for the render path.
    pub fn fx_bypass(&self) -> &BypassFade {
        &self.fx_bypass
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

    /// External-instrument playback source (doc #257). `Live` is the
    /// default and means exactly the pre-mode behaviour.
    pub fn playback_source(&self) -> PlaybackSource {
        if self.playback_source_recorded.load(Ordering::Relaxed) {
            PlaybackSource::Recorded
        } else {
            PlaybackSource::Live
        }
    }

    pub fn set_playback_source(&self, source: PlaybackSource) {
        self.playback_source_recorded
            .store(source == PlaybackSource::Recorded, Ordering::Relaxed);
    }

    /// True when the track is in external-instrument mode — see the
    /// [`external`](Self::external) field. Drives the mixer's per-track
    /// branch: an external track renders like an audio track even though
    /// its type is `Instrument`.
    pub fn is_external(&self) -> bool {
        self.external.load(Ordering::Relaxed)
    }

    pub fn set_external(&self, v: bool) {
        self.external.store(v, Ordering::Relaxed);
    }

    /// True when this track's sound is generated in-process by an
    /// instrument plugin, so it has no audio input of its own.
    ///
    /// This is the single rule behind two decisions that must agree:
    ///
    /// - **Playback** (`mixer::render::track_pass::render_track_source`)
    ///   runs the instrument for such a track and the clip + monitor mix
    ///   for every other one.
    /// - **Capture** (`engine::transport::begin_recording_stream`) opens an
    ///   audio recording buffer for every armed track *except* these —
    ///   arming one records a MIDI performance, not an audio signal.
    ///   Recording both filed two takes for a single loop pass and left a
    ///   junk WAV per pass behind (ba doc #292).
    ///
    /// Note the two exclusions this deliberately does **not** make.
    /// External-instrument tracks (epic #39) are `Instrument`-typed but
    /// their synth is outboard, so their audio genuinely arrives on the
    /// return input and must still be recorded. `Vocal` tracks accept MIDI
    /// yet render through the audio path, so they keep audio capture too.
    /// Testing `track_type.accepts_midi()` instead of this would break both.
    pub fn runs_internal_instrument(&self) -> bool {
        self.track_type == TrackType::Instrument && !self.is_external()
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
    ///
    /// Returns the chain it replaced. The engine hands that to its retire
    /// queue rather than dropping it, so a callback block still reading
    /// the old chain is never its last owner (code review MIX-04); a
    /// caller with no such concern can simply drop it.
    pub fn push_plugin(&self, id: PluginInstanceId) -> Arc<Vec<PluginInstanceId>> {
        let current = self.plugin_chain.load_full();
        let mut next = (*current).clone();
        next.push(id);
        drop(current);
        self.plugin_chain.swap(Arc::new(next))
    }

    /// Drop every plugin id where `pred` returns false. Copy-on-write
    /// like [`push_plugin`](Self::push_plugin), returning the replaced
    /// chain the same way.
    pub fn retain_plugins(
        &self,
        mut pred: impl FnMut(&PluginInstanceId) -> bool,
    ) -> Arc<Vec<PluginInstanceId>> {
        let current = self.plugin_chain.load_full();
        let mut next = (*current).clone();
        next.retain(|id| pred(id));
        drop(current);
        self.plugin_chain.swap(Arc::new(next))
    }

    /// Move `instance_id` to `to_index`, shifting the plugins between its
    /// old and new slot by one. `to_index` is clamped to the last slot.
    /// Returns the index the plugin ended up at, or `None` if it is not on
    /// this chain — in which case nothing is published at all.
    ///
    /// Copy-on-write like [`push_plugin`](Self::push_plugin): the reordered
    /// `Vec` is built by the caller's thread (the engine control thread,
    /// never the audio callback) and published with a single
    /// `ArcSwap::store`, so a concurrent [`plugins`](Self::plugins) load
    /// sees either the old order or the new one, never a half-rotated
    /// chain. A no-op move publishes nothing, so it cannot even cost
    /// readers a reload.
    pub fn move_plugin(&self, instance_id: PluginInstanceId, to_index: usize) -> Option<usize> {
        self.move_plugin_into(instance_id, to_index, drop)
    }

    /// [`move_plugin`](Self::move_plugin) that hands the replaced chain to
    /// `retire` instead of dropping it — the engine passes its retire
    /// queue (code review MIX-04). `retire` is not called for a no-op
    /// move, which publishes nothing.
    pub fn move_plugin_into(
        &self,
        instance_id: PluginInstanceId,
        to_index: usize,
        retire: impl FnOnce(Arc<Vec<PluginInstanceId>>),
    ) -> Option<usize> {
        let current = self.plugin_chain.load_full();
        let from = current.iter().position(|&id| id == instance_id)?;
        // `from` was found, so the chain is non-empty and this cannot wrap.
        let to = to_index.min(current.len() - 1);
        if from == to {
            return Some(to);
        }
        let mut next = (*current).clone();
        let id = next.remove(from);
        next.insert(to, id);
        drop(current);
        retire(self.plugin_chain.swap(Arc::new(next)));
        Some(to)
    }

    /// Replace the chain wholesale with `ids`. Used by project-load
    /// replay and by the plugin-scan path that clears every track's
    /// chain before re-instantiating the saved instances. Returns the
    /// replaced chain like [`push_plugin`](Self::push_plugin).
    pub fn set_plugin_chain(&self, ids: Vec<PluginInstanceId>) -> Arc<Vec<PluginInstanceId>> {
        self.plugin_chain.swap(Arc::new(ids))
    }

    /// Empty the chain. Convenience wrapper over
    /// [`set_plugin_chain`](Self::set_plugin_chain) for the common
    /// "wipe all FX" path.
    pub fn clear_plugins(&self) -> Arc<Vec<PluginInstanceId>> {
        self.set_plugin_chain(Vec::new())
    }

    /// Borrow this track's device-parameter map (keyed by
    /// [`DeviceParam::id`]). The returned [`Guard`] derefs to
    /// `&HashMap<String, DeviceParam>`. Like [`plugins`](Self::plugins)
    /// the `ArcSwap` snapshot is a single atomic load, so holding the
    /// guard never blocks a concurrent [`set_device_params`](Self::set_device_params).
    pub fn device_params(&self) -> Guard<Arc<HashMap<String, DeviceParam>>> {
        self.device_params.load()
    }

    /// Look up one device param by id. Cheap clone of the stored
    /// [`DeviceParam`] (or `None`), so the caller can drop the map guard
    /// before using it — handy on the render path.
    pub fn device_param(&self, param_id: &str) -> Option<DeviceParam> {
        self.device_params.load().get(param_id).cloned()
    }

    /// Replace the whole device-parameter map with `params`, keying each
    /// by its `id` (last-wins on a duplicate id). Returns the stored param
    /// ids in the order they were supplied, deduplicated — the order the
    /// confirming `AudioEvent::TrackDeviceParamsApplied` reports. An empty
    /// `params` clears the map. Copy-on-write publish via a single atomic
    /// store, mirroring [`set_plugin_chain`](Self::set_plugin_chain).
    pub fn set_device_params(&self, params: Vec<DeviceParam>) -> Vec<String> {
        let mut map = HashMap::with_capacity(params.len());
        let mut order = Vec::with_capacity(params.len());
        for p in params {
            if !map.contains_key(&p.id) {
                order.push(p.id.clone());
            }
            map.insert(p.id.clone(), p);
        }
        self.device_params.store(Arc::new(map));
        order
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

    /// Effective stereo gains at the end of the previous audio block.
    pub fn last_gains(&self) -> (f32, f32) {
        (
            f32::from_bits(self.last_gain_l_bits.load(Ordering::Relaxed)),
            f32::from_bits(self.last_gain_r_bits.load(Ordering::Relaxed)),
        )
    }

    /// Record the effective stereo gains this block ended on. Audio
    /// thread only.
    pub fn set_last_gains(&self, l: f32, r: f32) {
        self.last_gain_l_bits.store(l.to_bits(), Ordering::Relaxed);
        self.last_gain_r_bits.store(r.to_bits(), Ordering::Relaxed);
    }
}

/// Whether any top-level track is soloed. Sub-tracks follow their
/// parent's solo state, so they're excluded from the scan. Shared by
/// the live mixer and the bounce renderer so solo semantics match.
pub fn any_top_level_solo<'a>(tracks: impl IntoIterator<Item = &'a Track>) -> bool {
    tracks
        .into_iter()
        .filter(|t| t.sub_track_of.is_none())
        .any(|t| t.soloed())
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
    /// When bypassed, the mixer skips every plugin in this bus's FX
    /// chain, crossfading over the transition (see [`BypassFade`]).
    fx_bypass: BypassFade,
    /// When true, this bus acts as an aux *return* bus — the destination
    /// of aux sends rather than (or in addition to) a track-output
    /// group. Purely a role marker today; it does not change summing.
    is_return: AtomicBool,
    pub name: String,
    peak_l_bits: AtomicU32,
    peak_r_bits: AtomicU32,
    /// See [`Track::last_gains`]: previous block's effective stereo
    /// gains, used by the mixer's per-sample gain ramp.
    last_gain_l_bits: AtomicU32,
    last_gain_r_bits: AtomicU32,
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
            fx_bypass: BypassFade::new(),
            is_return: AtomicBool::new(false),
            name,
            peak_l_bits: AtomicU32::new(0),
            peak_r_bits: AtomicU32::new(0),
            last_gain_l_bits: AtomicU32::new(0),
            last_gain_r_bits: AtomicU32::new(0),
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
        self.fx_bypass.bypassed()
    }

    /// Ask for the chain to be bypassed (or re-engaged). The mixer
    /// crossfades to the new state over [`crate::bypass::BYPASS_FADE_MS`];
    /// nothing switches on this call.
    pub fn set_fx_bypassed(&self, v: bool) {
        self.fx_bypass.set_bypassed(v);
    }

    /// The chain-level bypass crossfade, for the render path.
    pub fn fx_bypass(&self) -> &BypassFade {
        &self.fx_bypass
    }

    /// Reorder this bus's insert chain: move `instance_id` to
    /// `to_index`, shifting everything between its old and new slot by
    /// one. Returns the slot it actually landed on after clamping, or
    /// `None` when that instance is not on this chain (ba doc #273, todo
    /// #1237).
    ///
    /// The bus twin of [`Track::move_plugin`], but a plain `Vec` edit
    /// rather than a copy-on-write `ArcSwap` publish: `plugin_ids` is
    /// owned data behind the engine's busses write lock, not a lock-free
    /// snapshot the audio callback loads. Moving a plugin to the slot it
    /// already occupies leaves the chain untouched and still reports
    /// that slot.
    pub fn move_plugin(&mut self, instance_id: PluginInstanceId, to_index: usize) -> Option<usize> {
        let from = self.plugin_ids.iter().position(|&id| id == instance_id)?;
        // `from` was found, so the chain is non-empty and this cannot
        // wrap.
        let to = to_index.min(self.plugin_ids.len() - 1);
        if from != to {
            let id = self.plugin_ids.remove(from);
            self.plugin_ids.insert(to, id);
        }
        Some(to)
    }

    /// Whether this bus is flagged as an aux return bus.
    pub fn is_return(&self) -> bool {
        self.is_return.load(Ordering::Relaxed)
    }

    pub fn set_is_return(&self, v: bool) {
        self.is_return.store(v, Ordering::Relaxed);
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

    /// See [`Track::last_gains`].
    pub fn last_gains(&self) -> (f32, f32) {
        (
            f32::from_bits(self.last_gain_l_bits.load(Ordering::Relaxed)),
            f32::from_bits(self.last_gain_r_bits.load(Ordering::Relaxed)),
        )
    }

    /// See [`Track::set_last_gains`].
    pub fn set_last_gains(&self, l: f32, r: f32) {
        self.last_gain_l_bits.store(l.to_bits(), Ordering::Relaxed);
        self.last_gain_r_bits.store(r.to_bits(), Ordering::Relaxed);
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

    /// Move `instance_id` to `to_index`, shifting the plugins between
    /// its old and new slot by one. Returns the slot it actually landed
    /// on (`to_index` clamped to the last slot), or `None` when that
    /// plugin is not on the master chain. The master twin of
    /// [`Bus::move_plugin`].
    pub fn move_plugin(&mut self, instance_id: PluginInstanceId, to_index: usize) -> Option<usize> {
        let from = self.plugin_ids.iter().position(|&id| id == instance_id)?;
        // `from` was found, so the chain is non-empty and this cannot
        // wrap.
        let to = to_index.min(self.plugin_ids.len() - 1);
        if from != to {
            let id = self.plugin_ids.remove(from);
            self.plugin_ids.insert(to, id);
        }
        Some(to)
    }
}
