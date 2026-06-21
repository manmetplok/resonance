//! Project templates: storage layout, metadata model, and user-template scanning.
//!
//! Templates are stored as folders with the same on-disk shape as saved projects
//! (project.json + midi/ + plugins/ + audio/), plus a sibling `template.json` metadata
//! sidecar. This allows templates to round-trip through the existing project I/O
//! path.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::project::{ProjectFile, PROJECT_FORMAT_VERSION};

/// Directory name for user templates under the config directory.
const TEMPLATES_DIR_NAME: &str = "templates";

/// Application directory name (same as used in recent.rs).
const APP_DIR: &str = "resonance";

/// Template storage directory, lazily created.
/// Mirrors the location pattern in `recent.rs`.
pub fn templates_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join(TEMPLATES_DIR_NAME))
}

/// Ensure the user templates directory exists, creating it if necessary.
/// Returns the path or None if the config directory cannot be resolved.
pub fn ensure_templates_dir() -> Option<PathBuf> {
    let dir = templates_dir()?;
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("Failed to create templates directory: {e}");
        None
    } else {
        Some(dir)
    }
}

/// Kind of template: built-in (defined in code) or user-created (from disk).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TemplateKind {
    Builtin,
    User,
}

/// Summary statistics precomputed from a project for display in the template picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateSummary {
    pub track_count: usize,
    pub bus_count: usize,
    pub plugin_count: usize,
    pub tempo_bpm: f32,
    pub time_sig: String,
}

/// Metadata sidecar stored alongside each template folder as `template.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMetadata {
    /// Human-readable name for display.
    pub name: String,
    /// Optional description for the template picker.
    pub description: String,
    /// Whether this template is built-in (always current schema) or user-created.
    pub built_in: bool,
    /// The project schema version this template was captured at.
    pub schema_version: u32,
    /// Precomputed summary for display.
    pub summary: TemplateSummary,
    /// Unix timestamp when the template was created.
    pub created_secs: u64,
}

/// Reason why a template entry is marked as stale.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StaleReason {
    /// The template's schema_version is newer than the current build.
    SchemaVersionNewer { schema_version: u32 },
    /// The project.json file failed to parse.
    ProjectParseError { reason: String },
    /// The template.json file failed to parse.
    MetadataParseError { reason: String },
}

/// A user template entry that is stale (incompatible or corrupted).
/// The underlying files are left untouched for potential recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleTemplate {
    /// Path to the template folder.
    pub path: PathBuf,
    /// Reason for the stale status.
    pub reason: StaleReason,
    /// The schema version from template.json if available.
    pub schema_version: Option<u32>,
}

/// A resolved template entry from the user templates directory.
#[derive(Debug, Clone)]
pub enum TemplateEntry {
    /// A valid, loadable template.
    Valid(Template),
    /// A stale template that cannot be loaded but is kept for recovery.
    Stale(StaleTemplate),
}

/// A complete template ready for instantiation.
#[derive(Debug, Clone)]
pub struct Template {
    /// Kind: built-in or user.
    pub kind: TemplateKind,
    /// Human-readable name.
    pub name: String,
    /// Optional description.
    pub description: String,
    /// Precomputed summary.
    pub summary: TemplateSummary,
    /// Absolute path to the template directory (for user templates) or a placeholder
    /// (for built-ins which don't exist on disk).
    pub path: PathBuf,
    /// Schema version of the captured project.
    pub schema_version: u32,
    /// Unix timestamp when created.
    pub created_secs: Option<u64>,
}

impl Template {
    /// Create a new user template from metadata and path.
    pub fn new_user(metadata: TemplateMetadata, path: PathBuf) -> Self {
        Self {
            kind: TemplateKind::User,
            name: metadata.name,
            description: metadata.description,
            summary: metadata.summary,
            path,
            schema_version: metadata.schema_version,
            created_secs: Some(metadata.created_secs),
        }
    }

    /// Create a new built-in template.
    pub fn new_builtin(
        name: String,
        description: String,
        summary: TemplateSummary,
    ) -> Self {
        Self {
            kind: TemplateKind::Builtin,
            name,
            description,
            summary,
            // Built-ins don't have a disk path; use a placeholder.
            path: PathBuf::new(),
            schema_version: PROJECT_FORMAT_VERSION,
            created_secs: None,
        }
    }
}

/// Scan the user templates directory and return all discovered templates.
///
/// This function:
/// - Resolves the templates directory (lazily creating it if missing)
/// - Enumerates all subdirectories
/// - For each directory, attempts to parse `template.json`
/// - If template.json is valid and parseable:
///   - Checks if schema_version > PROJECT_FORMAT_VERSION or if project.json fails to parse
///   - Returns a Stale entry if either check fails (without touching files)
///   - Returns a Valid Template entry if all checks pass
/// - Missing/unreadable directory -> returns empty list, never an error
/// - All I/O errors are swallowed and logged to stderr
pub fn scan_user_templates() -> Vec<TemplateEntry> {
    let Some(dir) = templates_dir() else {
        return Vec::new();
    };

    // If the directory doesn't exist, return empty (lazy creation happens on first save).
    if !dir.exists() {
        return Vec::new();
    }

    // If it exists but isn't a directory, return empty.
    if !dir.is_dir() {
        eprintln!("Templates path exists but is not a directory: {}", dir.display());
        return Vec::new();
    }

    let mut results = Vec::new();

    // Try to read the directory entries.
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Failed to read templates directory: {e}");
            return Vec::new();
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Failed to read templates directory entry: {e}");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_dir() {
            continue; // Skip non-directories
        }

        // Look for template.json in this directory.
        let template_json_path = path.join("template.json");
        let project_json_path = path.join("project.json");

        // Try to read and parse template.json.
        let template_metadata: TemplateMetadata = match fs::read_to_string(&template_json_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(m) => m,
                Err(e) => {
                    results.push(TemplateEntry::Stale(StaleTemplate {
                        path: path.clone(),
                        reason: StaleReason::MetadataParseError {
                            reason: format!("Failed to parse template.json: {e}"),
                        },
                        schema_version: None,
                    }));
                    continue;
                }
            },
            Err(e) => {
                eprintln!(
                    "Failed to read template.json at {}: {e}",
                    template_json_path.display()
                );
                // If template.json is missing or unreadable, this isn't a valid template.
                // We could mark it as stale, but without metadata we don't have schema_version.
                // For now, skip it silently (it might be a partial/aborted save).
                continue;
            }
        };

        // Check if schema_version is newer than current build.
        if template_metadata.schema_version > PROJECT_FORMAT_VERSION {
            results.push(TemplateEntry::Stale(StaleTemplate {
                path: path.clone(),
                reason: StaleReason::SchemaVersionNewer {
                    schema_version: template_metadata.schema_version,
                },
                schema_version: Some(template_metadata.schema_version),
            }));
            continue;
        }

        // Try to parse project.json to verify it's valid.
        // We don't need the full ProjectFile, just need to check it parses.
        let _: ProjectFile = match fs::read_to_string(&project_json_path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(p) => p,
                Err(e) => {
                    results.push(TemplateEntry::Stale(StaleTemplate {
                        path: path.clone(),
                        reason: StaleReason::ProjectParseError {
                            reason: format!("Failed to parse project.json: {e}"),
                        },
                        schema_version: Some(template_metadata.schema_version),
                    }));
                    continue;
                }
            },
            Err(e) => {
                eprintln!(
                    "Failed to read project.json at {}: {e}",
                    project_json_path.display()
                );
                results.push(TemplateEntry::Stale(StaleTemplate {
                    path: path.clone(),
                    reason: StaleReason::ProjectParseError {
                        reason: format!("Failed to read project.json: {e}"),
                    },
                    schema_version: Some(template_metadata.schema_version),
                }));
                continue;
            }
        };

        // All checks passed - this is a valid template.
        results.push(TemplateEntry::Valid(Template::new_user(
            template_metadata,
            path,
        )));
    }

    results
}

/// Compute a TemplateSummary from a ProjectFile.
pub fn compute_summary(project: &ProjectFile) -> TemplateSummary {
    TemplateSummary {
        track_count: project.tracks.len(),
        bus_count: project.busses.len(),
        plugin_count: project
            .tracks
            .iter()
            .map(|t| t.plugins.len())
            .sum::<usize>()
            + project
                .busses
                .iter()
                .map(|b| b.plugins.len())
                .sum::<usize>()
            + project.master_plugins.len(),
        tempo_bpm: project.bpm,
        time_sig: format!("{}/{}", project.time_sig_num, project.time_sig_den),
    }
}
