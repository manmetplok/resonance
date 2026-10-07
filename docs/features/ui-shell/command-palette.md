# Command palette

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/command-palette.md` (P2, P4); prototype
> `docs/history/prototypes/keyboard-shortcuts-command-palette/index.html`.

⌘K (or ⇧⌘P; Ctrl on Linux) opens a search box over the window. The user
types part of a command's name and presses ↵ to run it. Every command in the
[registry](command-registry-and-shortcuts.md) is listed with its shortcut.
Commands that can't run right now are dimmed with the reason. A leading
prefix switches to another source: `:` go to a bar, `@` jump to a marker or
section, `#` select a track, `+` add a plugin.

## Behaviour

**Opening and closing**

- ⌘K / ⇧⌘P opens the palette in command mode, and pressing ⌘K again closes
  it.
- ⌘J opens it in `:` mode. If the palette is already open, ⌘J switches the
  query to `:`.
- The palette never opens over the recovery prompt, the startup screen or a
  bounce, mixdown or freeze progress modal (`Overlay::allows_palette`).
- Any other modal overlay closes first, using its dismiss message.
- Closing keeps the query (`ui.palette_memory`). The next open shows it
  selected, so typing replaces it and ↵ runs the same command again.

**Look**

- A 640 px card sits 96 px from the window top, over a 60 % black backdrop.
  Clicking the backdrop closes the palette.
- Rows have a glyph tile, the name with the matched characters in
  `ACCENT_SOFT`, a dimmed breadcrumb ("Transport › Loop"), and the chord as
  platform keycaps on the right.
- The footer shows ↑↓ / ↵ / Esc hints and a result count. With an empty
  query it also lists the prefixes.
- Rows are 50 px, section headers 30 px and the list 380 px. These fixed
  heights let the reducer keep the selection in view (`follow_selection`).

**Empty query.** *Recent* comes first: up to 8 commands run from the palette
or by shortcut, newest first. *Suggested for this view* follows, a fixed
list per `ViewMode` that skips anything already in Recent:

- Arrange: loop, split, add-track menu.
- Mixer: add bus, master FX bypass, bounce.
- Compose: create section, loop section.
- Performance: play/stop, exit.

**Query.** `palette::score` runs `fuzzy_match` on the display name at full
weight and on the keywords at half weight. A name match keeps its highlight
even when a keyword scores higher.

- Rows sort by score, then by recent use, then available before
  unavailable, then registry order.
- At most 200 rows (`MAX_ROWS`).
- Rows group by category, and each category sits where its best hit ranks.
- Undo and Redo rows name the edit they would step over, e.g. "Undo delete
  bus".
- No match shows *No commands match "q"*. Each argument mode has its own
  empty message.

**Keys and pointer**

- ↑/↓ wrap around. They come through the global key handler, because
  `text_input` ignores vertical arrows.
- ↵ or a click runs the selected row.
- Hover selects a row only after a real pointer move (`pointer_armed`). A
  row that scrolls under a pointer at rest never takes the keyboard
  selection.

**Running a row.** Running closes the palette, then dispatches the command
through `shortcuts::execute`.

- This skips the typing gate, because the palette's own field had focus and
  the choice was explicit.
- Every other gate still applies.
- Availability is checked again at run time. An unavailable row flashes its
  reason and does nothing.
- While the palette is open, row availability is refreshed on every tick
  (`refresh_availability`) without re-ranking.

**Argument modes** (`PaletteMode`)

| Prefix | Rows | Runs |
|---|---|---|
| `:` | `17` bar, `17.3` bar and beat, `+5` / `-2` bars from the playhead. A bar past the song end or a beat the meter lacks is dimmed with the reason. | `TransportMessage::SeekTo(SeekTarget::Bar / NudgeBars)` |
| `@` | Arrangement markers and section placements, in timeline order | `MarkerMessage::JumpTo(id)`, or `SeekToSample(start)` for a section |
| `#` | Every track | `UiMessage::SelectTrack(Some(id))`. The track is selected but the arrangement does not scroll to it. |
| `+` | The plugin catalog: instruments for an empty instrument track, effects otherwise | `PluginMessage::AddPluginToTrack(selected, plugin)` |

**Recents persistence.** Recents are stored in `settings.json` as
`CommandId::key()` strings, under `palette.recent`. They are written 2 s
after the last change (`RECENT_WRITE_DELAY`) and again when the session
closes, so a burst of shortcuts costs one write. Unknown keys are dropped
when read. The palette-opening commands themselves are never recorded.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Model and ranking | `resonance-app/src/palette.rs` | `PaletteState`, `PaletteMode`, `PaletteItem`, `PaletteRow`, `PaletteMsg`, `build`, `score`, `suggested`, `record_recent`, `flush_recent`, `parse_bar`, `refresh_availability` |
| Reducer | `resonance-app/src/update/palette.rs` | `open`, `close`, `handle`, `key`, `run`, `follow_selection`, `ROW_HEIGHT` |
| View | `resonance-app/src/view/palette.rs` | `view_palette_overlay` (`CARD_WIDTH`, `CARD_TOP`) |
| Matcher | `resonance-app/src/commands/fuzzy.rs` | `fuzzy_match`, `FuzzyMatch` |
| Overlay slot | `resonance-app/src/state/overlay.rs` | `Overlay::Palette`, `allows_palette` |
| Persistence | `resonance-app/src/settings.rs` | `PaletteSettings` (lenient deserialisation) |

Invariant: the results are built in the reducer and stored on
`r.ui.palette`, so `view()` never ranks or allocates the list.

## Surfaces

- **GUI:** ⌘K / ⇧⌘P / ⌘J; the "Command Palette" command itself.
- **Control API / MCP:** none, by design (spec §2 non-goal).
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Open/toggle, type and run, typing-gate bypass, wrap, Esc keeps the query, unavailable flash, overlay interplay, pinned ranking, empty state, `:` parse, ⌘J, `@` order, `#` and `+` | `resonance-app/tests/performance/command_palette.rs` | `./scripts/run-tests.py -p resonance-app` |
| Recents debounce, pointer at rest, keyword highlight, go-to validation, per-mode empty text, row going unavailable before run | `resonance-app/tests/performance/shortcut_review_fixes.rs` | same |
| Goldens | `resonance-app/tests/snapshots/command_palette_{empty,results,no_match,linux_keycaps}-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test performance` |

Not covered: mouse hover and click on a rendered card (the reducer
messages are tested, real pointer events are not), and macOS keycaps.

## Decisions and deviations

- `fuzzy_match` tries every occurrence of the first query character as the
  start of the alignment. A match that starts at a word boundary takes no
  leading-gap penalty, so "loop st" finds both "Playhead to Loop Start" and
  "Set Loop Start…".
- Recent use is a tie-break after the score, not a boost added to it.
- The palette is drawn over the Performance shell too, so what the keyboard
  gate assumes is always what is on screen.
- These prototype features were not built: ⌘↵ "Run & keep open", and the
  "312 commands" footer (the footer counts results).

## Known gaps

- `#` selects a track without scrolling to it
  ([intent](../../intents/ui-shell/scroll-to-track.md)).
- There is no ⌘↵ to run a command and keep the palette open.
