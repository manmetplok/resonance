//! Reading the voicebank folders into [`InstalledBank`]s: which folders
//! are banks, the id each one gets, which of two banks with one id wins,
//! and keeping a bank whose files have not changed since the last scan.
//!
//! **Ids.** A user bank's id is its folder name as a slug
//! ([`layout::slugify`]). A folder whose name slugs to nothing (`日本語`,
//! `___`) takes the slug of the name its character file gives; with no
//! usable name either it is skipped with a warning.
//!
//! **Precedence when two banks share an id.** The user voicebank folder
//! beats every development root, so an installed bank replaces the
//! source-tree copy of the same id. Within the user folder the first
//! folder in sorted order wins and the others are skipped with a warning
//! naming both. Among development roots, `$RESONANCE_SVS_MODELS_DIR`
//! beats the source tree's `resonance-svs/models/` (the order
//! [`VoicebankRoots::dev_models`](super::VoicebankRoots) lists them).
//!
//! **Reuse.** A bank found in the same folder as last time, whose
//! [`BankStamp`] still matches the files on disk, keeps its
//! `Arc<InstalledBank>` — parsed configs, inventory and the lazily-read
//! dictionary included. A bank whose vocoder was missing is always
//! re-read: the vocoder it names may have been installed since.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use resonance_svs::voicebank::{self, layout, VoicebankManifest};

use super::stamp::{BankStamp, StampedPaths};
use super::{builtin, profile, InstalledBank, VoicebankRegistry, BUILTIN_BANKS};

type Banks = BTreeMap<String, Arc<InstalledBank>>;

impl VoicebankRegistry {
    /// Re-read every root: a bank installed, replaced or deleted since
    /// the last scan shows up (or goes) now. See the module docs for ids,
    /// precedence and reuse.
    pub fn rescan(&mut self) {
        let old = std::mem::take(&mut self.banks);
        let mut banks = Banks::new();
        if let Some(dir) = self.roots.banks.clone() {
            for sub in subdirs(&dir) {
                if let Some(bank) = self.user_bank(&old, &sub) {
                    claim(&mut banks, bank);
                }
            }
        }
        for models in self.roots.dev_models.clone() {
            for b in BUILTIN_BANKS {
                let dir = models.join(b.dev_dir);
                if !dir.is_dir() {
                    continue;
                }
                if let Some(winner) = banks.get(b.id) {
                    tracing::debug!(
                        "voicebank {} at {} is shadowed by {}",
                        b.id,
                        dir.display(),
                        winner.scanned_dir.display()
                    );
                    continue;
                }
                let bank = reusable(&old, &dir, false).or_else(|| {
                    let manifest = scan_manifest(&dir, b.id)?;
                    Some(Arc::new(self.build(manifest, b.id.to_owned(), false, &dir)))
                });
                if let Some(bank) = bank {
                    banks.insert(b.id.to_owned(), bank);
                }
            }
        }
        self.banks = banks;
    }

    /// The bank in user folder `dir`, or `None` when it is not one (or has
    /// no usable id).
    fn user_bank(&self, old: &Banks, dir: &Path) -> Option<Arc<InstalledBank>> {
        if let Some(bank) = reusable(old, dir, true) {
            return Some(bank);
        }
        if !layout::looks_like_bank(dir) {
            return None;
        }
        let folder = dir.file_name()?.to_string_lossy().into_owned();
        let manifest = scan_manifest(dir, &folder)?;
        let mut id = layout::slugify(&folder);
        if id.is_empty() {
            id = manifest
                .details
                .name
                .as_deref()
                .map(layout::slugify)
                .unwrap_or_default();
            if id.is_empty() {
                tracing::warn!(
                    "voicebank folder {} is skipped: its name gives no id and its character \
                     file names none; rename the folder to an ASCII name",
                    dir.display()
                );
                return None;
            }
            tracing::warn!(
                "voicebank folder {} gives no id; using {id:?} from its character file",
                dir.display()
            );
        }
        Some(Arc::new(self.build(manifest, id, true, dir)))
    }

    /// An [`InstalledBank`] from its scanned manifest.
    fn build(
        &self,
        manifest: VoicebankManifest,
        id: String,
        user: bool,
        dir: &Path,
    ) -> InstalledBank {
        let builtin = builtin::builtin(&id);
        let declared = Some(manifest.details.vocoder_name.clone()).filter(|v| !v.is_empty());
        let vocoder_config = manifest
            .vocoder_config
            .clone()
            .or_else(|| self.library_vocoder(declared.as_deref()?));
        // Report the vocoder by its own name when it is installed (the
        // config may only name the folder it sits in, `dsvocoder`).
        let installed_name = vocoder_config
            .as_deref()
            .and_then(layout::installed_vocoder_name);
        let vocoder_config = vocoder_config.filter(|_| installed_name.is_some());
        let vocoder_name = installed_name.or(declared);
        let speakers = profile::offered_speakers(builtin, &manifest.singers);
        let name = builtin
            .map(|b| b.name.to_owned())
            .or_else(|| manifest.details.name.clone())
            .unwrap_or_else(|| manifest.display_name.clone());
        let stamp = BankStamp::read(&StampedPaths {
            root: &manifest.root,
            acoustic_config: &manifest.acoustic_config,
            phoneme_dict: &manifest.phoneme_dict,
            english_dict: manifest.details.english_dict.as_deref(),
            vocoder_config: vocoder_config.as_deref(),
        });
        InstalledBank {
            id,
            name,
            version: manifest.details.version.clone(),
            root: manifest.root.clone(),
            scanned_dir: dir.to_path_buf(),
            stamp,
            user,
            acoustic_config: manifest.acoustic_config.clone(),
            vocoder_name,
            vocoder_config,
            speakers,
            curves: manifest.curve_support(),
            accepts_language_id: manifest.accepts_language_id,
            languages: manifest.languages.clone(),
            inventory: manifest.inventory().clone(),
            english_dict: manifest.details.english_dict.clone(),
            builtin,
            dictionary: Default::default(),
        }
    }

    /// A shared vocoder by id: the user vocoder folder, then each
    /// development root's `vocoder/` folder.
    fn library_vocoder(&self, name: &str) -> Option<PathBuf> {
        self.roots
            .vocoders
            .iter()
            .cloned()
            .chain(self.roots.dev_models.iter().map(|m| m.join("vocoder")))
            .find_map(|lib| layout::find_library_vocoder(&lib, name))
    }
}

/// Add `bank` unless an earlier folder already holds its id.
fn claim(banks: &mut Banks, bank: Arc<InstalledBank>) {
    if let Some(first) = banks.get(&bank.id) {
        tracing::warn!(
            "voicebank folders {} and {} both have the id {:?}; using the first",
            first.scanned_dir.display(),
            bank.scanned_dir.display(),
            bank.id
        );
        return;
    }
    banks.insert(bank.id.clone(), bank);
}

/// Last scan's bank from folder `dir`, when nothing it was read from has
/// changed (see the module docs).
fn reusable(old: &Banks, dir: &Path, user: bool) -> Option<Arc<InstalledBank>> {
    old.values()
        .find(|b| b.scanned_dir == dir && b.user == user)
        .filter(|b| b.vocoder_config.is_some() && b.stamp.is_current())
        .cloned()
}

fn scan_manifest(dir: &Path, id: &str) -> Option<VoicebankManifest> {
    match voicebank::scan(dir) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::warn!("voicebank {id} at {}: {e:#}", dir.display());
            None
        }
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}
