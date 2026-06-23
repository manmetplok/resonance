/// Shared utilities for Resonance plugins.
pub mod audio_probe;
pub mod automation;
mod denormal;
pub mod drum_map;
pub mod group_identity;
pub mod track_group;
pub mod midi_map;
pub mod registry;
mod scan;
mod wav;

pub use automation::{
    lane_value_to_plugin_param, lane_value_to_real, plugin_param_to_lane_value,
    real_to_lane_value, sample_lane, AutomationLane, AutomationTarget, Breakpoint, BusId,
    CurveKind, LaneId, PluginInstanceId, TrackId,
};
pub use midi_map::{
    apply_delta, cc_to_norm, decode_relative, delete_controller_map, load_controller_maps,
    save_controller_map, takeover_value, BindingId, CcMode, ControlSource, ControllerMap,
    ControllerMapStore, MidiBinding, MidiTarget, RelativeEnc, SendId, Takeover, TransportAction,
};
pub use audio_probe::{
    probe_audio_file, scan_audio_folder, waveform_thumbnail, AudioFileEntry, AudioFormat,
    AudioInfo, WaveformThumbnail,
};
pub use denormal::flush_denormals;
pub use group_identity::{GroupColor, GroupIdentityColor};
pub use track_group::{GroupId, TrackGroup};
pub use scan::scan_directory;
pub use wav::{
    decode_file, decode_wav_channels, decode_wav_stereo, linear_resample_mono,
    linear_resample_stereo, StreamingLinearResampler, WavChannels,
};
