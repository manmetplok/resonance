//! Apply / save user track presets in response to engine events that
//! signal a track was added (apply) or a per-plugin state was captured
//! (save).

use resonance_audio::types::{AudioCommand, TrackId, TrackType};

use crate::state::TrackState;
use crate::Resonance;

/// Apply a preset's settings and plugin chain to a newly created track.
/// Called from the `TrackAdded`/`InstrumentTrackAdded` handlers when
/// `pending_track_preset` was set.
pub(super) fn apply_preset_to_track(
    r: &mut Resonance,
    track: &mut TrackState,
    preset: &crate::presets::TrackPreset,
) {
    track.name = preset.name.clone();
    track.volume = preset.volume;
    track.pan = preset.pan;
    track.mono = preset.mono;
    track.instrument_type = preset.instrument_type;
    track.instrument_icon = preset.instrument_icon;
    track.role = preset.role;

    let track_id = track.id;

    r.engine.send(AudioCommand::SetTrackVolume {
        track_id,
        volume: crate::util::db_to_gain(preset.volume),
    });
    r.engine.send(AudioCommand::SetTrackPan {
        track_id,
        pan: preset.pan,
    });
    r.engine.send(AudioCommand::SetTrackMono {
        track_id,
        mono: preset.mono,
    });

    for pp in &preset.plugins {
        r.engine.send(AudioCommand::AddPlugin {
            track_id,
            clap_file_path: pp.clap_file_path.clone(),
            clap_plugin_id: pp.clap_plugin_id.clone(),
            id_hint: None,
        });
    }

    // Plugin state loading is deferred: we don't know the instance ids
    // yet (they're assigned by the engine). The PluginAdded event will
    // fire for each plugin. We store the preset plugin states so we can
    // match them up.
    if preset.plugins.iter().any(|p| p.state.is_some()) {
        let states: Vec<Option<Vec<u8>>> =
            preset.plugins.iter().map(|p| p.state.clone()).collect();
        r.pending_preset_plugin_states = Some((track_id, states));
    }
}

/// Complete a "Save track as preset" operation. Builds a `TrackPreset`
/// from the track's current state and the freshly-captured plugin
/// state blobs, then writes it to disk.
pub(super) fn finish_preset_save(r: &mut Resonance, track_id: TrackId) {
    let track = match r.registry.tracks.iter().find(|t| t.id == track_id) {
        Some(t) => t,
        None => return,
    };

    let plugins: Vec<crate::presets::PresetPlugin> = track
        .plugins
        .iter()
        .map(|p| crate::presets::PresetPlugin {
            plugin_name: p.plugin_name.clone(),
            clap_plugin_id: p.clap_plugin_id.clone(),
            clap_file_path: p.clap_file_path.clone(),
            state: r.plugin_state_cache.get(&p.instance_id).cloned(),
        })
        .collect();

    let preset = crate::presets::TrackPreset {
        name: track.name.clone(),
        track_type: match track.track_type {
            TrackType::Audio => "audio".to_string(),
            TrackType::Instrument => "instrument".to_string(),
            TrackType::Vocal => "vocal".to_string(),
        },
        volume: track.volume,
        pan: track.pan,
        mono: track.mono,
        instrument_type: track.instrument_type,
        instrument_icon: track.instrument_icon,
        role: track.role,
        plugins,
    };

    match crate::presets::save_user_preset(&preset) {
        Ok(_) => {
            r.user_presets = crate::presets::load_user_presets();
        }
        Err(e) => {
            r.error_message = Some(format!("Save preset: {e}"));
        }
    }
}
