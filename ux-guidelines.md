# Resonance UX Guidelines

These guidelines define the user experience principles and visual standards for Resonance. All UI work — in the main app (Iced) and plugin editors (egui) — must follow these rules.

## Design Philosophy

Resonance targets a **dark, lavender-accented pro-audio aesthetic** — quiet blue-greys, a single saturated brand accent, warm amber for bus/clip/loop chrome, and a small set of soft semantic colors. The UI should feel like an instrument: every control has a clear purpose, nothing is decorative, and visual noise is kept to a minimum. The interface should recede and let the user focus on music.

### Core Principles

1. **Clarity over density** — every element must be immediately readable at a glance. If a label or control isn't clear without a tooltip, it needs redesigning.
2. **Consistency** — identical actions look identical everywhere. A mute button in the mixer looks and behaves the same as in the track header.
3. **Direct manipulation** — prefer drag, click, and scroll interactions over menus and dialogs. The user should feel like they're touching the controls.
4. **Minimal chrome** — borders, separators, and containers should be subtle. Use spacing and background shade differences to group elements, not heavy outlines.
5. **Feedback** — every interaction must have visible feedback: hover states, press states, active indicators. No silent clicks.

## Color Palette

All colors are defined in `resonance-app/src/theme.rs` as canonical tokens. Do not introduce ad-hoc colors; if a new semantic role is needed, add a token. The historical aliases (`BG`, `PANEL`, `PANEL_DARK`, `SEPARATOR`, `TEXT`, `TEXT_DIM`, `SOLO_YELLOW`, `RECORD_RED`) still resolve so legacy call sites keep compiling, but new code should use the canonical tokens below directly.

### Backdrop layers (5 steps, window → raised control)

| Token | Hex | Usage |
|-------|-----|-------|
| `BG_0` | `#0f1013` | OS window backdrop |
| `BG_1` | `#15161b` | App body (inside window chrome) — meter / fader-track backgrounds, ruler bg |
| `BG_2` | `#1b1d23` | Panels, channel strips, cards, track headers, global-shelf rows |
| `BG_3` | `#23262e` | Hover state, raised controls |

### Borders & text

| Token | Hex | Usage |
|-------|-----|-------|
| `LINE` | `#272a31` | Standard 1px borders / dividers |
| `LINE_2` | `#1f2229` | Subtle inner hairlines, beat lines, track-row separators |
| `TEXT_1` | `#e8e7e3` | Primary text |
| `TEXT_2` | `#9aa0ac` | Secondary text, summary chrome, lane labels |
| `TEXT_3` | `#5d626d` | Tertiary / faint labels |
| `TEXT_4` | `#3f434c` | Disabled |

### Accent and semantic colors

| Token | Hex | Usage |
|-------|-----|-------|
| `ACCENT` | `#8b6dff` | Lavender — brand, selection, MIDI clips, active tabs |
| `ACCENT_SOFT` | `#a892ff` | Lavender text on dim backgrounds |
| `ACCENT_DIM` | `#8b6dff` @ 16% | Selection-fill wash |
| `ACCENT_LINE` | `#8b6dff` @ 34% | Selection outlines |
| `WARM` | `#e8c47b` | Warm amber — audio clips, busses, playhead, solo, loop markers |
| `WARM_LINE` | `#e8c47b` @ 34% | Bus-strip outlines |
| `GOOD` | `#6dd6a3` | Mint — meters, metronome on, success |
| `BAD` | `#e87b8b` | Soft pink — mute, peaking, record arm, error |

### Color Rules

- Never use pure white (`#ffffff`) — the brightest text is `TEXT_1` (`#e8e7e3`).
- Never use pure black (`#000000`) for backgrounds — the darkest is `BG_0` (`#0f1013`).
- `ACCENT` (lavender) is reserved for selection, brand chrome, and MIDI-domain affordances. Do not use it for static decoration.
- `WARM` (amber) is the audio-domain counterpart — clips, busses, playhead, loop region, solo state. Solo and peak/warning intentionally collapse onto the same warm amber.
- `BAD` (soft pink) is the unified record/mute/peak/error color — combined with a border or icon shape so it never relies on color alone.
- Introduce new semantic colors only through `theme.rs`, never inline.

## Typography

- **UI text**: System default sans-serif via Iced's default font.
- **Icons**: Font Awesome Solid via the bundled "Resonance Icons" font (`theme::ICON_FONT`). Custom glyphs exist for metronome, mono/stereo indicators.
- **Sizing**: Use Iced's default text size as the baseline. Section headers may go 2-4px larger. Never go below 11px equivalent.
- **Shaping**: Use `Shaping::Basic` for icon text.

## Layout Constants

These are defined in `theme.rs` and must be used consistently. Do not hard-code these numbers in view code.

### Arrange view

| Constant | Value | Usage |
|----------|-------|-------|
| `TRACK_HEIGHT` | 96px | Track row height ("balanced" density per redesign spec) |
| `RULER_HEIGHT` | 28px | Timeline ruler |
| `SECTION_BAND_HEIGHT` | 22px | Section-pill strip under the ruler |
| `GLOBAL_SHELF_HEADER_HEIGHT` | 32px | Always-visible "GLOBAL" summary strip (caret toggle + summary) |
| `GLOBAL_TRACK_CHORD_HEIGHT` | 56px | Chord lane inside the expanded shelf (section tabs above chord blocks) |
| `GLOBAL_TRACK_TEMPO_HEIGHT` | 40px | Tempo automation lane inside the expanded shelf |
| `GLOBAL_TRACK_SIG_HEIGHT` | 28px | Time-signature lane inside the expanded shelf |
| `GLOBAL_TRACK_GLYPH_SIZE` | 22px | Glyph tile next to each global-lane label |
| `TRACK_HEADER_WIDTH` | 280px | Left-side track header column |
| `CLIP_LANE_INSET` | 10px | Vertical inset of clip cards inside a track lane |
| `CLIP_EDGE_THRESHOLD` | 6px | Pixel radius that starts a trim instead of a move |

A legacy alias `GLOBAL_TRACK_ROW_HEIGHT` resolves to the tempo-lane height for old call sites; prefer the per-lane constants in new code.

### Mixer view

| Constant | Value | Usage |
|----------|-------|-------|
| `MIXER_STRIP_WIDTH` | 140px | Track channel strip |
| `MASTER_STRIP_WIDTH` | 156px | Master bus strip |
| `INSPECTOR_WIDTH` | 320px | Right-side inspector column |
| `MIXER_STRIP_HEIGHT` | 440px | Fixed track-strip height — pins the fader, scrolls FX list inside |
| `BUS_STRIP_HEIGHT` | 320px | Fixed bus-strip height (no instrument slot); sized with `MIXER_STRIP_HEIGHT` so both lanes fit the 1440×900 minimum window |
| `MIXER_STRIP_GAP` | 16px | Gap between unrelated strips in a lane (parent + sub-track clusters stay flush) |
| `MIXER_LANE_HPAD` | 26px | Horizontal lead-in/lead-out of the strip lanes |
| `FADER_HEIGHT` | 120px | Vertical fader travel |

### Compose view

| Constant | Value | Usage |
|----------|-------|-------|
| `COMPOSE_RAIL_WIDTH` | 324px | Right-rail column |

### Radius scale

Use the radius scale rather than hand-picking corner radii:

| Token | Value | Usage |
|-------|-------|-------|
| `RADIUS_XS` | 4px | Cells, tiny buttons |
| `RADIUS_SM` | 6px | Segmented tabs |
| `RADIUS_MD` | 7px | Standard buttons + inputs |
| `RADIUS_LG` | 8px | Clip cards, chord cards, instrument slots |
| `RADIUS_XL` | 12px | Strip cards, drum-grid panel |

### Timing

| Constant | Value | Usage |
|----------|-------|-------|
| `PEAK_DECAY` | 0.85 | VU-meter peak decay factor per frame |
| `TICK_INTERVAL_MS` | 16 | Engine-event drain tick (≈60 Hz) |

## Global-Tracks Shelf (Arrange view)

The global tracks (tempo, signature, sections, chord progression) live in a **collapsible shelf** that hangs below the section band and above the regular track lanes. The shelf is the primary global-musical-context surface on the Arrange view — when it's expanded, the user can scan harmony / tempo / meter without leaving the timeline; when it's collapsed, a one-line summary keeps the same information legible.

### Anatomy

1. **Header strip** (`GLOBAL_SHELF_HEADER_HEIGHT`, always visible). Left to right:
   - Caret toggle (▾ when expanded, ▸ when collapsed) — clicking it toggles the shelf.
   - `GLOBAL` tag in `TEXT_2`.
   - Count badge `{N}` showing the number of section/chord events.
   - Single-line summary: `{numerator}/{denominator} · {bpm} BPM · {root} {mode} · {chord-total} chords`.
   - The `+` add-track action lives here, at the right edge of the header strip.
   - The header strip is part of `fixed_header_height` — it never scrolls.

2. **Lanes**, top-to-bottom when expanded:
   - **Chord lane** (`GLOBAL_TRACK_CHORD_HEIGHT`, 56px). Section-name tabs row above flattened chord blocks. Block fill is tinted by chord quality:
     - Minor → lavender wash (`ACCENT_DIM` family).
     - Dominant 7 / altered → warm amber (`WARM` family).
     - Major / neutral / sus / suspended → neutral panel (`BG_2` with `LINE` border).
   - **Tempo lane** (`GLOBAL_TRACK_TEMPO_HEIGHT`, 40px). Tempo automation curve over `BG_2`.
   - **Signature lane** (`GLOBAL_TRACK_SIG_HEIGHT`, 28px). Compact pills + downbeat ticks; compound meters get a small hint glyph.

3. **Left-column chrome.** Each lane's row in the `TRACK_HEADER_WIDTH` column gets a label tile (`GLOBAL_TRACK_GLYPH_SIZE` tile + name). The shelf's header label aligns vertically with the caret in the canvas.

### Rules

- The shelf header strip is **always rendered**, even when collapsed — a fresh project must still show the summary so the user has at-a-glance project context.
- The caret-toggle hit zone covers the full header strip; clicking anywhere on the strip toggles unless the click lands on the `+` button on the right.
- Chord-quality tints come from existing tokens (`ACCENT_DIM`, `WARM`, `BG_2` + `LINE`). Do not introduce new per-chord colors.
- All lane heights are constants — never inline pixel numbers. If a lane needs to grow, change the constant.

## Collapsible Panels

Dense sections fold away behind the **same caret pattern as the global-tracks shelf**: a 12×12 caret tile (`view::controls::collapse_caret`, ▾ open / ▸ closed, 9px glyph in `TEXT_3`). Never draw an ad-hoc chevron.

Collapsible surfaces and their state homes:

- **Arrange — global-tracks shelf** (`viewport.global_tracks_expanded`, `UiMessage::ToggleGlobalTracks`).
- **Mixer inspector groups** SIGNAL / ROUTING / CHAIN (`MixerUiState::collapsed_inspector_groups`, `UiMessage::ToggleMixerInspectorGroup`). Header row: uppercase title left, caret right, hairline below; the hairline stays when collapsed.
- **Compose workspace group banners** SECTION / TRACKS (`ComposeState::{section,track}_lanes_collapsed`, `ComposeMessage::ToggleWorkspaceGroup`). The banner itself always stays visible; collapsing hides every lane under it.
- **Compose right-rail panel cards** (`ComposeState::collapsed_rail_panels`, `ComposeMessage::ToggleRailPanel`, keyed by `RailPanelKey`). When collapsed, only the header row of the card remains; any right-side meta (e.g. "ABAB · 4 LINES" on Lyric draft) stays visible so context isn't lost.

### Rules

- Collapse state is **runtime UI state** — never persisted into project files, never undoable (`UndoAction::Skip`).
- Everything defaults to **open**; state stores the *collapsed* set so the default is the empty set.
- Dynamic panels (per drum group, per track) are keyed by stable ids (`RailPanelKey::DrumMeter(group_id)`, …) — never by list index.
- The whole header row is the click target (not just the caret) and gets a hover state (`small_button_style` or equivalent).
- Action rows (Generate buttons, group-selector tabs, the editing-context header) are navigation, not content — they don't collapse.
- The collapsed branch should skip *building* the body, not merely hide it.
- Performance: a live (audio-tick) readout must never sit inside an `iced::widget::lazy` region whose fingerprint omits it — the mixer inspector keeps SIGNAL outside the lazy ROUTING/CHAIN block, and collapse flags are hashed into the lazy fingerprint.

## Controls

### Buttons

Use the appropriate style function from `theme.rs`:

- `transport_button_style` — playback transport controls
- `record_armed_button_style` — record arm (red tint + border)
- `tab_button_style(active)` — view tab switching (Arrange/Mixer/Compose)
- `section_button_style(active, color)` — section buttons in Compose
- `toggle_button_style(active, color, small)` — toggles (monitor, metronome, punch)
- `mono_button_style(is_mono)` — mono/stereo toggle
- `small_button_style` — compact inline buttons (delete, add)
- `floating_button_style` — overlay buttons on canvas (zoom)

All buttons must have distinct hover, pressed, and default states. Use the `RADIUS_*` scale rather than hand-picking corner radii — `RADIUS_MD` (7px) for standard buttons, `RADIUS_XS` (4px) for small/inline buttons, `RADIUS_LG` (8px) for transport / record-armed style.

### Knobs

- Vertical drag to change value (not circular). Full travel = 140px.
- Shift held = fine adjustment.
- Double-click = reset to default/center.
- Arc indicator with dead zone at bottom (270-degree sweep).

### Faders

- Vertical orientation, `FADER_HEIGHT` (120px) travel.
- Same shift-for-fine and double-click-reset as knobs.

### Metering

- VU meters use `METER_BG` (= `BG_1`, `#15161b`) as background.
- Peak decay factor: `PEAK_DECAY` (0.85) per frame tick (`TICK_INTERVAL_MS` = 16 ms).
- Peak hold indicator before decay.

## Interaction Patterns

### Mouse

- **Drag**: Primary interaction for knobs, faders, clip moving/trimming, selection.
- **Shift+drag**: Fine adjustment for continuous controls.
- **Double-click**: Reset to default (knobs, faders) or open edit mode (clip names, BPM).
- **Right-click**: Context menus where applicable.
- **Scroll**: Zoom (timeline), scroll (lists).

### Keyboard

- Transport controls should have single-key shortcuts (Space = play/stop, R = record).
- Tab-based navigation for main views.
- Escape to cancel/deselect.

## Container Hierarchy

Use the container style helpers from `theme.rs`:

1. `base_bg` — outermost window background
2. `panel_bg` / `panel_outlined` — main content areas (track headers, panels)
3. `panel_dark_bg` / `panel_dark_outlined` — recessed sub-areas (mixer strips)
4. `separator_bg` — 1px divider strips between regions
5. `timing_panel_style` — bordered panel for the transport timing display

Nesting depth should rarely exceed 3 levels. If you need more, reconsider the layout.

## Plugin Editors (egui)

Plugin UIs use egui via `wayland-plugin-gui`. They should feel consistent with the main app despite the different framework:

- Match the dark color palette as closely as possible.
- Use the shared plugin UI helpers from `resonance-plugin/src/ui.rs`.
- Tab-based layout for multi-section editors (OSC, ENV, FLT, LFO, MOD, FX).
- Parameter sliders should match the drag-to-adjust, shift-for-fine pattern.
- Visualization panels (oscilloscope, filter response, envelope shapes) go in a `viz/` submodule.

## Accessibility

- All interactive controls must have hover and active visual states — no invisible hit targets.
- Maintain sufficient contrast: text on panel backgrounds must be clearly readable.
- Don't rely on color alone to convey state — combine color with shape or position changes (e.g., record arm uses red color AND a border).

## Adding New UI Elements

When adding new controls or views:

1. Check if an existing control can be reused or extended.
2. Define any new colors in `theme.rs`, not inline.
3. Follow the existing message-driven architecture: UI emits `Message` variants, `update.rs` dispatches.
4. Keep view functions pure — no side effects, only return Iced elements.
5. New layout constants go in `theme.rs`.
6. Test at the standard window size and verify elements don't overlap or clip.
