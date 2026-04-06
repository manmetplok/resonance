use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub buffer_size: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { buffer_size: 256 }
    }
}

pub const BUFFER_SIZE_OPTIONS: &[u32] = &[64, 128, 256, 512, 1024, 2048];

impl Settings {
    fn config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("resonance").join("settings.toml"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        let mut settings = match std::fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_else(|e| {
                eprintln!("Failed to parse settings, using defaults: {}", e);
                Self::default()
            }),
            Err(_) => Self::default(),
        };
        if !BUFFER_SIZE_OPTIONS.contains(&settings.buffer_size) {
            eprintln!(
                "Invalid buffer_size {} in settings, using default",
                settings.buffer_size
            );
            settings.buffer_size = 256;
        }
        settings
    }

    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            eprintln!("Could not determine config directory");
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match toml::to_string_pretty(self) {
            Ok(contents) => {
                if let Err(e) = std::fs::write(&path, contents) {
                    eprintln!("Failed to save settings: {}", e);
                }
            }
            Err(e) => eprintln!("Failed to serialize settings: {}", e),
        }
    }
}
