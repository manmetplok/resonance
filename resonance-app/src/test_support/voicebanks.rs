//! Test-only access to the voicebank registry, and a fixture writer for
//! minimal on-disk banks: configs, a phoneme list, an optional English
//! dictionary and a vocoder — no real ONNX, which nothing short of a real
//! render reads (vocal-mcp-control-and-quality.md §4).

use std::path::{Path, PathBuf};

use crate::compose::vocal_svs::{BankProfile, VoicebankRoots};
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
    /// Write a bundled `dsvocoder/` (config + empty model file)?
    pub vocoder: bool,
    pub use_tension_embed: bool,
    /// `use_lang_id: true` plus this `languages.json` body.
    pub languages: Option<String>,
    /// `character.txt` name.
    pub name: Option<String>,
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
            vocoder: true,
            use_tension_embed: false,
            languages: None,
            name: None,
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
    /// `dsacoustic/tiger_*` speakers among others, Lilia's missing `v`
    /// and tension input, Meiji's `en/` namespace, language ids and four
    /// `embeds/*` modes.
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
            "lilia" => Self {
                use_tension_embed: true,
                ..Self::default().without(&["v"])
            },
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
    cfg.push_str("vocoder: dsvocoder\n");
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
    if !spec.english_dict.is_empty() {
        let mut yaml = String::from("symbols:\n- {symbol: aa, type: vowel}\nentries:\n");
        for (grapheme, phonemes) in &spec.english_dict {
            yaml.push_str(&format!(
                "- {{grapheme: \"{grapheme}\", phonemes: [{}]}}\n",
                phonemes.join(", ")
            ));
        }
        write("dsdur/dsdict-en.yaml", &yaml);
    }
    if let Some(name) = &spec.name {
        write("character.txt", &format!("name={name}\nversion=v1\n"));
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

    /// Test-only: forget the development models root a developer may
    /// have opted into with `RESONANCE_SVS_MODELS_DIR`, so the test sees
    /// only its own (initially empty) voicebank folder whatever the
    /// environment.
    #[doc(hidden)]
    pub fn test_without_dev_voicebanks(&mut self) {
        let roots = VoicebankRoots {
            dev_models: Vec::new(),
            ..self.test_voicebank_roots()
        };
        self.test_set_voicebank_roots(roots);
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
}
