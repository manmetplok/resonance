//! The seeded facet vocabulary (plugin-preset-library.md §4.4).
//!
//! Controlled facets are what browsers and agents filter on reliably; free
//! tags are everything else. Values outside these lists are accepted and
//! kept (lowercased and slugged by [`slug`]): they sort after the seeded
//! values and show in a facet list only when some item uses them. The MCP
//! schemas list the seeded values as examples, never as an enum.

/// Preset categories for instrument plugins (0..1 per preset).
pub const CATEGORIES_INSTRUMENT: &[&str] = &[
    "Bass", "Lead", "Pad", "Pluck", "Keys", "Arp", "Brass", "Strings", "Drone", "FX", "Drums",
    "Init",
];

/// Preset categories for effect plugins (0..1 per preset).
/// `Init` is shared with the instrument list: every bank's reset preset
/// uses it.
pub const CATEGORIES_EFFECT: &[&str] = &["Init", "Utility", "Track", "Bus", "Master", "Creative"];

/// "What is it for" (0..n). For an effect it names the source it suits.
pub const INSTRUMENT: &[&str] = &[
    "vocal",
    "lead-vocal",
    "backing-vocal",
    "guitar",
    "electric-guitar",
    "acoustic-guitar",
    "bass",
    "synth-bass",
    "drums",
    "kick",
    "snare",
    "hats",
    "room",
    "keys",
    "piano",
    "synth",
    "strings",
    "violin",
    "mix-bus",
    "drum-bus",
    "master",
    "full-mix",
];

/// Genres (0..n): a superset of `resonance-mastering-assist`'s `Genre` and
/// the agent plugin's genre skills.
pub const GENRES: &[&str] = &[
    "ambient",
    "americana",
    "cinematic",
    "drum-and-bass",
    "electronic",
    "folk",
    "hip-hop",
    "house",
    "indie",
    "industrial",
    "jazz",
    "metal",
    "pop",
    "post-metal",
    "rock",
    "singer-songwriter",
    "techno",
    "trance",
    "synthwave",
    "dubstep",
    "dub",
    "lo-fi",
];

/// Timbre words (0..n).
pub const CHARACTER: &[&str] = &[
    "warm",
    "bright",
    "dark",
    "clean",
    "gritty",
    "saturated",
    "punchy",
    "soft",
    "wide",
    "narrow",
    "lush",
    "dry",
    "subtle",
    "aggressive",
    "vintage",
    "modern",
    "evolving",
    "static",
    "metallic",
    "airy",
];

/// A controlled facet with a seeded vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Facet {
    Instrument,
    Genres,
    Character,
}

impl Facet {
    pub const ALL: [Facet; 3] = [Facet::Instrument, Facet::Genres, Facet::Character];

    /// The field name used in files, query tokens and MCP schemas.
    pub fn name(self) -> &'static str {
        match self {
            Facet::Instrument => "instrument",
            Facet::Genres => "genres",
            Facet::Character => "character",
        }
    }

    pub fn from_name(name: &str) -> Option<Facet> {
        Facet::ALL.into_iter().find(|f| f.name() == name)
    }

    /// The seeded values, in display order.
    pub fn seeded(self) -> &'static [&'static str] {
        match self {
            Facet::Instrument => INSTRUMENT,
            Facet::Genres => GENRES,
            Facet::Character => CHARACTER,
        }
    }

    /// Where `value` sits in the seeded list (`None` for a user value),
    /// so seeded values sort first in their declared order.
    pub fn seeded_rank(self, value: &str) -> Option<usize> {
        self.seeded().iter().position(|v| *v == value)
    }
}

/// Every seeded facet value (instrument, genres, character), for tag
/// completion.
pub fn all_seeded() -> impl Iterator<Item = &'static str> {
    Facet::ALL.into_iter().flat_map(|f| f.seeded().iter().copied())
}

/// Normalise a facet value the way free tags are: lowercase `[a-z0-9-]`.
/// `"Drum & Bass"` → `"drum-bass"`, `"Shoegaze"` → `"shoegaze"`.
pub fn slug(raw: &str) -> Option<String> {
    super::normalize_tag(raw)
}

/// Replace the common Latin accented letters with their ASCII base, so
/// search and slugs are accent-insensitive (`"Café"` matches `"cafe"`).
/// Characters with no mapping pass through unchanged.
pub fn fold_accents(text: &str) -> std::borrow::Cow<'_, str> {
    if text.is_ascii() {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match fold_char(ch) {
            Some(s) => out.push_str(s),
            None => out.push(ch),
        }
    }
    std::borrow::Cow::Owned(out)
}

fn fold_char(ch: char) -> Option<&'static str> {
    Some(match ch {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => "a",
        'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' => "A",
        'æ' => "ae",
        'Æ' => "AE",
        'ç' | 'ć' | 'č' => "c",
        'Ç' | 'Ć' | 'Č' => "C",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => "e",
        'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ę' | 'Ě' => "E",
        'ì' | 'í' | 'î' | 'ï' | 'ī' => "i",
        'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' => "I",
        'ñ' | 'ń' | 'ň' => "n",
        'Ñ' | 'Ń' | 'Ň' => "N",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => "o",
        'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' => "O",
        'œ' => "oe",
        'Œ' => "OE",
        'ß' => "ss",
        'š' | 'ś' => "s",
        'Š' | 'Ś' => "S",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' => "u",
        'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' | 'Ů' => "U",
        'ý' | 'ÿ' => "y",
        'Ý' | 'Ÿ' => "Y",
        'ž' | 'ź' | 'ż' => "z",
        'Ž' | 'Ź' | 'Ż' => "Z",
        'ł' => "l",
        'Ł' => "L",
        _ => return None,
    })
}
