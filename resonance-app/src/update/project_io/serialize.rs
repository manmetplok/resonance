//! Pure serialization: build a `ProjectFile` from current GUI state.
//! No I/O, no engine commands, no state mutation — just a transformation
//! from the runtime model to the on-disk shape.

use resonance_audio::types::*;

use crate::project::{
    audio_format_tag, ProjectBus, ProjectClip, ProjectFile, ProjectMidiClip, ProjectPlugin,
    ProjectPoolAsset, ProjectReference, ProjectReferenceMarker, ProjectReferenceSettings,
    ProjectTrack, PROJECT_FORMAT_VERSION,
};
use crate::Resonance;

/// Serialize current GUI state to the on-disk `ProjectFile` shape.
pub fn build_project_file(r: &Resonance) -> ProjectFile {
    let tracks = r
        .sorted_tracks()
        .iter()
        .map(|t| ProjectTrack {
            id: t.id,
            name: t.name.clone(),
            order: t.order,
            volume: t.volume,
            pan: t.pan,
            muted: t.muted,
            soloed: t.soloed,
            fx_bypassed: t.fx_bypassed,
            record_armed: t.record_armed,
            monitor_enabled: t.monitor_enabled,
            mono: t.mono,
            input_device_name: t.input_device_name.clone(),
            plugins: t
                .plugins
                .iter()
                .map(|p| ProjectPlugin {
                    instance_id: p.instance_id,
                    plugin_name: p.plugin_name.clone(),
                    clap_plugin_id: p.clap_plugin_id.clone(),
                    clap_file_path: p.clap_file_path.clone(),
                    state_file: format!("plugins/plugin_{}.bin", p.instance_id),
                })
                .collect(),
            track_type: match t.track_type {
                TrackType::Audio => "audio".to_string(),
                TrackType::Instrument => "instrument".to_string(),
                TrackType::Vocal => "vocal".to_string(),
            },
            output_bus: match t.output {
                TrackOutput::Master => None,
                TrackOutput::Bus(id) => Some(id),
            },
            instrument_type: t.instrument_type,
            instrument_icon: t.instrument_icon,
            role: t.role,
            sub_track: t.sub_track,
            input_port_index: Some(t.input_port_index),
            midi_input_device: t.midi_input_device.clone(),
            midi_input_channel: t.midi_input_channel,
            midi_output_device: t.midi_output_device.clone(),
            midi_output_channel: t.midi_output_channel,
        })
        .collect();

    let busses = r
        .sorted_busses()
        .iter()
        .map(|b| ProjectBus {
            id: b.id,
            name: b.name.clone(),
            order: b.order,
            volume: b.volume,
            pan: b.pan,
            muted: b.muted,
            fx_bypassed: b.fx_bypassed,
            plugins: b
                .plugins
                .iter()
                .map(|p| ProjectPlugin {
                    instance_id: p.instance_id,
                    plugin_name: p.plugin_name.clone(),
                    clap_plugin_id: p.clap_plugin_id.clone(),
                    clap_file_path: p.clap_file_path.clone(),
                    state_file: format!("plugins/plugin_{}.bin", p.instance_id),
                })
                .collect(),
        })
        .collect();

    let clips = r
        .clips
        .iter()
        .map(|c| ProjectClip {
            id: c.id,
            track_id: c.track_id,
            start_sample: c.start_sample,
            name: c.name.clone(),
            total_frames: c.total_frames,
            trim_start_frames: c.trim_start_frames,
            trim_end_frames: c.trim_end_frames,
            audio_file: format!("audio/clip_{}.wav", c.id),
            // Pool-asset provenance (doc #175): persist the link so an
            // imported+placed clip reconnects to its pool asset on reload.
            asset_ref: c.asset_ref.map(|r| r.asset_id),
        })
        .collect();

    let midi_clips = r
        .midi_clips
        .iter()
        .map(|mc| {
            // Per-note lyric annotations from the side-table. Strip
            // trailing empties so a clip without slurs / overrides
            // doesn't bloat the project file with an empty-string
            // array; the replay path pads back to `notes.len()` on
            // load.
            let mut vocal_lyrics: Vec<String> = r
                .compose
                .vocal_audio
                .clip_lyrics
                .get(&mc.id)
                .cloned()
                .unwrap_or_default();
            while vocal_lyrics.last().is_some_and(|s| s.is_empty()) {
                vocal_lyrics.pop();
            }
            ProjectMidiClip {
                id: mc.id,
                track_id: mc.track_id,
                start_sample: mc.start_sample,
                duration_ticks: mc.duration_ticks,
                name: mc.name.clone(),
                trim_start_ticks: mc.trim_start_ticks,
                trim_end_ticks: mc.trim_end_ticks,
                midi_file: format!("midi/clip_{}.mid", mc.id),
                vocal_lyrics,
            }
        })
        .collect();

    let master_plugins = r
        .master_plugins
        .iter()
        .map(|p| ProjectPlugin {
            instance_id: p.instance_id,
            plugin_name: p.plugin_name.clone(),
            clap_plugin_id: p.clap_plugin_id.clone(),
            clap_file_path: p.clap_file_path.clone(),
            state_file: format!("plugins/plugin_{}.bin", p.instance_id),
        })
        .collect();

    // Reference A/B block. Persist only the durable facts (path, name,
    // cached loudness, markers); the decoded PCM / waveform are rebuilt by
    // re-issuing `LoadReferenceTrack` on load. The active reference is
    // addressed by index so a reload's reallocated engine ids don't matter.
    let references: Vec<ProjectReference> = r
        .reference
        .entries
        .iter()
        .map(|e| ProjectReference {
            path: e.path.clone(),
            name: e.name.clone(),
            integrated_lufs: e.integrated_lufs,
            markers: e
                .markers
                .iter()
                .map(|m| ProjectReferenceMarker {
                    id: m.id,
                    position_samples: m.position_samples,
                    label: m.label.clone(),
                })
                .collect(),
        })
        .collect();
    let reference_settings = ProjectReferenceSettings {
        monitor_only: true,
        active: r
            .reference
            .active_id
            .and_then(|id| r.reference.index_of(id)),
        ab_source_is_reference: r.reference.ab_source == ABSource::Reference,
        loudness_match: r.reference.loudness_match,
        trim_db: r.reference.trim_db,
        loop_to_mix: r.reference.loop_to_mix,
    };

    // Media pool (doc #175). Persist the durable facts about each
    // imported asset — its project-relative WAV path, source provenance,
    // and the project-rate duration — in import order. The waveform
    // thumbnail and live usage counts are runtime-derived and rebuilt on
    // load, so they're left out of the file.
    let pool_assets: Vec<ProjectPoolAsset> = r
        .pool
        .assets
        .iter()
        .map(|a| ProjectPoolAsset {
            id: a.id,
            project_relative_path: a.project_relative_path.clone(),
            original_path: a.original_path.clone(),
            format: audio_format_tag(a.format).to_string(),
            channels: a.channels,
            source_sample_rate: a.source_sample_rate,
            duration_frames: a.duration_frames,
        })
        .collect();

    ProjectFile {
        version: PROJECT_FORMAT_VERSION,
        sample_rate: r.sample_rate,
        bpm: r.transport.bpm,
        time_sig_num: r.transport.time_sig_num,
        time_sig_den: r.transport.time_sig_den,
        metronome_enabled: r.transport.metronome_enabled,
        master_volume: r.master_volume,
        master_plugins,
        master_fx_bypassed: r.master_fx_bypassed,
        loop_enabled: r.transport.loop_enabled,
        loop_in: r.transport.loop_in,
        loop_out: r.transport.loop_out,
        tracks,
        clips,
        midi_clips,
        busses,
        section_definitions: r.compose.to_project_definitions(),
        section_placements: r.compose.to_project_placements(),
        tempo_events: r.tempo_events.clone(),
        signature_events: r.signature_events.clone(),
        midi_clock_send_enabled: r.midi_clock_send_enabled,
        midi_clock_send_device: r.midi_clock_send_device.clone(),
        midi_clock_recv_enabled: r.midi_clock_recv_enabled,
        midi_clock_recv_device: r.midi_clock_recv_device.clone(),
        // Legacy field — current code persists the full pattern bank
        // below. Kept empty here so projects authored by this build skip
        // straight to the new shape, and the legacy loader only kicks in
        // for files written by older builds.
        drum_groups: Vec::new(),
        drum_patterns: r.compose.drum_patterns.clone(),
        references,
        reference_settings,
        arrangement_markers: r.markers.markers.clone(),
        pool_assets,
    }
}
