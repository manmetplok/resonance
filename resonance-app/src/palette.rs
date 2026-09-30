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
    /// The command registry (no prefix).
    Commands,
    /// `:` — go to a bar (`17`) or a beat (`17.3`).
    GoToBar,
    /// `@` — jump to a marker or a section placement.
    Jump,
    /// `#` — select a track.
    Tracks,
    /// `+` — add a plugin to the selected track.
    Plugins,
}

impl PaletteMode {
    /// The query a palette opened in this mode starts with.
    pub fn prefix(self) -> &'static str {
        match self {
            PaletteMode::Commands => "",
            PaletteMode::GoToBar => ":",
            PaletteMode::Jump => "@",
            PaletteMode::Tracks => "#",
            PaletteMode::Plugins => "+",
        }
    }

    /// The mode a query's leading character selects, and the rest of it.
    pub fn of(query: &str) -> (PaletteMode, &str) {
        for mode in [
            PaletteMode::GoToBar,
            PaletteMode::Jump,
            PaletteMode::Tracks,
            PaletteMode::Plugins,
        ] {
            if let Some(rest) = query.strip_prefix(mode.prefix()) {
                return (mode, rest);
            }
        }
        (PaletteMode::Commands, query)
    }
}

/// Parse the `:` mode's argument: `17` is bar 17, `17.3` its third beat
/// (both 1-based).
pub fn parse_bar(text: &str) -> Option<(u32, u32)> {
    let text = text.trim();
    let (bar, beat) = match text.split_once('.') {
        Some((bar, beat)) => (bar, Some(beat)),
        None => (text, None),
    };
    let bar: u32 = bar.trim().parse().ok().filter(|&b| b >= 1)?;
    let beat: u32 = match beat {
        Some(b) => b.trim().parse().ok().filter(|&b| b >= 1)?,
        None => 1,
    };
    Some((bar, beat))
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
    /// `:` — seek to a 1-based bar and beat.
    GoTo { bar: u32, beat: u32 },
    /// `@` — an arrangement marker.
    Marker(u64),
    /// `@` — a section placement, by its start sample.
    Section { start: u64 },
    /// `#` — a track.
    Track(resonance_audio::types::TrackId),
    /// `+` — a plugin, by index into the scanned catalog.
    Plugin(usize),
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
        ViewMode::Arrange => &[TransportToggleLoop, SplitClipAtPlayhead, OpenAddTrackMenu],
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

/// Build the palette's sections for `query` (§7.2, §7.4).
pub fn build(r: &Resonance, query: &str) -> Vec<PaletteSection> {
    let (mode, rest) = PaletteMode::of(query);
    match mode {
        PaletteMode::Commands => {}
        PaletteMode::GoToBar => return build_goto(rest),
        PaletteMode::Jump => return build_jump(r, rest),
        PaletteMode::Tracks => return build_tracks(r, rest),
        PaletteMode::Plugins => return build_plugins(r, rest),
    }
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

fn item_row(item: PaletteItem, name: String, ranges: Vec<(usize, usize)>, crumb: String, glyph: char) -> PaletteRow {
    PaletteRow {
        item,
        name,
        ranges,
        breadcrumb: crumb,
        glyph: Some(glyph),
        chord: None,
        unavailable: None,
    }
}

fn one_section(title: &str, rows: Vec<PaletteRow>) -> Vec<PaletteSection> {
    if rows.is_empty() {
        return Vec::new();
    }
    vec![PaletteSection {
        title: title.to_string(),
        rows,
    }]
}

/// `:` — one row, parsed live.
fn build_goto(rest: &str) -> Vec<PaletteSection> {
    use crate::theme::fa;
    let row = match parse_bar(rest) {
        Some((bar, 1)) if !rest.contains('.') => item_row(
            PaletteItem::GoTo { bar, beat: 1 },
            format!("Go to bar {bar}"),
            Vec::new(),
            "Transport › Playhead".to_string(),
            fa::FORWARD_STEP,
        ),
        Some((bar, beat)) => item_row(
            PaletteItem::GoTo { bar, beat },
            format!("Go to bar {bar}, beat {beat}"),
            Vec::new(),
            "Transport › Playhead".to_string(),
            fa::FORWARD_STEP,
        ),
        None => PaletteRow {
            unavailable: Some("Type a bar, e.g. 17 or 17.3"),
            ..item_row(
                PaletteItem::GoTo { bar: 1, beat: 1 },
                "Go to bar…".to_string(),
                Vec::new(),
                "Transport › Playhead".to_string(),
                fa::FORWARD_STEP,
            )
        },
    };
    one_section("Go to", vec![row])
}

/// Rows whose `name` fuzzy-matches `needle`, best first, stable on ties.
fn filtered(needle: &str, mut rows: Vec<PaletteRow>) -> Vec<PaletteRow> {
    let mut scored: Vec<(i32, usize, PaletteRow)> = rows
        .drain(..)
        .enumerate()
        .filter_map(|(i, mut row)| {
            let m = fuzzy_match(needle, &row.name)?;
            row.ranges = m.ranges;
            Some((m.score, i, row))
        })
        .collect();
    if !needle.trim().is_empty() {
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    scored.into_iter().map(|(_, _, row)| row).collect()
}

/// `@` — markers and section placements, in timeline order.
fn build_jump(r: &Resonance, rest: &str) -> Vec<PaletteSection> {
    use crate::theme::fa;
    let bar_of = |sample: u64| r.tempo_map.sample_to_bar(sample, r.sample_rate).0 + 1;
    let mut entries: Vec<(u64, PaletteRow)> = r
        .markers
        .markers
        .iter()
        .map(|m| {
            (
                m.start_sample,
                item_row(
                    PaletteItem::Marker(m.id),
                    m.name.clone(),
                    Vec::new(),
                    format!("Marker › Bar {}", bar_of(m.start_sample)),
                    fa::FLAG,
                ),
            )
        })
        .collect();
    for p in &r.compose.placements {
        let Some(def) = r.compose.find_definition(p.definition_id) else {
            continue;
        };
        let start = r.tempo_map.bar_to_sample(p.start_bar);
        entries.push((
            start,
            item_row(
                PaletteItem::Section { start },
                def.name.clone(),
                Vec::new(),
                format!("Section › Bar {}", p.start_bar + 1),
                fa::MUSIC,
            ),
        ));
    }
    entries.sort_by_key(|(s, _)| *s);
    one_section(
        "Markers & sections",
        filtered(rest, entries.into_iter().map(|(_, r)| r).collect()),
    )
}

/// `#` — every track, in track order.
fn build_tracks(r: &Resonance, rest: &str) -> Vec<PaletteSection> {
    use crate::theme::fa;
    let rows = r
        .sorted_tracks()
        .iter()
        .map(|t| {
            item_row(
                PaletteItem::Track(t.id),
                t.name.clone(),
                Vec::new(),
                format!("Track › {:?}", t.track_type),
                fa::SLIDERS,
            )
        })
        .collect();
    one_section("Tracks", filtered(rest, rows))
}

/// `+` — the plugins the selected track can take next: an instrument for
/// an empty instrument track, effects otherwise.
fn build_plugins(r: &Resonance, rest: &str) -> Vec<PaletteSection> {
    use crate::theme::fa;
    let Some(track) = r
        .ui
        .interaction
        .selected_track
        .and_then(|id| r.registry.tracks.iter().find(|t| t.id == id))
    else {
        let row = PaletteRow {
            unavailable: Some("Select a track first"),
            ..item_row(
                PaletteItem::Plugin(usize::MAX),
                "Add plugin…".to_string(),
                Vec::new(),
                "Mixer › Chain".to_string(),
                fa::SLIDERS,
            )
        };
        return one_section("Add plugin", vec![row]);
    };
    let wants_instrument = matches!(track.track_type, resonance_audio::types::TrackType::Instrument)
        && track.plugins.is_empty()
        && track.sub_track.is_none();
    let rows = r
        .plugin_catalog
        .available_plugins
        .iter()
        .enumerate()
        .filter(|(_, p)| p.is_instrument == wants_instrument)
        .map(|(i, p)| {
            item_row(
                PaletteItem::Plugin(i),
                p.name.clone(),
                Vec::new(),
                format!("Add to {} › {}", track.name, p.vendor),
                fa::SLIDERS,
            )
        })
        .collect();
    one_section("Add plugin", filtered(rest, rows))
}
