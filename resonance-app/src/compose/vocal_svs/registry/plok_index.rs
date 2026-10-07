//! The voicebank half of the plok.org index (vocal-mcp-control-and-quality.md
//! §5), and the record an install leaves in the bank's folder.
//!
//! `index.json` carries optional `voicebanks[]` and `vocoders[]` next to
//! the drums' `drumkits[]`. An entry's `url` is the **creator's own**
//! download (a GitHub release asset): bank terms forbid rehosting, so
//! plok.org holds the index only. `sha256` pins that upstream file; a
//! download whose hash differs fails with "upstream file changed; the
//! index needs updating" rather than installing something nobody vetted.
//!
//! Everything past `id`, `name` and `url` is optional and read leniently
//! (`plok_download::lenient`): one odd field does not cost the user the
//! whole list.

use std::path::Path;

use resonance_common::plok_download::{is_safe_id, lenient, lenient_tags, Target};
use serde::{Deserialize, Serialize};

/// The record an install writes into the bank (or vocoder) folder: what
/// was installed from where, and the terms it came with.
pub const SIDECAR_FILE: &str = ".resonance-plok.json";

/// The lists of `index.json` the voicebank library reads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlokIndex {
    #[serde(default, deserialize_with = "lenient_list")]
    pub voicebanks: Vec<PlokVoicebank>,
    #[serde(default, deserialize_with = "lenient_list")]
    pub vocoders: Vec<PlokVocoder>,
}

/// An index entry: its `id` names the folder it installs into.
trait IndexEntry {
    fn id(&self) -> &str;
}

impl IndexEntry for PlokVoicebank {
    fn id(&self) -> &str {
        &self.id
    }
}

impl IndexEntry for PlokVocoder {
    fn id(&self) -> &str {
        &self.id
    }
}

/// Keep the entries of a list that parse and whose id is a safe folder
/// name ([`plok_download::is_safe_id`](resonance_common::plok_download::is_safe_id):
/// `../x`, `/abs`, `a/b` or `.hidden` would install or delete outside the
/// library); drop the rest (and a non-list).
fn lenient_list<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned + IndexEntry,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|i| serde_json::from_value::<T>(i).ok())
            .filter(|e| {
                let safe = is_safe_id(e.id());
                if !safe {
                    tracing::warn!("plok.org index: entry id {:?} is not a folder name; skipped", e.id());
                }
                safe
            })
            .collect(),
        _ => Vec::new(),
    })
}

impl PlokIndex {
    pub fn voicebank(&self, id: &str) -> Option<&PlokVoicebank> {
        self.voicebanks.iter().find(|b| b.id == id)
    }

    pub fn vocoder(&self, id: &str) -> Option<&PlokVocoder> {
        self.vocoders.iter().find(|v| v.id == id)
    }
}

/// One downloadable voicebank.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlokVoicebank {
    /// The bank id once installed (its folder name; what
    /// `vocal.set_params` takes).
    pub id: String,
    pub name: String,
    /// The creator's download URL.
    pub url: String,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Language codes the bank sings (`en`, `ja`, …).
    #[serde(default, deserialize_with = "lenient_tags", skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    /// Voice modes, for display.
    #[serde(default, deserialize_with = "lenient_tags", skip_serializing_if = "Vec::is_empty")]
    pub speakers: Vec<String>,
    /// Display size ("609 MB"); the fallback when `bytes` is absent.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// sha256 of the upstream file, lowercase hex.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub terms_url: Option<String>,
    /// The credit line the terms ask for.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    /// `free_with_credit`, `paid` or `permission`.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub commercial: Option<String>,
    /// The vocoder (a `vocoders[]` id) the bank needs when it does not
    /// ship its own; the bank's `dsconfig.yaml` names it too.
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub vocoder: Option<String>,
    /// When the entry was added (ISO date).
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub added: Option<String>,
}

/// One downloadable vocoder. Its `id` is the vocoder's name as a bank's
/// `dsconfig.yaml` names it (`pc_nsf_hifigan_44.1k_hop512_128bin_2025.02`),
/// and the folder it installs into under the vocoder library.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlokVocoder {
    pub id: String,
    pub name: String,
    pub url: String,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub terms_url: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub commercial: Option<String>,
    #[serde(default, deserialize_with = "lenient", skip_serializing_if = "Option::is_none")]
    pub added: Option<String>,
}

/// The file part of a URL (`…/Gahata_Meiji_v170.zip` → `Gahata_Meiji_v170.zip`):
/// what the download's part file is named after.
fn url_file(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or("download")
        .to_string()
}

impl PlokVoicebank {
    pub fn target(&self) -> Target {
        Target {
            name: self.name.clone(),
            file: url_file(&self.url),
            url: self.url.clone(),
            bytes: self.bytes,
            sha256: self.sha256.clone(),
        }
    }

    /// "609.3 MB" from `bytes`, else the index's `size`.
    pub fn size_text(&self) -> Option<String> {
        self.bytes
            .map(resonance_common::drumkit_library::format_bytes)
            .or_else(|| self.size.clone())
    }

    pub fn sidecar(&self) -> PlokSidecar {
        PlokSidecar {
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            url: self.url.clone(),
            sha256: self.sha256.as_ref().map(|s| s.trim().to_ascii_lowercase()),
            terms_url: self.terms_url.clone(),
            credit: self.credit.clone(),
            commercial: self.commercial.clone(),
            installed_at: now_rfc3339(),
        }
    }
}

impl PlokVocoder {
    pub fn target(&self) -> Target {
        Target {
            name: self.name.clone(),
            file: url_file(&self.url),
            url: self.url.clone(),
            bytes: self.bytes,
            sha256: self.sha256.clone(),
        }
    }

    pub fn sidecar(&self) -> PlokSidecar {
        PlokSidecar {
            id: self.id.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            url: self.url.clone(),
            sha256: self.sha256.as_ref().map(|s| s.trim().to_ascii_lowercase()),
            terms_url: self.terms_url.clone(),
            credit: self.credit.clone(),
            commercial: self.commercial.clone(),
            installed_at: now_rfc3339(),
        }
    }
}

fn now_rfc3339() -> String {
    use resonance_common::library_marks::{format_timestamp, now_unix};
    format_timestamp(now_unix()).unwrap_or_default()
}

/// What an install recorded in [`SIDECAR_FILE`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlokSidecar {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terms_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commercial: Option<String>,
    #[serde(default)]
    pub installed_at: String,
}

impl PlokSidecar {
    /// The record in `dir`, if an install left one.
    pub fn read(dir: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(dir.join(SIDECAR_FILE)).ok()?).ok()
    }

    pub fn write(&self, dir: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        resonance_common::atomic_write(&dir.join(SIDECAR_FILE), &bytes)
            .map_err(std::io::Error::other)
    }

    /// Whether this install is `entry` as the index lists it now: the same
    /// pinned file, else the same version and URL.
    pub fn matches(&self, entry: &PlokVoicebank) -> bool {
        match (&self.sha256, &entry.sha256) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b.trim()),
            _ => self.version == entry.version && self.url == entry.url,
        }
    }
}
