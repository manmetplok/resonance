//! Track presets for the Resonance application.
//!
//! A track preset captures the configuration of a track — its type, mixer
//! settings, and optionally its plugin chain with plugin state blobs —
//! so the user can stamp out new tracks from a template.
//!
//! **Default presets** are baked into the binary and provide common
//! starting points (bass guitar, rhythm guitar, vocals, etc.) without
//! any plugin chain.
//!
//! **User presets** are saved to `~/.local/share/resonance/track-presets/`
//! and can include the full plugin chain with serialized plugin state.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::state::{InstrumentIcon, InstrumentType, TrackRole};

/// On-disk track preset format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackPreset {
    pub name: String,
    /// `"audio"` or `"instrument"`.
    pub track_type: String,
    pub volume: f32,
    pub pan: f32,
    pub mono: bool,
    #[serde(default)]
    pub instrument_type: InstrumentType,
    #[serde(default)]
    pub instrument_icon: InstrumentIcon,
    #[serde(default)]
    pub role: Option<TrackRole>,
    #[serde(default)]
    pub plugins: Vec<PresetPlugin>,
}

/// A plugin slot inside a track preset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetPlugin {
    pub plugin_name: String,
    pub clap_plugin_id: String,
    pub clap_file_path: String,
    /// Opaque CLAP state blob. Stored as a JSON array of bytes.
    #[serde(default)]
    pub state: Option<Vec<u8>>,
}

// ---- Default (built-in) presets ------------------------------------------

pub fn default_presets() -> Vec<TrackPreset> {
    vec![
        TrackPreset {
            name: "Bass Guitar".into(),
            track_type: "instrument".into(),
            volume: 0.0,
            pan: 0.0,
            mono: true,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Guitar,
            role: None,
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Rhythm Guitar".into(),
            track_type: "instrument".into(),
            volume: 0.0,
            pan: 0.0,
            mono: true,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Guitar,
            role: None,
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Solo Guitar".into(),
            track_type: "instrument".into(),
            volume: 0.0,
            pan: 0.0,
            mono: true,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Guitar,
            role: None,
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Acoustic Guitar".into(),
            track_type: "instrument".into(),
            volume: 0.0,
            pan: 0.0,
            mono: true,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Guitar,
            role: None,
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Vocal".into(),
            track_type: "vocal".into(),
            volume: 0.0,
            pan: 0.0,
            mono: true,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Microphone,
            role: None,
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Backing Vocal".into(),
            track_type: "vocal".into(),
            volume: 0.0,
            pan: 0.0,
            mono: true,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Microphone,
            role: None,
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Drums".into(),
            track_type: "instrument".into(),
            volume: 0.0,
            pan: 0.0,
            mono: false,
            instrument_type: InstrumentType::Drum,
            instrument_icon: InstrumentIcon::Drum,
            role: None,
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Synth".into(),
            track_type: "instrument".into(),
            volume: 0.0,
            pan: 0.0,
            mono: false,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Music,
            role: None,
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Synth Bass".into(),
            track_type: "instrument".into(),
            volume: 0.0,
            pan: 0.0,
            mono: false,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::WaveSquare,
            role: Some(TrackRole::Bass),
            plugins: Vec::new(),
        },
        TrackPreset {
            name: "Synth Pad".into(),
            track_type: "instrument".into(),
            volume: 0.0,
            pan: 0.0,
            mono: false,
            instrument_type: InstrumentType::Synth,
            instrument_icon: InstrumentIcon::Music,
            role: Some(TrackRole::Pad),
            plugins: Vec::new(),
        },
    ]
}

// ---- User preset persistence ---------------------------------------------

/// Directory for user-saved track presets.
fn presets_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("resonance/track-presets"))
}

/// Load all user presets from disk.
pub fn load_user_presets() -> Vec<TrackPreset> {
    let dir = match presets_dir() {
        Some(d) => d,
        None => return Vec::new(),
    };
    if !dir.exists() {
        return Vec::new();
    }
    let mut presets = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                match load_preset_file(&path) {
                    Ok(preset) => presets.push(preset),
                    Err(e) => {
                        eprintln!("Warning: could not load preset {}: {e}", path.display())
                    }
                }
            }
        }
    }
    presets.sort_by(|a, b| a.name.cmp(&b.name));
    presets
}

fn load_preset_file(path: &Path) -> Result<TrackPreset, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("Read: {e}"))?;
    serde_json::from_str(&json).map_err(|e| format!("Parse: {e}"))
}

/// Save a user preset to disk.
pub fn save_user_preset(preset: &TrackPreset) -> Result<PathBuf, String> {
    let dir = presets_dir().ok_or_else(|| "Could not determine data directory".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Create presets dir: {e}"))?;

    let file_name = sanitize_filename(&preset.name);
    let path = dir.join(format!("{file_name}.json"));
    let json =
        serde_json::to_string_pretty(preset).map_err(|e| format!("Serialize preset: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("Write preset: {e}"))?;
    Ok(path)
}

/// Delete a user preset from disk.
pub fn delete_user_preset(name: &str) -> Result<(), String> {
    let dir = presets_dir().ok_or_else(|| "Could not determine data directory".to_string())?;
    let file_name = sanitize_filename(name);
    let path = dir.join(format!("{file_name}.json"));
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Delete preset: {e}"))?;
    }
    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
