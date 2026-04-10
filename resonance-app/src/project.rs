/// Project save/load for the Resonance application.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use resonance_audio::types::{ClipId, PluginInstanceId};

pub mod sections;
pub use sections::{ProjectSectionChord, ProjectSectionDefinition, ProjectSectionPlacement};

/// On-disk project format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectFile {
    pub version: u32,
    pub sample_rate: u32,
    pub bpm: f32,
    pub time_sig_num: u8,
    pub time_sig_den: u8,
    pub metronome_enabled: bool,
    pub master_volume: f32,
    pub punch_enabled: bool,
    pub punch_in: u64,
    pub punch_out: u64,
    pub tracks: Vec<ProjectTrack>,
    pub clips: Vec<ProjectClip>,
    #[serde(default)]
    pub midi_clips: Vec<ProjectMidiClip>,
    #[serde(default)]
    pub busses: Vec<ProjectBus>,
    #[serde(default)]
    pub section_definitions: Vec<ProjectSectionDefinition>,
    #[serde(default)]
    pub section_placements: Vec<ProjectSectionPlacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectTrack {
    pub id: u64,
    pub name: String,
    pub order: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub soloed: bool,
    pub record_armed: bool,
    pub monitor_enabled: bool,
    pub mono: bool,
    pub input_device_name: Option<String>,
    pub plugins: Vec<ProjectPlugin>,
    #[serde(default = "default_track_type")]
    pub track_type: String,
    /// If Some, the track routes to this bus id. None (default on old
    /// projects) means the track routes directly to master.
    #[serde(default)]
    pub output_bus: Option<u64>,
    /// Instrument sub-type (synth/drum) for display in Compose. Default for
    /// legacy projects is `Synth`.
    #[serde(default)]
    pub instrument_type: crate::state::InstrumentType,
    /// Display icon for the instrument. Default for legacy projects is the
    /// icon matching `instrument_type`.
    #[serde(default)]
    pub instrument_icon: crate::state::InstrumentIcon,
    /// When set, this track is a sub-track driven by a non-main output
    /// port of `parent_track_id`'s instrument plugin. Legacy projects
    /// load with `None` (no sub-tracks existed before this feature).
    #[serde(default)]
    pub sub_track: Option<crate::state::SubTrackLink>,
}

/// On-disk bus state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectBus {
    pub id: u64,
    pub name: String,
    pub order: usize,
    pub volume: f32,
    pub pan: f32,
    pub muted: bool,
    pub plugins: Vec<ProjectPlugin>,
}

fn default_track_type() -> String {
    "audio".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPlugin {
    pub instance_id: u64,
    pub plugin_name: String,
    pub clap_plugin_id: String,
    pub clap_file_path: String,
    pub state_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectClip {
    pub id: u64,
    pub track_id: u64,
    pub start_sample: u64,
    pub name: String,
    pub total_frames: u64,
    pub trim_start_frames: u64,
    pub trim_end_frames: u64,
    pub audio_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMidiClip {
    pub id: u64,
    pub track_id: u64,
    pub start_sample: u64,
    pub duration_ticks: u64,
    pub name: String,
    pub trim_start_ticks: u64,
    pub trim_end_ticks: u64,
    pub notes: Vec<ProjectMidiNote>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMidiNote {
    pub note: u8,
    pub velocity: f32,
    pub start_tick: u64,
    pub duration_ticks: u64,
}

/// Everything needed to reconstruct a project after loading from disk.
#[derive(Debug, Clone)]
pub struct LoadedProject {
    pub file: ProjectFile,
    pub audio_data: HashMap<ClipId, Vec<f32>>,
    pub plugin_states: HashMap<PluginInstanceId, Vec<u8>>,
}

/// State accumulated during an async save operation.
pub struct SaveCollector {
    pub path: PathBuf,
    pub clip_data: HashMap<ClipId, Vec<f32>>,
    pub plugin_states: Vec<(PluginInstanceId, Vec<u8>)>,
    pub clips_done: bool,
    pub plugins_done: bool,
}

/// Write a project to disk.
pub fn save_project(
    path: &Path,
    project: &ProjectFile,
    clip_data: &HashMap<ClipId, Vec<f32>>,
    plugin_states: &[(PluginInstanceId, Vec<u8>)],
) -> Result<(), String> {
    let audio_dir = path.join("audio");
    let plugins_dir = path.join("plugins");

    std::fs::create_dir_all(&audio_dir).map_err(|e| format!("Create audio dir: {e}"))?;
    std::fs::create_dir_all(&plugins_dir).map_err(|e| format!("Create plugins dir: {e}"))?;

    // Write audio clip data as raw f32 little-endian bytes
    for clip in &project.clips {
        if let Some(data) = clip_data.get(&clip.id) {
            let bytes: &[u8] = bytemuck::cast_slice(data);
            let file_path = path.join(&clip.audio_file);
            std::fs::write(&file_path, bytes)
                .map_err(|e| format!("Write {}: {e}", clip.audio_file))?;
        }
    }

    // Write plugin state blobs
    for (instance_id, data) in plugin_states {
        let file_name = format!("plugin_{instance_id}.bin");
        let file_path = plugins_dir.join(&file_name);
        std::fs::write(&file_path, data)
            .map_err(|e| format!("Write {file_name}: {e}"))?;
    }

    // Write project.json
    let json = serde_json::to_string_pretty(project)
        .map_err(|e| format!("Serialize project: {e}"))?;
    std::fs::write(path.join("project.json"), json)
        .map_err(|e| format!("Write project.json: {e}"))?;

    Ok(())
}

/// Read a project from disk.
pub fn load_project(path: &Path) -> Result<LoadedProject, String> {
    let json_path = if path.join("project.json").exists() {
        path.join("project.json")
    } else if path.file_name().map(|f| f == "project.json").unwrap_or(false) {
        path.to_path_buf()
    } else {
        return Err("No project.json found".to_string());
    };

    let project_dir = json_path.parent().unwrap_or(path);

    let json = std::fs::read_to_string(&json_path)
        .map_err(|e| format!("Read project.json: {e}"))?;
    let file: ProjectFile =
        serde_json::from_str(&json).map_err(|e| format!("Parse project.json: {e}"))?;

    if file.version != 1 {
        return Err(format!("Unsupported project version: {}", file.version));
    }

    // Read audio clip data
    let mut audio_data = HashMap::new();
    for clip in &file.clips {
        let clip_path = project_dir.join(&clip.audio_file);
        match std::fs::read(&clip_path) {
            Ok(bytes) => {
                if bytes.len() % 4 != 0 {
                    return Err(format!("Invalid audio file size: {}", clip.audio_file));
                }
                let data: Vec<f32> = bytemuck::cast_slice(&bytes).to_vec();
                audio_data.insert(clip.id, data);
            }
            Err(e) => {
                eprintln!("Warning: could not load audio file {}: {e}", clip.audio_file);
            }
        }
    }

    // Read plugin state blobs
    let mut plugin_states = HashMap::new();
    for track in &file.tracks {
        for plugin in &track.plugins {
            let state_path = project_dir.join(&plugin.state_file);
            match std::fs::read(&state_path) {
                Ok(data) => {
                    plugin_states.insert(plugin.instance_id, data);
                }
                Err(e) => {
                    eprintln!(
                        "Warning: could not load plugin state {}: {e}",
                        plugin.state_file
                    );
                }
            }
        }
    }

    Ok(LoadedProject {
        file,
        audio_data,
        plugin_states,
    })
}
