//! `display_pick!` macro for the verbatim "wrap an enum in a Display
//! newtype and cache `Enum::ALL` as a `OnceLock<Vec<NewType>>`" pattern.
//!
//! `pick_list` takes its option list by value (a `Borrow<[T]>` slice). The
//! inspector hosts several dropdowns whose options never change at
//! runtime, so we cache them in a process-wide static instead of
//! reallocating per frame. Each dropdown also wraps its source enum
//! (e.g. `VocalMood`) in a tiny `Display`-impl newtype (`MoodPick`) so
//! the pick_list can render the enum's `as_str` label without disturbing
//! the canonical enum's `Display` (which our generators rely on).
//!
//! The three newtype dropdowns in `vocal/common.rs` (MoodPick, PovPick,
//! VoiceTypePick) are byte-for-byte identical except for names, so this
//! macro replaces ~24 lines of boilerplate with three lines.

/// Generates a `Display`-newtype + cached `OnceLock<Vec<NewType>>`
/// options function from an enum that exposes `pub const ALL: [Self; N]`
/// and an `$accessor()` method returning `&str`.
///
/// Example:
/// ```ignore
/// display_pick!(MoodPick, VocalMood, as_str, mood_pick_options);
/// // expands to:
/// //   pub(super) struct MoodPick(pub(super) VocalMood);
/// //   impl std::fmt::Display for MoodPick { ... f.write_str(self.0.as_str()) }
/// //   pub(super) fn mood_pick_options() -> &'static [MoodPick] { ... }
/// ```
///
/// The visibility is fixed to `pub(super)` since every existing use site
/// shares the same module-private scope; lift if a wider visibility is
/// ever needed.
macro_rules! display_pick {
    ($new:ident, $inner:ty, $accessor:ident, $options_fn:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub(super) struct $new(pub(super) $inner);

        impl std::fmt::Display for $new {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0.$accessor())
            }
        }

        pub(super) fn $options_fn() -> &'static [$new] {
            static V: std::sync::OnceLock<Vec<$new>> = std::sync::OnceLock::new();
            V.get_or_init(|| <$inner>::ALL.iter().map(|x| $new(*x)).collect())
        }
    };
}

pub(super) use display_pick;
