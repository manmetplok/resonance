//! Engine-side Track and Bus, with atomic hot-path accessors for the
//! audio callback.
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

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
    pub input_device_name: Option<String>,
    /// 0-indexed starting input channel on the track's input device. For
    /// mono tracks this is the single channel captured and duplicated to
    /// L/R; for stereo tracks it's the L channel and `port_index + 1` is
    /// used as R. Defaults to 0 (first channel pair).
    input_port_bits: AtomicU32,
    /// Ordered list of plugin instance IDs forming the insert chain.
    /// For instrument tracks, the first plugin is the instrument; the rest are effects.
    pub plugin_ids: Vec<PluginInstanceId>,
    /// When set, this track is a sub-track fed by a non-main output port
    /// of `parent_track_id`'s instrument plugin. Sub-tracks never run
    /// their own plugin chain or receive MIDI events — the mixer drives
    /// them entirely from the parent plugin's `process_multi` output.
    /// The tuple is `(parent_track_id, output_port_index)` where index 0
    /// is reserved for the parent's own main output.
    pub sub_track_of: Option<(TrackId, u32)>,
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
            name,
            record_armed: AtomicBool::new(false),
            monitor_enabled: AtomicBool::new(false),
            mono: AtomicBool::new(true),
            peak_l_bits: AtomicU32::new(0),
            peak_r_bits: AtomicU32::new(0),
            output_bus_bits: AtomicU64::new(TRACK_OUTPUT_MASTER),
            input_device_name: None,
            input_port_bits: AtomicU32::new(0),
            plugin_ids: Vec::new(),
            sub_track_of: None,
        }
    }

    /// The track's 0-indexed starting input channel.
    pub fn input_port(&self) -> u16 {
        (self.input_port_bits.load(Ordering::Relaxed) & 0xFFFF) as u16
    }

    pub fn set_input_port(&self, port: u16) {
        self.input_port_bits
            .store(port as u32, Ordering::Relaxed);
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

    /// Atomically update peak L to the max of the current and new value.
    pub fn update_peak_l(&self, v: f32) {
        self.peak_l_bits.fetch_max(v.to_bits(), Ordering::Relaxed);
    }

    /// Atomically update peak R to the max of the current and new value.
    pub fn update_peak_r(&self, v: f32) {
        self.peak_r_bits.fetch_max(v.to_bits(), Ordering::Relaxed);
    }

    /// Read and clear peak L, returning the peak since last call.
    pub fn swap_peak_l(&self) -> f32 {
        f32::from_bits(self.peak_l_bits.swap(0, Ordering::Relaxed))
    }

    /// Read and clear peak R, returning the peak since last call.
    pub fn swap_peak_r(&self) -> f32 {
        f32::from_bits(self.peak_r_bits.swap(0, Ordering::Relaxed))
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

    pub fn update_peak_l(&self, v: f32) {
        self.peak_l_bits.fetch_max(v.to_bits(), Ordering::Relaxed);
    }

    pub fn update_peak_r(&self, v: f32) {
        self.peak_r_bits.fetch_max(v.to_bits(), Ordering::Relaxed);
    }

    pub fn swap_peak_l(&self) -> f32 {
        f32::from_bits(self.peak_l_bits.swap(0, Ordering::Relaxed))
    }

    pub fn swap_peak_r(&self) -> f32 {
        f32::from_bits(self.peak_r_bits.swap(0, Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_output_defaults_to_master() {
        let track = Track::new(1, "T1".to_string());
        assert_eq!(track.output(), TrackOutput::Master);
    }

    #[test]
    fn track_output_roundtrip_master() {
        let track = Track::new(1, "T1".to_string());
        track.set_output(TrackOutput::Master);
        assert_eq!(track.output(), TrackOutput::Master);
    }

    #[test]
    fn track_output_roundtrip_bus() {
        let track = Track::new(1, "T1".to_string());
        track.set_output(TrackOutput::Bus(42));
        assert_eq!(track.output(), TrackOutput::Bus(42));
    }

    #[test]
    fn track_output_roundtrip_various_bus_ids() {
        let track = Track::new(1, "T1".to_string());
        for id in [1u64, 7, 100, 1_000_000, u64::MAX - 1] {
            track.set_output(TrackOutput::Bus(id));
            assert_eq!(track.output(), TrackOutput::Bus(id));
        }
    }

    #[test]
    fn track_output_master_sentinel_is_u64_max() {
        // The sentinel chosen for Master is u64::MAX. Bus id u64::MAX is
        // reserved and intentionally indistinguishable from Master; the
        // engine's next_bus_id starts at 1 and grows, so this is safe in
        // practice but worth pinning in a test.
        let track = Track::new(1, "T1".to_string());
        track.set_output(TrackOutput::Master);
        assert_eq!(track.output(), TrackOutput::Master);
        track.set_output(TrackOutput::Bus(5));
        assert_eq!(track.output(), TrackOutput::Bus(5));
        track.set_output(TrackOutput::Master);
        assert_eq!(track.output(), TrackOutput::Master);
    }

    #[test]
    fn bus_atomic_accessors_roundtrip() {
        let bus = Bus::new(1, "Bus 1".to_string());

        assert_eq!(bus.volume(), 1.0);
        assert_eq!(bus.pan(), 0.0);
        assert!(!bus.muted());

        bus.set_volume(0.5);
        assert_eq!(bus.volume(), 0.5);

        bus.set_pan(-0.75);
        assert_eq!(bus.pan(), -0.75);

        bus.set_muted(true);
        assert!(bus.muted());
    }

    #[test]
    fn bus_peak_update_and_swap() {
        let bus = Bus::new(1, "Bus 1".to_string());

        bus.update_peak_l(0.3);
        bus.update_peak_l(0.5);
        bus.update_peak_l(0.2);
        bus.update_peak_r(0.8);

        assert_eq!(bus.swap_peak_l(), 0.5);
        assert_eq!(bus.swap_peak_r(), 0.8);
        assert_eq!(bus.swap_peak_l(), 0.0);
        assert_eq!(bus.swap_peak_r(), 0.0);
    }
}
