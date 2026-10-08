//! [`BankProfile`]: one bank id's facts, whether or not it is installed —
//! what the render, the pronunciation gate, the vocal roll and the control
//! readouts ask instead of matching on a bank enum.
//!
//! Each answer comes from the bank's files when it is installed (curve
//! flags, phoneme inventory, language ids, speakers), from the built-in
//! table's record of the shipped configs when it is one of the known banks
//! but not installed, and from conservative defaults otherwise (full
//! ARPAbet, no curves but pitch bend, a wide comfortable range).

use std::path::PathBuf;
use std::sync::Arc;

use resonance_music_theory::g2p::{self, PhonemeDictionary};
use resonance_music_theory::{VoiceType, VoicebankId};
use resonance_svs::predict::BankModelPaths;
use resonance_svs::voicebank::{CurveSupport, SingerInfo};

use super::builtin::{self, BuiltinBank, ShippedSpelling, DEFAULT_RANGE};
use super::InstalledBank;
use crate::compose::vocal_svs::CurveKind;

/// Pipeline control tokens every bank takes, whatever its inventory says.
const CONTROL_TOKENS: [&str; 3] = ["AP", "SP", "cl"];

/// A bank counts as English-capable when it lacks at most this many of
/// the 40 ARPAbet phones (directly or `en/`-prefixed). A Japanese or
/// X-SAMPA bank lacks most of them.
const ENGLISH_MAX_MISSING: usize = 8;

/// One speaker (voice mode) a bank offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpeakerInfo {
    /// Wire / project id (`disco`, `phantom`).
    pub id: String,
    /// Display name (`Disco`).
    pub name: String,
    /// The acoustic config's `speakers` entry the render passes
    /// (`dsacoustic/tiger_disco`, `embeds/phantom`).
    pub model_key: String,
}

/// Where a comfortable range comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeSource {
    /// The built-in table's estimate for this bank.
    Builtin,
    /// Nothing is known about the bank: the wide default.
    Default,
}

impl RangeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RangeSource::Builtin => "builtin",
            RangeSource::Default => "default",
        }
    }
}

/// The MIDI range a bank sings clearly, inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComfortableRange {
    pub low: u8,
    pub high: u8,
    pub source: RangeSource,
}

/// What the render needs to load a bank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderTarget {
    pub acoustic_config: PathBuf,
    pub vocoder_config: PathBuf,
    /// The model speaker key, `None` for a bank without speakers.
    pub speaker: Option<String>,
}

/// A render or a bank switch that names a bank (or its vocoder) this
/// machine does not have (§4.6). Its message names what is missing and
/// where to get it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicebankMissing {
    pub bank: String,
    /// Set when the bank is installed but the vocoder it names is not.
    pub vocoder: Option<String>,
    /// Where a user bank with this id goes.
    pub install_dir: Option<PathBuf>,
}

impl std::fmt::Display for VoicebankMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(vocoder) = &self.vocoder {
            return write!(
                f,
                "voicebank {:?} is installed but its vocoder {vocoder:?} is not; install the \
                 vocoder into the vocoder library (vocal_voicebank_install fetches it with \
                 its bank)",
                self.bank
            );
        }
        write!(
            f,
            "voicebank {:?} is not installed; install it (vocal_voicebank_install, or the \
             voicebank library",
            self.bank
        )?;
        if let Some(dir) = &self.install_dir {
            write!(f, ": a bank folder at {}", dir.display())?;
        }
        f.write_str(") or pick an installed bank (vocal_voicebanks lists them)")
    }
}

/// One bank id's facts. Cheap to clone (an `Arc` and two words) and
/// `Send`, so the render thread takes its own.
#[derive(Debug, Clone)]
pub struct BankProfile {
    id: VoicebankId,
    installed: Option<Arc<InstalledBank>>,
    builtin: Option<&'static BuiltinBank>,
}

impl BankProfile {
    /// A not-installed profile answering from `builtin` — a fixture
    /// table in tests (`Resonance`-free, see `test_support`).
    pub(crate) fn from_builtin(builtin: &'static BuiltinBank) -> Self {
        Self {
            id: VoicebankId::new(builtin.id),
            installed: None,
            builtin: Some(builtin),
        }
    }

    pub(super) fn new(id: VoicebankId, installed: Option<Arc<InstalledBank>>) -> Self {
        let builtin = builtin::builtin(id.as_str());
        Self {
            id,
            installed,
            builtin,
        }
    }

    /// The profile of bank `id` as if nothing were installed: the
    /// built-in table's facts, or the defaults. What a hermetic test
    /// renders segments against.
    pub fn not_installed(id: &str) -> Self {
        Self::new(VoicebankId::new(id), None)
    }

    pub fn id(&self) -> &VoicebankId {
        &self.id
    }

    pub fn installed(&self) -> Option<&InstalledBank> {
        self.installed.as_deref()
    }

    pub fn builtin(&self) -> Option<&'static BuiltinBank> {
        self.builtin
    }

    pub fn is_installed(&self) -> bool {
        self.installed.is_some()
    }

    pub fn name(&self) -> String {
        match (&self.installed, self.builtin) {
            (Some(bank), _) => bank.name.clone(),
            (None, Some(b)) => b.name.to_owned(),
            (None, None) => self.id.to_string(),
        }
    }

    /// Which expression curves the acoustic model takes.
    pub fn curve_support(&self) -> CurveSupport {
        match (&self.installed, self.builtin) {
            (Some(bank), _) => bank.curves,
            (None, Some(b)) => b.shipped_curves,
            (None, None) => CurveSupport::default(),
        }
    }

    /// Whether `curve` reaches the model. Pitch bend is an f0 edit and
    /// always applies.
    pub fn curve_supported(&self, curve: CurveKind) -> bool {
        let support = self.curve_support();
        match curve {
            CurveKind::PitchBend => true,
            CurveKind::Dynamics => support.energy,
            CurveKind::Tension => support.tension,
            CurveKind::Breathiness => support.breathiness,
        }
    }

    /// Whether the bank sings ARPAbet phone `ph` as itself (bare or in its
    /// `en/` namespace). The control tokens `AP`, `SP` and `cl` always.
    pub fn has_phoneme(&self, ph: &str) -> bool {
        if CONTROL_TOKENS.contains(&ph) {
            return true;
        }
        match (&self.installed, self.builtin) {
            (Some(bank), _) => bank.inventory.dict_key_for(ph).is_some(),
            (None, Some(b)) => !b.shipped_missing.contains(&ph),
            (None, None) => true,
        }
    }

    /// The ARPAbet phones the bank lacks, in canonical order.
    pub fn missing_phonemes(&self) -> Vec<&'static str> {
        g2p::ARPABET_PHONEMES
            .iter()
            .copied()
            .filter(|ph| !self.has_phoneme(ph))
            .collect()
    }

    /// Whether the bank can sing English at all (see
    /// [`ENGLISH_MAX_MISSING`]). Lyrics are English, so a bank that
    /// cannot is listed but refused by `vocal.set_params`.
    pub fn english(&self) -> bool {
        self.missing_phonemes().len() <= ENGLISH_MAX_MISSING
    }

    /// How far the timbre chip moves this bank's `gender` input (see
    /// [`BuiltinBank::gender_scale`]); 1 for a bank the table does not know.
    pub fn gender_scale(&self) -> f32 {
        self.builtin.map_or(1.0, |b| b.gender_scale)
    }

    /// The built-in substitution hints for `ph` (none of the shipped banks
    /// needs one today).
    pub fn substitution_hints(&self, ph: &str) -> &'static [&'static str] {
        self.builtin
            .and_then(|b| b.substitutions.iter().find(|(from, _)| *from == ph))
            .map(|(_, to)| *to)
            .unwrap_or(&[])
    }

    /// The symbol the acoustic model's dictionary stores ARPAbet `ph`
    /// under: what the bank dictionary's replacements respell it as (Lilia
    /// `w` → `L`), else bare, or `en/`-prefixed in a bank that namespaces
    /// English.
    pub fn phoneme_name(&self, ph: &str) -> String {
        match (&self.installed, self.builtin) {
            (Some(bank), _) => bank.inventory.phoneme_name(ph),
            (None, Some(b)) if let Some((_, to)) =
                b.shipped_replacements.iter().find(|(from, _)| *from == ph) =>
            {
                (*to).to_owned()
            }
            (None, Some(b)) => match b.shipped_spelling {
                ShippedSpelling::Namespaced { .. } if !is_universal(ph) => format!("en/{ph}"),
                _ => ph.to_owned(),
            },
            (None, None) => ph.to_owned(),
        }
    }

    /// The per-token language id for `ph`, or `None` when the model takes
    /// no language input. A namespaced key reports its language's id from
    /// the bank's `languages.json` (`en/ah` → Meiji's `en`, 3); a bare
    /// symbol reports 0 in a bank that namespaces English (its universal
    /// bucket: `AP`, `hh`, …) and the bank's id for `en` otherwise.
    pub fn language_id(&self, ph: &str) -> Option<i64> {
        match (&self.installed, self.builtin) {
            (Some(bank), _) => {
                if !bank.accepts_language_id {
                    return None;
                }
                let key = bank.inventory.phoneme_name(ph);
                Some(match key.split_once('/') {
                    Some((lang, _)) => bank.languages.get(lang).copied().unwrap_or(0),
                    None if CONTROL_TOKENS.contains(&ph) || bank.namespaces_english() => 0,
                    None => bank.languages.get("en").copied().unwrap_or(0),
                })
            }
            (None, Some(b)) => match b.shipped_spelling {
                ShippedSpelling::Namespaced { en_id } => {
                    Some(if is_universal(ph) { 0 } else { en_id })
                }
                ShippedSpelling::Bare => None,
            },
            (None, None) => None,
        }
    }

    /// The speakers the bank offers: the installed bank's (filtered by the
    /// built-in allow-list), else the built-in list.
    pub fn speakers(&self) -> Vec<SpeakerInfo> {
        match (&self.installed, self.builtin) {
            (Some(bank), _) => bank.speakers.clone(),
            (None, Some(b)) => b
                .speakers
                .iter()
                .map(|s| SpeakerInfo {
                    id: s.id.to_owned(),
                    name: s.name.to_owned(),
                    model_key: s.model_key.to_owned(),
                })
                .collect(),
            (None, None) => Vec::new(),
        }
    }

    /// The speaker a lane gets on switching to this bank: the built-in
    /// default (TIGER's follows the voice type), else the first speaker.
    /// `None` for a bank with at most one speaker.
    pub fn default_speaker(&self, voice: VoiceType) -> Option<String> {
        let speakers = self.speakers();
        if speakers.len() <= 1 {
            return None;
        }
        self.builtin
            .and_then(|b| (b.default_speaker)(voice))
            .filter(|id| speakers.iter().any(|s| s.id == *id))
            .map(str::to_owned)
            .or_else(|| speakers.first().map(|s| s.id.clone()))
    }

    /// Whether `speaker` is one of the bank's speaker ids.
    pub fn has_speaker(&self, speaker: &str) -> bool {
        self.speakers().iter().any(|s| s.id == speaker)
    }

    /// The speaker a lane with `speaker` (its own pick, if any) and
    /// `voice` sings with: its pick, else the bank's default for the
    /// voice. What the render loads and the range check reads.
    pub fn lane_speaker(&self, speaker: Option<&str>, voice: VoiceType) -> Option<String> {
        speaker
            .map(str::to_owned)
            .or_else(|| self.default_speaker(voice))
    }

    /// The comfortable range of the speaker a lane sings with
    /// ([`Self::lane_speaker`]): a lane that never picked one is checked
    /// against the default speaker's range, the one it is rendered with.
    pub fn lane_comfortable_range(
        &self,
        speaker: Option<&str>,
        voice: VoiceType,
    ) -> ComfortableRange {
        self.comfortable_range(self.lane_speaker(speaker, voice).as_deref())
    }

    /// The comfortable range for `speaker`: the built-in table's range for
    /// that speaker where it knows one, else the bank's. `None` is the
    /// bank's own range; a lane's goes through
    /// [`Self::lane_comfortable_range`].
    pub fn comfortable_range(&self, speaker: Option<&str>) -> ComfortableRange {
        let (range, source) = match self.builtin {
            Some(b) => {
                let per_speaker = speaker.and_then(|id| {
                    b.speaker_ranges
                        .iter()
                        .find(|(s, _)| *s == id)
                        .map(|(_, r)| *r)
                });
                (per_speaker.unwrap_or(b.range), RangeSource::Builtin)
            }
            None => (DEFAULT_RANGE, RangeSource::Default),
        };
        ComfortableRange {
            low: range.0,
            high: range.1,
            source,
        }
    }

    /// The bank's own duration / pitch / variance models it ships (none
    /// when it is not installed).
    pub fn bank_models(&self) -> BankModelPaths {
        self.installed
            .as_ref()
            .map(|b| b.models.clone())
            .unwrap_or_default()
    }

    /// The bank models a lane with `bank_timing` / `bank_pitch` renders
    /// with: the duration model for timing, the pitch model for pitch, and
    /// — with pitch — the variance model when the acoustic model takes a
    /// curve it predicts (tension on Lilia and Meiji). Each `None` where
    /// the flag is off or the bank does not ship the model.
    pub fn active_bank_models(&self, bank_timing: bool, bank_pitch: bool) -> BankModelPaths {
        let all = self.bank_models();
        let support = self.curve_support();
        let accepts_variance = support.tension || support.energy || support.breathiness;
        BankModelPaths {
            duration: all.duration.filter(|_| bank_timing),
            pitch: all.pitch.clone().filter(|_| bank_pitch),
            variance: all
                .variance
                .filter(|_| bank_pitch && all.pitch.is_some() && accepts_variance),
        }
    }

    /// The bank's English dictionary, when it is installed and ships one.
    pub fn dictionary(&self) -> Option<Arc<PhonemeDictionary>> {
        self.installed.as_ref()?.dictionary()
    }

    /// The files a render of this bank loads, with `speaker` (or the
    /// default for `voice`) resolved to its model key — or what is missing.
    pub fn render_target(
        &self,
        speaker: Option<&str>,
        voice: VoiceType,
        install_dir: Option<PathBuf>,
    ) -> Result<RenderTarget, VoicebankMissing> {
        let missing = |vocoder: Option<String>| VoicebankMissing {
            bank: self.id.to_string(),
            vocoder,
            install_dir: install_dir.clone(),
        };
        let Some(bank) = &self.installed else {
            return Err(missing(None));
        };
        let Some(vocoder_config) = bank.vocoder_config.clone() else {
            return Err(missing(Some(
                bank.vocoder_name
                    .clone()
                    .unwrap_or_else(|| "(unnamed)".to_owned()),
            )));
        };
        let wanted = self.lane_speaker(speaker, voice);
        let key = wanted
            .and_then(|id| bank.speakers.iter().find(|s| s.id == id))
            .or_else(|| bank.speakers.first())
            .map(|s| s.model_key.clone());
        Ok(RenderTarget {
            acoustic_config: bank.acoustic_config.clone(),
            vocoder_config,
            speaker: key,
        })
    }
}

/// Meiji's bare bucket: silence, closure and the shared consonants it
/// keeps outside every language namespace.
fn is_universal(ph: &str) -> bool {
    matches!(ph, "AP" | "SP" | "hh" | "cl" | "ban" | "vf")
}

/// The speakers to offer for a scanned bank: the built-in allow-list
/// matched against the model's `speakers` (by last path segment), or —
/// for a bank the table does not list speakers for — every model speaker,
/// its id the slug of its last path segment.
pub(super) fn offered_speakers(
    builtin: Option<&'static BuiltinBank>,
    singers: &[SingerInfo],
) -> Vec<SpeakerInfo> {
    let last = |key: &str| key.rsplit('/').next().unwrap_or(key).to_owned();
    match builtin.filter(|b| !b.speakers.is_empty()) {
        Some(b) => b
            .speakers
            .iter()
            .filter_map(|s| {
                let singer = singers.iter().find(|m| last(&m.id) == s.model_key)?;
                Some(SpeakerInfo {
                    id: s.id.to_owned(),
                    name: s.name.to_owned(),
                    model_key: singer.id.clone(),
                })
            })
            .collect(),
        None => {
            let mut out: Vec<SpeakerInfo> = Vec::new();
            for singer in singers {
                let id = resonance_svs::voicebank::layout::slugify(&last(&singer.id));
                if id.is_empty() || out.iter().any(|s| s.id == id) {
                    continue;
                }
                out.push(SpeakerInfo {
                    name: title_case(&id),
                    id,
                    model_key: singer.id.clone(),
                });
            }
            out
        }
    }
}

fn title_case(id: &str) -> String {
    id.split('-')
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}
