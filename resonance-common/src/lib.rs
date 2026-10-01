/// Shared utilities for Resonance plugins.
///
/// The DAW-model modules (`model` feature, on by default) and the WAV/audio
/// decode modules (`decode` feature, on by default) are feature-gated so a
/// plugin can build with `default-features = false` and not be able to name
/// a model type at all (ARCH-07 A7-3) — see `Cargo.toml`.
pub mod atomic_file;
#[cfg(feature = "decode")]
pub mod audio_probe;
#[cfg(feature = "model")]
pub mod automation;
#[cfg(feature = "model")]
pub mod automation_shape;
#[cfg(feature = "model")]
pub mod device_definition;
#[cfg(feature = "model")]
pub mod device_registry;
#[cfg(feature = "model")]
pub mod external_instrument;
pub mod factory_presets;
pub mod preset_session;
pub mod drum_map;
#[cfg(feature = "model")]
pub mod group_identity;
pub mod library_marks;
pub mod nam_library;
#[cfg(feature = "model")]
pub mod midi_map;
#[cfg(feature = "model")]
pub mod freeze;
pub mod registry;
pub mod resample;
pub mod reveal;
mod scan;
#[cfg(feature = "model")]
pub mod take;
#[cfg(feature = "model")]
pub mod track_group;
#[cfg(feature = "decode")]
mod wav;

pub use atomic_file::{atomic_write, quarantine_corrupt, AtomicWriteError};
#[cfg(feature = "model")]
pub use automation::{
    lane_value_to_plugin_param, lane_value_to_real, plugin_param_to_lane_value,
    real_to_lane_value, sample_lane, AutomationLane, AutomationTarget, Breakpoint, BusId,
    CurveKind, LaneId, PluginInstanceId, TrackId, GAIN_MAX_DB, GAIN_MIN_DB,
};
#[cfg(feature = "model")]
pub use automation_shape::{
    generate_shape, replace_range, BarSpan, GeneratedPoint, ShapeError, ShapeKind, ShapeOutput,
    ShapeRequest, StepQuantizer, MAX_SHAPE_POINTS_PER_CALL,
};
// `device_definition::MidiBinding` (a device parameter's CC/NRPN address) is a
// distinct concept from `midi_map::MidiBinding` (a MIDI-Learn control mapping);
// it stays reachable as `device_definition::MidiBinding` to avoid the name clash.
#[cfg(feature = "model")]
pub use device_definition::{
    binding_value_to_lane, lane_value_to_binding_value, DeviceDefinition, DeviceDefinitionError,
    DeviceJsonError, DeviceParam, ParamCurve, PatchEntry, SCHEMA_VERSION,
};
#[cfg(feature = "model")]
pub use device_registry::{
    bundled_definitions, user_definitions_dir, DeviceDefinitionRegistry, DeviceLoadError,
    DeviceSaveError, DeviceScanError, DEVICE_DEFINITION_EXT,
};
#[cfg(feature = "model")]
pub use external_instrument::{ExternalInstrument, PlaybackSource};
#[cfg(feature = "model")]
pub use midi_map::{
    apply_delta, cc_to_norm, decode_relative, delete_controller_map, load_controller_maps,
    save_controller_map, takeover_value, BindingId, CcMode, ControlSource, ControllerMap,
    ControllerMapStore, MidiBinding, MidiMapError, MidiTarget, RelativeEnc, SendId, Takeover,
    TransportAction,
};
#[cfg(feature = "decode")]
pub use audio_probe::{
    probe_audio_file, scan_audio_folder, waveform_thumbnail, AudioFileEntry, AudioFormat,
    AudioInfo, AudioProbeError, WaveformThumbnail,
};
#[cfg(feature = "model")]
pub use group_identity::{GroupColor, GroupIdentityColor};
pub use scan::scan_directory;
#[cfg(feature = "model")]
pub use take::{
    effective_cover, latest_take, ClipId, Comp, CompSegment, CoverSource, CoverSpan, SlotCover,
    Take, TakeContent, TakeGroup, TakeGroupId, TakeId, TakeNote, TimelineRange,
};
#[cfg(feature = "model")]
pub use track_group::{MACRO_LEVEL_UNITY, TrackGroup};
#[cfg(feature = "decode")]
pub use wav::{
    decode_file, decode_wav_channels, decode_wav_stereo, linear_resample_mono,
    linear_resample_stereo, StreamingLinearResampler, WavChannels, WavDecodeError,
};
#[cfg(feature = "model")]
pub use freeze::{
    compute_fingerprint, FreezeCacheRef, FreezeCacheStatus, FreezeFingerprintBuilder,
    FreezeFingerprintInputs, TrackFreezeState,
};
