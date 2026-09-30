//! Transient view-layer/session UI state (ARCH-06 second tier, A-12e):
//! pure presentation and session-interaction bookkeeping that is never
//! persisted and never rides the undo snapshot.
//!
//! Grouped together per the A-12 survey
//! (`docs/design/A-12-resonance-fields.md`): the current tab, the tab
//! Performance mode was entered from, cached pick-list options, the
//! transport-bar label cache, mixer-panel UI state, and timeline/clip
//! interaction state, plus the report of the last arrangement bar-shift.
//! `Resonance::update_depth` — a re-entrancy guard around the `update()`
//! dispatch loop itself, not view-presentation state — is deliberately
//! left out; see its doc comment on `Resonance`.

use crate::state::ClipInteractionState;
use crate::state::MixerUiState;
use crate::state::ViewMode;

/// Transient view-layer/session UI state — never persisted, never in the
/// undo snapshot.
///
/// No `Clone` / `Default`: `ClipInteractionState` and `MixerUiState` are
/// `Debug, Default` only (no `Clone`), and `ViewMode` has no `Default` (it
/// is always given explicitly — see [`Resonance::assemble`]), matching
/// `Resonance` itself, which was never `Clone` or `Default` either.
#[derive(Debug)]
pub struct UiTransientState {
    /// Cached pick-list option lists for the view layer. Rebuilt only
    /// when source data changes (devices, busses, plugin scan) so a
    /// continuous resize doesn't reallocate option vecs every frame.
    /// See `view::ui_caches` for the cache and rebuild API.
    ///
    /// `pub(crate)`, not `pub` like this struct's other fields: `UiViewCaches`
    /// itself is `pub(crate)` (a view-layer-only cache type), so this field
    /// keeps that visibility rather than widening it.
    pub(crate) view_caches: crate::view::ui_caches::UiViewCaches,
    /// Lazy-memoised label strings for the transport bar's stat blocks
    /// (position, time, sig, key, loop). Re-formatted only when the
    /// underlying inputs change. Refreshed by `refresh_transport_labels`
    /// after every `update()` dispatch (plus at construction and after
    /// demo seeding) so `view()` only ever reads it — the view layer
    /// never mutates state. See `view::transport_labels`.
    ///
    /// `pub(crate)` for the same reason as `view_caches` above.
    pub(crate) transport_labels: crate::view::transport_labels::TransportLabels,
    /// The active tab (Arrange / Mixer / Compose / Performance).
    pub view_mode: ViewMode,
    /// The view that was active when Performance mode was entered, so
    /// exiting (`F` toggle / `Esc` / the Exit button) returns the user to
    /// where they were rather than always to Arrange. `None` whenever the
    /// current `view_mode` is not `Performance`.
    pub pre_performance_view: Option<ViewMode>,
    /// What the last `arrangement.insert_bars` / `remove_bars` moved.
    ///
    /// A bar shift touches five collections at once, so its report can
    /// only be assembled while it runs — the control handler dispatches
    /// the edit through `update()` (which is what makes it one undo
    /// entry) and reads the tally back from here. Overwritten by each
    /// shift and never persisted.
    pub last_arrangement_shift: Option<crate::update::arrangement::ShiftOutcome>,
    /// Timeline/clip selection and interaction state (drag, trim, MIDI
    /// editor, expanded-lane tracking, ...).
    pub interaction: ClipInteractionState,
    /// Mixer-panel UI state (selection, collapsed inspector groups, open
    /// menus, ...).
    pub mixer: MixerUiState,
    /// The active keymap: what every global shortcut and canvas-local key
    /// resolves through (command-palette.md §4). The Resonance default
    /// until the keymap settings land.
    pub keymap: crate::commands::BindingMap,
    /// How the typing gate asks whether a text field holds focus; always
    /// `Live` outside tests.
    pub typing_probe: crate::update::shortcuts::TypingProbe,
    /// The open command palette, if any (command-palette.md §7).
    pub palette: Option<crate::palette::PaletteState>,
    /// The query the palette closed with, restored (pre-selected) on the
    /// next open so "run it again" is ⌘K ↵.
    pub palette_memory: String,
}
