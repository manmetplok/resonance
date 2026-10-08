//! Test-only access to the voicebank registry, and a fixture writer for
//! minimal on-disk banks: configs, a phoneme list, an optional English
//! dictionary and a vocoder — no real ONNX, which nothing short of a real
//! render reads (vocal-mcp-control-and-quality.md §4).

use std::path::{Path, PathBuf};

use crate::compose::vocal_svs::registry::{LibraryConfig, VoicebankLibrary};
use crate::compose::vocal_svs::{BankProfile, VoicebankRoots};
use crate::state::voicebank_library::LibraryOverlay;
use crate::Resonance;

/// What [`write_voicebank_fixture`] writes.
#[derive(Debug, Clone)]
pub struct VoicebankFixture {
    /// The acoustic config's `speakers` (empty: a single-voice bank).
    pub speakers: Vec<String>,
    /// The phoneme inventory, one symbol per line.
    pub phonemes: Vec<String>,
    /// `dsdict-en.yaml` entries as `(grapheme, phonemes)`.
    pub english_dict: Vec<(String, Vec<String>)>,
    /// `dsdict-en.yaml` `replacements:` as `(from, to)`.
    pub english_replacements: Vec<(String, String)>,
    /// Write a bundled `dsvocoder/` (config + empty model file)?
    pub vocoder: bool,
    /// Without a bundled vocoder: the shared vocoder the acoustic config
    /// names (`vocoder: <name>`), as a bank that relies on the vocoder
    /// library does.
    pub vocoder_name: Option<String>,
    pub use_tension_embed: bool,
    /// `use_energy_embed: true` (no shipped bank has it).
    pub use_energy_embed: bool,
    /// `use_lang_id: true` plus this `languages.json` body.
    pub languages: Option<String>,
    /// `character.txt` name.
    pub name: Option<String>,
    /// Ship the bank's own duration / pitch / variance models (`dsdur/`,
    /// `dspitch/`, `dsvariance/` configs, the phoneme list, empty model
    /// files and the acoustic speakers' embeddings) — enough for the
    /// registry to find them, not to run them.
    pub bank_models: bool,
}

impl Default for VoicebankFixture {
    /// A single-voice English bank with the full ARPAbet set, a bundled
    /// vocoder and no dictionary.
    fn default() -> Self {
        let mut phonemes: Vec<String> = ["AP", "SP", "cl"].iter().map(|s| s.to_string()).collect();
        phonemes.extend(
            resonance_music_theory::g2p::ARPABET_PHONEMES
                .iter()
                .map(|s| s.to_string()),
        );
        Self {
            speakers: Vec::new(),
            phonemes,
            english_dict: Vec::new(),
            english_replacements: Vec::new(),
            vocoder: true,
            vocoder_name: None,
            use_tension_embed: false,
            use_energy_embed: false,
            languages: None,
            name: None,
            bank_models: false,
        }
    }
}

impl VoicebankFixture {
    /// [`Self::default`] without the phonemes in `drop`.
    pub fn without(mut self, drop: &[&str]) -> Self {
        self.phonemes.retain(|p| !drop.contains(&p.as_str()));
        self
    }

    /// A config-only stand-in for one of the shipped banks, laid out like
    /// the real one where it matters to the registry: TIGER's seven
    /// `dsacoustic/tiger_*` speakers among others, Lilia's multi-language
    /// inventory (no `v`; English spelled `L`, `t0`, … through her
    /// dictionary's replacements) and tension input, Meiji's `en/`
    /// namespace, language ids and four `embeds/*` modes.
    pub fn shipped(id: &str) -> Self {
        let arpabet = resonance_music_theory::g2p::ARPABET_PHONEMES;
        match id {
            "tiger" => Self {
                speakers: ["alex_allen", "tiger_glam", "tiger_fresh", "tiger_disco"]
                    .iter()
                    .chain(&[
                        "tiger_royal",
                        "tiger_electric",
                        "tiger_mystic",
                        "tiger_vinyl",
                    ])
                    .map(|s| format!("dsacoustic/{s}"))
                    .collect(),
                ..Self::default()
            },
            "lilia" => {
                let respelt = [
                    ("t", "t0"),
                    ("d", "d0"),
                    ("r", "r0"),
                    ("l", "ll"),
                    ("v", "w"),
                    ("w", "L"),
                    ("y", "j"),
                ];
                let mut lilia = Self::default().without(&["v"]);
                lilia
                    .phonemes
                    .extend(["L", "t0", "d0", "r0", "ll", "j"].map(str::to_owned));
                Self {
                    use_tension_embed: true,
                    english_replacements: respelt
                        .iter()
                        .map(|(a, b)| (a.to_string(), b.to_string()))
                        .collect(),
                    ..lilia
                }
            }
            "meiji" => {
                let mut phonemes: Vec<String> = ["AP", "SP", "cl", "hh"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect();
                phonemes.extend(
                    arpabet
                        .iter()
                        .filter(|p| **p != "hh")
                        .map(|p| format!("en/{p}")),
                );
                Self {
                    speakers: ["standard", "hunter", "lilith", "phantom"]
                        .iter()
                        .map(|s| format!("embeds/{s}"))
                        .collect(),
                    phonemes,
                    use_tension_embed: true,
                    languages: Some(r#"{"ja": 1, "en": 3}"#.to_owned()),
                    ..Self::default()
                }
            }
            other => panic!("no shipped fixture for {other:?}"),
        }
    }
}

/// Write a bank laid out the way OpenUtau banks are into `dir`.
pub fn write_voicebank_fixture(dir: &Path, spec: &VoicebankFixture) {
    let write = |rel: &str, body: &str| {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    };
    let mut cfg = String::from("phonemes: phonemes.txt\nacoustic: acoustic.onnx\n");
    match (&spec.vocoder_name, spec.vocoder) {
        (Some(name), false) => cfg.push_str(&format!("vocoder: {name}\n")),
        _ => cfg.push_str("vocoder: dsvocoder\n"),
    }
    if spec.use_energy_embed {
        cfg.push_str("use_energy_embed: true\n");
    }
    if spec.use_tension_embed {
        cfg.push_str("use_tension_embed: true\n");
    }
    if let Some(languages) = &spec.languages {
        cfg.push_str("use_lang_id: true\n");
        write("languages.json", languages);
    }
    if !spec.speakers.is_empty() {
        cfg.push_str("speakers:\n");
        for s in &spec.speakers {
            cfg.push_str(&format!("  - {s}\n"));
        }
    }
    write("dsconfig.yaml", &cfg);
    write("phonemes.txt", &spec.phonemes.join("\n"));
    write("acoustic.onnx", "");
    if spec.vocoder {
        write(
            "dsvocoder/vocoder.yaml",
            "name: fixture_vocoder\nmodel: vocoder.onnx\n",
        );
        write("dsvocoder/vocoder.onnx", "");
    }
    if !spec.english_dict.is_empty() || !spec.english_replacements.is_empty() {
        let mut yaml = String::from("symbols:\n- {symbol: aa, type: vowel}\nentries:\n");
        for (grapheme, phonemes) in &spec.english_dict {
            yaml.push_str(&format!(
                "- {{grapheme: \"{grapheme}\", phonemes: [{}]}}\n",
                phonemes.join(", ")
            ));
        }
        if !spec.english_replacements.is_empty() {
            yaml.push_str("replacements: # the bank's own spelling\n");
            for (from, to) in &spec.english_replacements {
                yaml.push_str(&format!("- {{from: {from}, to: {to}}}\n"));
            }
        }
        write("dsdur/dsdict-en.yaml", &yaml);
    }
    if let Some(name) = &spec.name {
        write("character.txt", &format!("name={name}\nversion=v1\n"));
    }
    if spec.bank_models {
        for (folder, key) in [("dsdur", "dur"), ("dspitch", "pitch"), ("dsvariance", "variance")] {
            let mut cfg = format!(
                "phonemes: phonemes.txt\nlinguistic: linguistic.onnx\n{key}: {key}.onnx\n\
                 hop_size: 512\nsample_rate: 44100\n"
            );
            if !spec.speakers.is_empty() {
                cfg.push_str("speakers:\n");
                for s in &spec.speakers {
                    let name = s.rsplit('/').next().unwrap_or(s);
                    cfg.push_str(&format!("  - embeds/{name}\n"));
                    let emb: Vec<u8> = (0..256).flat_map(|_| 0f32.to_le_bytes()).collect();
                    let path = dir.join(folder).join(format!("embeds/{name}.emb"));
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, emb).unwrap();
                }
            }
            write(&format!("{folder}/dsconfig.yaml"), &cfg);
            write(&format!("{folder}/phonemes.txt"), &spec.phonemes.join("\n"));
            write(&format!("{folder}/linguistic.onnx"), "");
            write(&format!("{folder}/{key}.onnx"), "");
        }
    }
}

impl Resonance {
    /// Test-only: the voicebank roots this app scans (a private folder
    /// under the hermetic root for a test app).
    #[doc(hidden)]
    pub fn test_voicebank_roots(&self) -> VoicebankRoots {
        self.voicebanks.roots().clone()
    }

    /// Test-only: where a user bank `id` goes in this app's registry.
    #[doc(hidden)]
    pub fn test_user_voicebank_dir(&self, id: &str) -> PathBuf {
        self.voicebanks
            .user_bank_dir(id)
            .expect("a test app has a user voicebank folder")
    }

    /// Test-only: write a fixture bank `id` into this app's user
    /// voicebank folder and rescan, as an install would.
    #[doc(hidden)]
    pub fn test_install_voicebank(&mut self, id: &str, spec: &VoicebankFixture) -> PathBuf {
        let dir = self.test_user_voicebank_dir(id);
        write_voicebank_fixture(&dir, spec);
        self.rescan_voicebanks();
        dir
    }

    /// Test-only: install config-only stand-ins for TIGER, Lilia and
    /// Meiji ([`VoicebankFixture::shipped`]), so a test can drive a render
    /// past the `voicebank_missing` gate. The render task itself is never
    /// run by a test (the stand-ins have no real models).
    #[doc(hidden)]
    pub fn test_install_shipped_voicebanks(&mut self) {
        for id in ["tiger", "lilia", "meiji"] {
            let dir = self.test_user_voicebank_dir(id);
            write_voicebank_fixture(&dir, &VoicebankFixture::shipped(id));
        }
        self.rescan_voicebanks();
    }

    /// Test-only: opt in to the development banks a developer pointed
    /// `RESONANCE_SVS_MODELS_DIR` at (a test app never scans them on its
    /// own). Returns whether the variable is set; when it is not, the
    /// app's banks are unchanged.
    #[doc(hidden)]
    pub fn test_install_dev_voicebanks_from_env(&mut self) -> bool {
        let Some(models) = VoicebankRoots::dev_models_from_env() else {
            return false;
        };
        let roots = VoicebankRoots {
            dev_models: vec![models],
            ..self.test_voicebank_roots()
        };
        self.test_set_voicebank_roots(roots);
        true
    }

    /// Test-only: rescan the voicebank folders.
    #[doc(hidden)]
    pub fn test_rescan_voicebanks(&mut self) {
        self.rescan_voicebanks();
    }

    /// Test-only: point this app's registry at `roots` and scan them.
    #[doc(hidden)]
    pub fn test_set_voicebank_roots(&mut self, roots: VoicebankRoots) {
        self.voicebanks = crate::compose::vocal_svs::VoicebankRegistry::scan(roots);
        self.ui
            .view_caches
            .rebuild_voicebank_choices(&self.voicebanks);
    }

    /// Test-only: the profile of bank `id` as this app sees it.
    #[doc(hidden)]
    pub fn test_voicebank_profile(&self, id: &str) -> BankProfile {
        self.voicebanks
            .profile(&resonance_music_theory::VoicebankId::new(id))
    }

    /// Test-only: the installed bank ids, in order.
    #[doc(hidden)]
    pub fn test_installed_voicebanks(&self) -> Vec<String> {
        self.voicebanks.installed().map(|b| b.id.clone()).collect()
    }

    /// Test-only: point the voicebank library at the index `url` (a local
    /// server: plain HTTP allowed, a short stall limit).
    #[doc(hidden)]
    pub fn test_set_voicebank_index(&mut self, url: &str) {
        self.voicebank_library.lib.set_config(LibraryConfig {
            index_url: Some(url.to_owned()),
            http: resonance_common::plok_download::HttpConfig {
                https_only: false,
                read_timeout: std::time::Duration::from_secs(5),
            },
            free_space: None,
        });
    }

    /// Test-only: the voicebank library (its shared state, installs).
    #[doc(hidden)]
    pub fn test_voicebank_library(&self) -> &VoicebankLibrary {
        &self.voicebank_library.lib
    }

    /// Test-only: the open voicebank library overlay.
    #[doc(hidden)]
    pub fn test_voicebank_overlay(&self) -> Option<&LibraryOverlay> {
        self.voicebank_library.overlay.as_ref()
    }

    /// Test-only: what the tick does for the library — collect finished
    /// installs (rescan, resolve their jobs) and rebuild the overlay rows.
    #[doc(hidden)]
    pub fn test_poll_voicebank_library(&mut self) {
        crate::update::voicebank_library::poll(self);
    }

    /// Test-only: show bank `id` as installing at `phase` without running
    /// anything (a frozen progress state for a golden image).
    #[doc(hidden)]
    pub fn test_show_voicebank_installing(
        &mut self,
        id: &str,
        phase: crate::compose::vocal_svs::registry::InstallPhase,
    ) {
        {
            let mut s = self.voicebank_library.lib.state();
            s.jobs.retain(|j| j.bank_id != id);
            s.jobs.push(crate::compose::vocal_svs::registry::InstallJob {
                bank_id: id.to_owned(),
                name: id.to_owned(),
                phase,
                cancel: Default::default(),
                control_job: None,
                result: None,
            });
            s.revision += 1;
        }
        self.voicebank_library_changed();
    }

    /// Test-only: move bank `id`'s running (frozen) install to `phase`, as
    /// an install thread's progress does, and run the tick's row refresh.
    #[doc(hidden)]
    pub fn test_set_voicebank_install_phase(
        &mut self,
        id: &str,
        phase: crate::compose::vocal_svs::registry::InstallPhase,
    ) {
        {
            let mut s = self.voicebank_library.lib.state();
            if let Some(job) = s
                .jobs
                .iter_mut()
                .find(|j| j.bank_id == id && j.result.is_none())
            {
                job.phase = phase;
            }
            s.progress_revision += 1;
        }
        self.test_poll_voicebank_library();
    }

    /// Test-only: poll the library until no install runs, none waits to
    /// be collected and no index fetch runs, or `timeout` passes. Returns
    /// whether it settled.
    #[doc(hidden)]
    pub fn test_settle_voicebank_library(&mut self, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            self.test_poll_voicebank_library();
            let lib = &self.voicebank_library.lib;
            let busy = lib.has_live_installs() || lib.has_finished() || lib.state().fetching_index;
            if !busy {
                return true;
            }
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

impl BankProfile {
    /// Test-only: a not-installed profile answering from `builtin` — a
    /// fixture row of the built-in table (speakers, ranges, default
    /// speaker) no shipped bank has.
    #[doc(hidden)]
    pub fn test_from_builtin(
        builtin: &'static crate::compose::vocal_svs::registry::BuiltinBank,
    ) -> Self {
        Self::from_builtin(builtin)
    }
}
