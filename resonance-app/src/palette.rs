//! The command palette's model (command-palette.md §7): its state, the
//! messages that drive it, and the ranked, sectioned result list built from
//! the command registry.
//!
//! Results are rebuilt in the reducer whenever the query or the selection
//! changes and stored on [`PaletteState`], so `view()` only reads them (the
//! view-layer rule: nothing allocates a list per frame).

use crate::commands::{fuzzy_match, Available, CommandCategory, CommandId, KeyChord};
use crate::state::ViewMode;
use crate::Resonance;

/// How many commands *Recent* remembers.
pub const RECENT_LIMIT: usize = 8;

/// The text input's widget id, for programmatic focus.
pub fn query_input_id() -> iced::widget::Id {
    iced::widget::Id::new("command-palette-query")
}

/// The result list's scrollable id, for keeping the selection in view.
pub fn list_id() -> iced::widget::Id {
    iced::widget::Id::new("command-palette-list")
}

/// Which data source the palette opens on. The mode is carried by the
/// query's leading prefix character, so opening in a mode just seeds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteMode {
    Commands,
}

impl PaletteMode {
    /// The query a palette opened in this mode starts with.
    pub fn prefix(self) -> &'static str {
        match self {
            PaletteMode::Commands => "",
        }
    }
}

/// Palette interaction, routed as `UiMessage::Palette`.
#[derive(Debug, Clone)]
pub enum PaletteMsg {
    /// The query text changed.
    Query(String),
    /// ↑ / ↓: move the selection by `delta` rows, wrapping around.
    Move(i32),
    /// ↵: run the selected row.
    Submit,
    /// The pointer entered row `index`.
    Hover(usize),
    /// Row `index` was clicked.
    Click(usize),
    /// The result list scrolled (keeps the selection-follow offset honest).
    Scrolled(f32),
}

/// What a row runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaletteItem {
    Command(CommandId),
}

/// One result row, fully resolved for drawing.
#[derive(Debug, Clone)]
pub struct PaletteRow {
    pub item: PaletteItem,
    pub name: String,
    /// Matched `[start, end)` char ranges in `name`, for highlighting.
    pub ranges: Vec<(usize, usize)>,
    pub breadcrumb: String,
    pub glyph: Option<char>,
    pub chord: Option<KeyChord>,
    /// `Some(reason)` when the row can't run right now (drawn dimmed).
    pub unavailable: Option<&'static str>,
}

/// A titled group of rows.
#[derive(Debug, Clone)]
pub struct PaletteSection {
    pub title: String,
    pub rows: Vec<PaletteRow>,
}

/// The open palette.
#[derive(Debug, Clone, Default)]
pub struct PaletteState {
    pub query: String,
    /// Index into the flattened rows.
    pub selected: usize,
    /// The reason flashed after ↵ on an unavailable row, until the next
    /// interaction.
    pub flash: Option<&'static str>,
    pub sections: Vec<PaletteSection>,
    /// The list's current vertical scroll offset, in pixels.
    pub scroll_y: f32,
}

impl PaletteState {
    /// Every row, in display order.
    pub fn rows(&self) -> impl Iterator<Item = &PaletteRow> {
        self.sections.iter().flat_map(|s| s.rows.iter())
    }

    pub fn row_count(&self) -> usize {
        self.sections.iter().map(|s| s.rows.len()).sum()
    }

    pub fn selected_row(&self) -> Option<&PaletteRow> {
        self.rows().nth(self.selected)
    }
}

/// The short static "Suggested for this view" list (§7.2).
pub fn suggested(view: ViewMode) -> &'static [CommandId] {
    use CommandId::*;
    match view {
        ViewMode::Arrange => &[TransportToggleLoop, SetLoopStartAtPlayhead, OpenAddTrackMenu],
        ViewMode::Mixer => &[AddBus, ToggleMasterFxBypass, BounceToWav],
        ViewMode::Compose => &[ComposeCreateSection, LoopSectionAtPlayhead],
        ViewMode::Performance => &[TransportTogglePlay, ExitPerformanceMode],
    }
}

/// The recent commands, newest first (unknown keys dropped).
pub fn recent(r: &Resonance) -> Vec<CommandId> {
    r.settings
        .palette
        .recent
        .iter()
        .filter_map(|k| CommandId::from_key(k))
        .collect()
}

/// Remember `command` as run: move it to the front of *Recent*.
pub fn record_recent(r: &mut Resonance, command: CommandId) {
    let recent = &mut r.settings.palette.recent;
    let key = command.key();
    if recent.first().is_some_and(|k| k == key) {
        return;
    }
    recent.retain(|k| k != key);
    recent.insert(0, key.to_string());
    recent.truncate(RECENT_LIMIT);
    crate::settings::persist(&r.settings);
}

fn command_row(r: &Resonance, id: CommandId, ranges: Vec<(usize, usize)>) -> PaletteRow {
    PaletteRow {
        item: PaletteItem::Command(id),
        name: id.display_name().to_string(),
        ranges,
        breadcrumb: id.breadcrumb(),
        glyph: id.glyph(),
        chord: r.ui.keymap.chord_for(id),
        unavailable: match id.availability(r) {
            Available::Yes => None,
            Available::No(reason) => Some(reason),
        },
    }
}

/// How well `query` matches `id`: the name at full weight, the keywords at
/// half. The highlight ranges come from a name match only.
pub fn score(query: &str, id: CommandId) -> Option<(i32, Vec<(usize, usize)>)> {
    let name = fuzzy_match(query, id.display_name());
    let keyword = id
        .keywords()
        .iter()
        .filter_map(|k| fuzzy_match(query, k))
        .map(|m| m.score / 2)
        .max();
    match (name, keyword) {
        (Some(n), Some(k)) if k > n.score => Some((k, Vec::new())),
        (Some(n), _) => Some((n.score, n.ranges)),
        (None, Some(k)) => Some((k, Vec::new())),
        (None, None) => None,
    }
}

/// Build the palette's sections for `query` (§7.2).
pub fn build(r: &Resonance, query: &str) -> Vec<PaletteSection> {
    if query.trim().is_empty() {
        return build_empty(r);
    }
    let recent = recent(r);
    let recent_rank = |id: CommandId| recent.iter().position(|&c| c == id).unwrap_or(usize::MAX);
    let mut hits: Vec<(i32, usize, bool, usize, CommandId, Vec<(usize, usize)>)> = CommandId::ALL
        .iter()
        .enumerate()
        .filter_map(|(order, &id)| {
            let (score, ranges) = score(query, id)?;
            let available = id.availability(r).is_yes();
            Some((score, recent_rank(id), available, order, id, ranges))
        })
        .collect();
    // Score, then recent use, then available first, then registry order.
    hits.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.cmp(&b.1))
            .then(b.2.cmp(&a.2))
            .then(a.3.cmp(&b.3))
    });
    // Group by category; a category sits where its best hit ranks.
    let mut sections: Vec<(CommandCategory, PaletteSection)> = Vec::new();
    for (_, _, _, _, id, ranges) in hits {
        let category = id.category();
        let row = command_row(r, id, ranges);
        match sections.iter_mut().find(|(c, _)| *c == category) {
            Some((_, section)) => section.rows.push(row),
            None => sections.push((
                category,
                PaletteSection {
                    title: category.display_name().to_string(),
                    rows: vec![row],
                },
            )),
        }
    }
    sections.into_iter().map(|(_, s)| s).collect()
}

/// The empty-query state: *Recent*, then *Suggested for this view*.
fn build_empty(r: &Resonance) -> Vec<PaletteSection> {
    let recent = recent(r);
    let mut sections = Vec::new();
    if !recent.is_empty() {
        sections.push(PaletteSection {
            title: "Recent".to_string(),
            rows: recent.iter().map(|&id| command_row(r, id, Vec::new())).collect(),
        });
    }
    let suggested: Vec<PaletteRow> = suggested(r.ui.view_mode)
        .iter()
        .filter(|id| !recent.contains(id))
        .map(|&id| command_row(r, id, Vec::new()))
        .collect();
    if !suggested.is_empty() {
        sections.push(PaletteSection {
            title: "Suggested for this view".to_string(),
            rows: suggested,
        });
    }
    sections
}
