//! One definition per choice parameter (ba todo #1267).
//!
//! Every `IntParam` that selects a mode used to be described in three
//! unrelated places: the declared range in `params.rs`, an inline
//! `match` from index to enum in `lib.rs`, and a label list in the
//! editor. Nothing tied them together, so reordering labels or widening
//! a range silently desynced what the UI said from what the DSP did.
//!
//! [`choice_param!`] generates the single mapping instead: the variant
//! order *is* the parameter's index order *is* the label order. The
//! editor's cached static slices come off [`ChoiceParam::LABELS`], the
//! audio path resolves indices through [`ChoiceParam::from_index`], and
//! `tests/editor_groups.rs` asserts the generated tables against each
//! parameter's declared range.

/// The generated mapping every choice parameter's enum implements.
///
/// Implemented through [`choice_param!`]; the trait exists so tests and
/// the editor can talk about "a choice parameter" generically.
pub trait ChoiceParam: Copy + PartialEq + Sized + 'static {
    /// Every variant, in parameter-index order.
    const ALL: &'static [Self];
    /// Display label per variant, same order as [`Self::ALL`].
    const LABELS: &'static [&'static str];
    /// Variant an out-of-range index resolves to on the audio path.
    /// `IntParam::set_value` does not clamp, so this is reachable — it
    /// is pinned to whatever the plugin's original catch-all arm
    /// produced, so widening a range is the only way to change it.
    const FALLBACK: Self;

    /// Exact mapping: `None` for an index outside the declared range.
    fn try_from_index(index: i32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|i| Self::ALL.get(i))
            .copied()
    }

    /// Total mapping used by the audio path: out-of-range resolves to
    /// [`Self::FALLBACK`].
    fn from_index(index: i32) -> Self {
        Self::try_from_index(index).unwrap_or(Self::FALLBACK)
    }

    /// This variant's parameter index.
    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|v| *v == self)
            .expect("ALL lists every variant")
    }

    /// This variant's display label.
    fn label(self) -> &'static str {
        Self::LABELS[self.index()]
    }
}

/// Declare the index → variant → label mapping of one choice parameter.
///
/// ```ignore
/// choice_param!(TimeMode, fallback: TimeMode::PerGrain, {
///     TimeMode::Fade => "Fade",
///     TimeMode::Repitch => "Repitch",
///     TimeMode::PerGrain => "Grain",
/// });
/// ```
///
/// The listing order is the parameter's integer order, so the range
/// declared in `params.rs` is `0..=(n - 1)`.
macro_rules! choice_param {
    ($ty:ty, fallback: $fallback:expr, { $($variant:expr => $label:literal),+ $(,)? }) => {
        impl $crate::choice::ChoiceParam for $ty {
            const ALL: &'static [Self] = &[$($variant),+];
            const LABELS: &'static [&'static str] = &[$($label),+];
            const FALLBACK: Self = $fallback;
        }
    };
}

pub(crate) use choice_param;
