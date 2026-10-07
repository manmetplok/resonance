//! One voicebank install, on its own thread (see [`super::library`]):
//!
//! 1. the index entry (the index is fetched when stale);
//! 2. a bank already installed as the index lists it, with its vocoder,
//!    is left alone (`already_installed`);
//! 3. the bank's upstream file is downloaded into the user voicebank
//!    folder (resumable `.part`, sha256 against the index — a mismatch is
//!    "upstream file changed; the index needs updating") and unpacked into
//!    `<voicebanks>/<id>/`, a wrapper directory stripped, replacing an
//!    older install only once the new one is whole;
//! 4. the install record ([`super::plok_index::SIDECAR_FILE`]) keeps the
//!    entry's version, hash, terms and credit;
//! 5. a bank that ships its own vocoder uses it; otherwise the vocoder its
//!    `dsconfig.yaml` names (or the entry's `vocoder`) is looked up in the
//!    vocoder library and, when missing, fetched from the index's
//!    `vocoders[]` into `<vocoders>/<vocoder id>/`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use resonance_common::drumkit_library::{ImportJob, ImportProgress};
use resonance_common::plok_download::package::{install_package, PackageRule};
use resonance_common::plok_download::{self as plok, DownloadError, Stop, Target};
use resonance_svs::voicebank::{self, layout};

use super::library::{
    InstallError, InstallErrorKind as Kind, InstallOutcome, InstallPhase, VoicebankLibrary,
};
use super::plok_index::{PlokIndex, PlokSidecar, PlokVocoder, PlokVoicebank};
use super::VoicebankRoots;
use crate::control_jobs::JobBoard;

/// How deep a bank's `dsconfig.yaml` may sit below its folder (the
/// registry's own rule, §4.2: Meiji nests `configs/configs/`).
const BANK_MARKER_DEPTH: usize = 2;
/// A vocoder's `vocoder.yaml` sits at its folder's top (a wrapper is
/// stripped first), or one folder down.
const VOCODER_MARKER_DEPTH: usize = 1;

pub(super) struct Install<'a> {
    pub lib: &'a VoicebankLibrary,
    pub bank_id: &'a str,
    pub roots: &'a VoicebankRoots,
    pub cancel: &'a AtomicBool,
    /// The control job and the board its progress goes to.
    pub board: Option<(u64, &'a JobBoard)>,
}

fn failed(message: impl Into<String>) -> InstallError {
    InstallError::new(Kind::Failed, message)
}

impl Install<'_> {
    fn phase(&self, phase: InstallPhase) {
        if let (Some((job, board)), Some(f)) = (self.board, phase.fraction()) {
            board.set_progress(job, f);
        }
        self.lib.set_phase(self.bank_id, phase);
    }

    fn check_cancel(&self) -> Result<(), InstallError> {
        if self.cancel.load(Ordering::SeqCst) {
            Err(InstallError::new(Kind::Cancelled, "cancelled"))
        } else {
            Ok(())
        }
    }

    pub fn run(&self) -> Result<InstallOutcome, InstallError> {
        let banks = self
            .roots
            .banks
            .clone()
            .ok_or_else(|| failed("there is no user voicebank folder"))?;
        self.phase(InstallPhase::FetchingIndex);
        let index = self
            .lib
            .index_for_install(Some(&banks))
            .map_err(|e| InstallError::new(Kind::NotFound, format!("the plok.org index: {e}")))?;
        let entry = index.voicebank(self.bank_id).cloned().ok_or_else(|| {
            InstallError::new(
                Kind::NotFound,
                format!(
                    "the plok.org index lists no voicebank {:?}; vocal_voicebanks lists the \
                     available ids",
                    self.bank_id
                ),
            )
        })?;
        self.check_cancel()?;
        let dir = banks.join(&entry.id);

        let current = dir.is_dir()
            && layout::looks_like_bank(&dir)
            && PlokSidecar::read(&dir).is_some_and(|s| s.matches(&entry));
        if !current {
            self.install_bank(&entry, &banks, &dir)?;
        }
        let (vocoder, vocoder_installed, vocoder_fetched) = self.ensure_vocoder(&entry, &index, &dir)?;
        Ok(InstallOutcome {
            id: entry.id.clone(),
            name: entry.name.clone(),
            version: entry.version.clone(),
            dir,
            terms_url: entry.terms_url.clone(),
            credit: entry.credit.clone(),
            commercial: entry.commercial.clone(),
            vocoder,
            vocoder_installed,
            vocoder_fetched,
            already_installed: current && !vocoder_fetched,
        })
    }

    fn install_bank(
        &self,
        entry: &PlokVoicebank,
        banks: &Path,
        dir: &Path,
    ) -> Result<(), InstallError> {
        let target = entry.target();
        let is_marker = |p: &Path| p.file_name().is_some_and(|n| n == "dsconfig.yaml");
        let rule = PackageRule {
            what: "voicebank",
            is_marker: &is_marker,
            max_depth: BANK_MARKER_DEPTH,
        };
        self.fetch_and_unpack(&target, banks, dir, &rule)?;
        if !layout::looks_like_bank(dir) {
            let _ = std::fs::remove_dir_all(dir);
            return Err(failed(format!(
                "{} unpacked, but it holds no acoustic dsconfig.yaml — it is not a DiffSinger \
                 voicebank in OpenUtau layout",
                entry.name
            )));
        }
        entry
            .sidecar()
            .write(dir)
            .map_err(|e| failed(format!("record the install of {}: {e}", entry.name)))
    }

    /// Download `target` into `part_dir` and unpack it as `dest`.
    fn fetch_and_unpack(
        &self,
        target: &Target,
        part_dir: &Path,
        dest: &Path,
        rule: &PackageRule<'_>,
    ) -> Result<(), InstallError> {
        let config = self.lib.config();
        let agent = plok::http::agent(&config.http, None);
        let ctl = Transfer {
            install: self,
            name: &target.name,
            free_space: config.free_space.as_deref(),
        };
        let part = plok::download(&agent, target, part_dir, self.lib.nonce(), &ctl)
            .map_err(|e| download_error(e, target))?;
        if let Err(e) = self.check_cancel() {
            part.remove();
            return Err(e);
        }
        let what = target.name.clone();
        let mut progress = |p: ImportProgress| {
            self.phase(InstallPhase::Extracting {
                what: what.clone(),
                done: p.bytes_done,
                total: p.bytes_total,
            });
        };
        let mut job = ImportJob::new().cancel(self.cancel).progress(&mut progress);
        if let Some(probe) = config.free_space.clone() {
            job = job.free_space(move |p| probe(p));
        }
        let result = install_package(part.path(), dest, rule, job);
        part.remove();
        result.map(drop).map_err(|e| {
            if e.is_cancelled() {
                InstallError::new(Kind::Cancelled, "cancelled")
            } else {
                failed(format!("install {}: {e}", target.name))
            }
        })
    }

    /// The bank's vocoder: its own, else the shared one it names —
    /// installed already, or fetched now from the index. Returns (its
    /// name, installed, fetched now).
    fn ensure_vocoder(
        &self,
        entry: &PlokVoicebank,
        index: &PlokIndex,
        dir: &Path,
    ) -> Result<(Option<String>, bool, bool), InstallError> {
        let manifest = voicebank::scan(dir)
            .map_err(|e| failed(format!("read the installed {}: {e:#}", entry.name)))?;
        if let Some(own) = manifest
            .vocoder_config
            .as_deref()
            .and_then(layout::installed_vocoder_name)
        {
            return Ok((Some(own), true, false));
        }
        let declared = Some(manifest.details.vocoder_name.clone()).filter(|v| !v.is_empty());
        let wanted = entry.vocoder.clone().or(declared);
        let Some(name) = wanted else {
            return Ok((None, false, false));
        };
        if self.library_has_vocoder(&name) {
            return Ok((Some(name), true, false));
        }
        let Some(vocoder) = index.vocoder(&name) else {
            tracing::warn!(
                "voicebank {}: its vocoder {name:?} is not installed and the index does not list it",
                entry.id
            );
            return Ok((Some(name), false, false));
        };
        self.install_vocoder(vocoder)?;
        Ok((Some(name), true, true))
    }

    fn library_has_vocoder(&self, name: &str) -> bool {
        self.roots
            .vocoders
            .iter()
            .cloned()
            .chain(self.roots.dev_models.iter().map(|m| m.join("vocoder")))
            .any(|lib| {
                layout::find_library_vocoder(&lib, name)
                    .is_some_and(|cfg| layout::installed_vocoder_name(&cfg).is_some())
            })
    }

    fn install_vocoder(&self, vocoder: &PlokVocoder) -> Result<(), InstallError> {
        self.check_cancel()?;
        let lib = self
            .roots
            .vocoders
            .clone()
            .ok_or_else(|| failed("there is no user vocoder folder"))?;
        let dest = lib.join(&vocoder.id);
        let is_marker = |p: &Path| p.file_name().is_some_and(|n| n == "vocoder.yaml");
        let rule = PackageRule {
            what: "vocoder",
            is_marker: &is_marker,
            max_depth: VOCODER_MARKER_DEPTH,
        };
        self.fetch_and_unpack(&vocoder.target(), &lib, &dest, &rule)?;
        let found = layout::find_library_vocoder(&lib, &vocoder.id)
            .and_then(|cfg| layout::installed_vocoder_name(&cfg));
        if found.is_none() {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(failed(format!(
                "{} unpacked, but holds no usable vocoder.yaml and model",
                vocoder.name
            )));
        }
        vocoder
            .sidecar()
            .write(&dest)
            .map_err(|e| failed(format!("record the install of {}: {e}", vocoder.name)))
    }
}

fn download_error(e: DownloadError, target: &Target) -> InstallError {
    match e {
        DownloadError::Cancelled | DownloadError::Abandoned => {
            InstallError::new(Kind::Cancelled, "cancelled")
        }
        DownloadError::ChecksumMismatch {
            actual, expected, ..
        } => InstallError::new(
            Kind::UpstreamChanged,
            format!(
                "upstream file changed; the index needs updating ({} at {}: sha256 {actual}, the \
                 index pins {expected}). Nothing was installed.",
                target.name, target.url
            ),
        ),
        DownloadError::Failed(e) => failed(e),
    }
}

/// The install's side of a running download.
struct Transfer<'a, 'b> {
    install: &'a Install<'b>,
    name: &'a str,
    free_space: Option<&'a super::library::FreeSpaceProbe>,
}

impl plok::Control for Transfer<'_, '_> {
    fn stop(&self) -> Option<Stop> {
        self.install
            .cancel
            .load(Ordering::SeqCst)
            .then_some(Stop::Cancelled)
    }

    fn progress(&self, p: plok::Progress) {
        self.install.phase(InstallPhase::Downloading {
            what: self.name.to_string(),
            done: p.downloaded_bytes,
            total: p.total_bytes,
        });
    }

    fn verifying(&self) {
        self.install
            .phase(InstallPhase::Verifying(self.name.to_string()));
    }

    fn free_space(&self, at: &Path) -> Option<u64> {
        match self.free_space {
            Some(probe) => probe(at),
            None => resonance_common::drumkit_library::free_space(at),
        }
    }
}
