use resonance_plugin::{IntParam, IntRange, Param, TempoInfo};

pub const DIVISION_LABELS: &[&str] = &[
    "1/1", "1/2", "1/2D", "1/2T", "1/4", "1/4D", "1/4T", "1/8", "1/8D", "1/8T", "1/16", "1/16T",
];

const DIVISION_BEATS: &[f32] = &[
    4.0,       // 1/1
    2.0,       // 1/2
    3.0,       // 1/2D  (dotted)
    4.0 / 3.0, // 1/2T  (triplet)
    1.0,       // 1/4
    1.5,       // 1/4D
    2.0 / 3.0, // 1/4T
    0.5,       // 1/8
    0.75,      // 1/8D
    1.0 / 3.0, // 1/8T
    0.25,      // 1/16
    1.0 / 6.0, // 1/16T
];

/// Length of `division` in beats, clamped to the table. Shared with the
/// wet gate ([`crate::gate`]), which counts its period in the same
/// divisions the delay time uses.
pub fn division_beats(division: usize) -> f32 {
    DIVISION_BEATS[division.min(DIVISION_BEATS.len() - 1)]
}

/// Musical label for `division` ("1/8D", …), clamped to the table.
pub fn division_label(division: usize) -> &'static str {
    DIVISION_LABELS[division.min(DIVISION_LABELS.len() - 1)]
}

/// Index of a division label, matched case-insensitively. `None` for
/// anything that is not in the table.
pub fn division_from_label(text: &str) -> Option<usize> {
    let text = text.trim();
    DIVISION_LABELS
        .iter()
        .position(|l| l.eq_ignore_ascii_case(text))
}

/// An integer parameter whose value is an index into
/// [`DIVISION_LABELS`], displayed and parsed as the musical division.
///
/// The label lives on the *parameter*, not in the editor, so everything
/// that renders a parameter reads the division: the editor knob, the
/// host's automation lane (the CLAP bridge calls [`Param::display`] for
/// `param_value_to_text`), and typed-in values from a host
/// (`param_text_to_value` → [`Param::parse`]). A plain integer still
/// parses, so automation written against the raw index keeps working.
///
/// This is the local stand-in for the shared `IntParam` formatter
/// builder tracked as ba todo #1289; when that lands this type
/// collapses into `IntParam::with_value_to_string(...)`. The control
/// API only gains the text once ba todo #1290 puts a `text` field on
/// `PluginParamView` — it reads `value`/`min`/`max` today, which this
/// change leaves untouched.
pub struct DivisionParam {
    inner: IntParam,
}

impl DivisionParam {
    pub fn new(id: &'static str, name: &'static str, default: i32) -> Self {
        Self {
            inner: IntParam::new(
                id,
                name,
                default,
                IntRange::Linear {
                    min: 0,
                    max: DIVISION_LABELS.len() as i32 - 1,
                },
            ),
        }
    }

    pub fn value(&self) -> i32 {
        self.inner.value()
    }

    pub fn set_value(&self, v: i32) {
        self.inner.set_value(v);
    }
}

impl Param for DivisionParam {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn get_plain(&self) -> f64 {
        self.inner.get_plain()
    }
    fn set_plain(&self, v: f64) {
        self.inner.set_plain(v);
    }
    fn default_plain(&self) -> f64 {
        self.inner.default_plain()
    }
    fn min_plain(&self) -> f64 {
        self.inner.min_plain()
    }
    fn max_plain(&self) -> f64 {
        self.inner.max_plain()
    }
    fn display(&self, value: f64) -> String {
        if !value.is_finite() {
            return self.inner.display(value);
        }
        let clamped = value.clamp(self.min_plain(), self.max_plain());
        division_label(clamped.round() as usize).to_string()
    }
    fn parse(&self, text: &str) -> Option<f64> {
        division_from_label(text)
            .map(|i| i as f64)
            .or_else(|| self.inner.parse(text))
    }
    fn is_hidden(&self) -> bool {
        self.inner.is_hidden()
    }
    fn is_stepped(&self) -> bool {
        true
    }
}

pub fn delay_samples(
    sync: bool,
    division: usize,
    time_ms: f32,
    tempo: Option<TempoInfo>,
    sample_rate: f32,
    max_delay: f32,
) -> f32 {
    let raw = if sync {
        if let Some(t) = tempo {
            let bpm = t.bpm.max(20.0);
            let samples_per_beat = 60.0 / bpm * sample_rate;
            let div = division.min(DIVISION_BEATS.len() - 1);
            samples_per_beat * DIVISION_BEATS[div]
        } else {
            time_ms * 0.001 * sample_rate
        }
    } else {
        time_ms * 0.001 * sample_rate
    };
    raw.clamp(1.0, max_delay - 4.0)
}
