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
use resonance_common::{atomic_write, quarantine_corrupt};
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
/// Where user presets live.
///
/// `RESONANCE_PRESET_DIR` overrides the default, which is what lets a
/// test save and re-read a preset without writing into the machine's
/// real preset folder (ba todo #1303) — the same escape hatch
/// `RESONANCE_SVS_MODELS_DIR` gives the voicebank loader.
fn presets_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(PRESET_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    crate::user_dirs::data_dir().map(|d| d.join("resonance/track-presets"))
}

/// Environment override for [`presets_dir`].
pub const PRESET_DIR_ENV: &str = "RESONANCE_PRESET_DIR";

/// Whether a user preset by this name already exists on disk.
///
/// Saving is a file write that replaces whatever is there, so both
/// surfaces ask first: the GUI turns its button into "Overwrite" and
/// `track.save_preset` refuses without `overwrite: true` (the control
/// API's destructive-operation convention). Matched on the name stored
/// inside the file, so a preset saved under an older filename scheme is
/// still found (code review STATE-15).
pub fn user_preset_exists(name: &str) -> bool {
    let Some(dir) = presets_dir() else {
        return false;
    };
    !files_named(&dir, name).is_empty()
}

/// The preset files in `dir` whose stored name is `name`: the one at
/// [`preset_file`], plus the file the old lossy sanitizer would have used
/// for it — the only other place a preset of that name can be. Checking
/// just those two keeps this cheap enough for the save prompt, which asks
/// on every keystroke.
fn files_named(dir: &Path, name: &str) -> Vec<PathBuf> {
    let current = preset_file(dir, name);
    let legacy = dir.join(format!("{}.json", legacy_filename(name)));
    let mut out = Vec::new();
    for path in [current, legacy] {
        if !out.contains(&path)
            && load_preset_file(&path).is_ok_and(|preset| preset.name == name)
        {
            out.push(path);
        }
    }
    out
}

/// The pre-STATE-15 filename scheme: anything but alphanumerics, `-` and
/// `_` became `_`. Only used to find presets saved before the switch.
fn legacy_filename(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

/// The file a preset named `name` is saved to.
fn preset_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.json", encode_filename(name)))
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
                        eprintln!(
                            "Warning: preset {} is corrupt ({e}); quarantining it as \
                             a .corrupt file rather than overwriting it on the next save",
                            path.display()
                        );
                        quarantine_corrupt(&path);
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
///
/// Refuses an empty name, and refuses to replace a file that holds a
/// *different* preset (a case-insensitive filesystem folds "Bass" and
/// "bass" onto one file) rather than silently losing it. Replacing a
/// preset of the same name is the caller's call — both surfaces confirm
/// through [`user_preset_exists`] first.
pub fn save_user_preset(preset: &TrackPreset) -> Result<PathBuf, String> {
    if preset.name.trim().is_empty() {
        return Err("name a preset before saving it".to_string());
    }
    let dir = presets_dir().ok_or_else(|| "Could not determine data directory".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Create presets dir: {e}"))?;

    let path = preset_file(&dir, &preset.name);
    if path.exists() {
        match load_preset_file(&path) {
            Ok(existing) if existing.name == preset.name => {}
            Ok(existing) => {
                return Err(format!(
                    "{} already holds the preset {:?}; save under another name",
                    path.display(),
                    existing.name
                ))
            }
            Err(e) => {
                return Err(format!(
                    "{} exists and is not a readable preset ({e}); not replacing it",
                    path.display()
                ))
            }
        }
    }
    let json =
        serde_json::to_string_pretty(preset).map_err(|e| format!("Serialize preset: {e}"))?;
    atomic_write(&path, json.as_bytes())?;
    // Overwriting a preset stored under an older filename: drop that copy
    // so the name is not listed twice.
    for stale in files_named(&dir, &preset.name) {
        if stale != path {
            let _ = std::fs::remove_file(stale);
        }
    }
    Ok(path)
}

/// Delete a user preset from disk: every file whose stored name is `name`,
/// and nothing else (code review STATE-15).
pub fn delete_user_preset(name: &str) -> Result<(), String> {
    let dir = presets_dir().ok_or_else(|| "Could not determine data directory".to_string())?;
    for path in files_named(&dir, name) {
        std::fs::remove_file(&path).map_err(|e| format!("Delete preset: {e}"))?;
    }
    Ok(())
}

/// Injective preset-name → file-stem encoding (code review STATE-15).
///
/// Alphanumerics, `-`, `_` and space are kept so the files stay readable;
/// every other character — `%` itself included — becomes `%XX` per UTF-8
/// byte. Distinct names therefore never share a file (the old sanitizer
/// mapped "A B", "A.B" and "A_B" all to `A_B.json`), and a name of dots
/// can neither hide the file nor step out of the directory.
fn encode_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}
