//! A stepped parameter whose values are named choices.
//!
//! The label table lives on the *parameter*, not in the editor, so
//! everything that renders a parameter reads the same words: the editor
//! chips, the host's automation lane (the CLAP bridge routes
//! `param_value_to_text` through [`Param::display`]), a typed-in value
//! from the host (`param_text_to_value` → [`Param::parse`]) and the
//! control API's parameter listing. A bare index still parses, so
//! automation written against the raw number keeps working.
//!
//! This is the drums-local stand-in for the shared
//! `IntParam::with_choices` builder added by ba todo #1289. That builder
//! lives on epic #201, which is not in this branch's ancestry; once it
//! reaches master this type collapses into
//! `IntParam::new(..).with_choices(LABELS)` with no change to any id,
//! value or displayed string. `resonance-delay`'s `DivisionParam` is the
//! same stand-in for the same reason.

use std::sync::{Arc, OnceLock};

use resonance_plugin::{IntParam, IntRange, Param};

/// Text for a choice value that the static table cannot know when the
/// parameter is built (the kit's articulation labels): `None` falls back
/// to the table.
pub type ChoiceText = Arc<dyn Fn(i32) -> Option<String> + Send + Sync>;

/// An integer parameter whose value indexes a static table of labels.
pub struct ChoiceParam {
    inner: IntParam,
    labels: &'static [&'static str],
    /// Set once, after construction ([`ChoiceParam::set_text`]).
    text: OnceLock<ChoiceText>,
}

impl ChoiceParam {
    /// Build a choice parameter over `labels`. The range is
    /// `0..=labels.len() - 1`; `labels` must not be empty.
    pub fn new(
        id: &'static str,
        name: &'static str,
        default: i32,
        labels: &'static [&'static str],
    ) -> Self {
        debug_assert!(!labels.is_empty(), "a choice param needs at least one label");
        let max = labels.len().saturating_sub(1) as i32;
        Self {
            inner: IntParam::new(id, name, default, IntRange::Linear { min: 0, max }),
            labels,
            text: OnceLock::new(),
        }
    }

    /// Have the parameter's text (display and parse) come from `text`
    /// where it gives one, the static labels elsewhere. The first call
    /// wins; returns whether this one did.
    pub fn set_text(&self, text: ChoiceText) -> bool {
        self.text.set(text).is_ok()
    }

    /// The text of value `index`: the attached text's, else the label's.
    pub fn text_at(&self, index: i32) -> String {
        self.text
            .get()
            .and_then(|text| text(index))
            .unwrap_or_else(|| self.label_at(index).to_string())
    }

    /// Hide the parameter from the host's parameter list. Used for
    /// choices that exist for id stability but cannot do anything on the
    /// current kit — the value is still addressable and still persists,
    /// it just isn't advertised as a control.
    pub fn hidden(mut self) -> Self {
        self.inner = self.inner.hidden();
        self
    }

    pub fn value(&self) -> i32 {
        self.inner.value()
    }

    pub fn set_value(&self, v: i32) {
        self.inner.set_value(v);
    }

    /// The label table, for editors that draw one control per choice.
    pub fn labels(&self) -> &'static [&'static str] {
        self.labels
    }

    /// Label of the current value.
    pub fn label(&self) -> &'static str {
        self.label_at(self.value())
    }

    fn label_at(&self, index: i32) -> &'static str {
        let i = index.max(0) as usize;
        self.labels.get(i).copied().unwrap_or("")
    }
}

impl Param for ChoiceParam {
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
        let index = value.clamp(self.min_plain(), self.max_plain()).round() as i32;
        let label = self.text_at(index);
        if label.is_empty() {
            // A value outside the table shows the number rather than
            // silently reading as some other choice.
            self.inner.display(value)
        } else {
            label
        }
    }
    fn parse(&self, text: &str) -> Option<f64> {
        let trimmed = text.trim();
        let dynamic = self.text.get().and_then(|dynamic| {
            (0..self.labels.len() as i32).find(|&i| {
                dynamic(i).is_some_and(|label| label.trim().eq_ignore_ascii_case(trimmed))
            })
        });
        if let Some(index) = dynamic {
            return Some(index as f64);
        }
        self.labels
            .iter()
            .position(|l| l.eq_ignore_ascii_case(trimmed))
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
