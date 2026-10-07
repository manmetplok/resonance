# Theme and view-layer performance

> Verified against master @ `8eac71b3` on 2026-10-07.
> Rules: `docs/guides/ux-guidelines.md` (the always-true design and
> view-performance rules). Code review UX-05/07/19/23.

The app's look comes from one module of design tokens: colours, fonts,
layout sizes, radii, and button and container styles. Its speed comes from
a few view-layer rules. iced rebuilds the whole widget tree on every
`view()` call, which means every 16 ms tick while anything moves. So `view()`
must not allocate lists, live visuals are Canvas programs with their own
caches, and regions that don't change are `lazy`. This doc covers where
both live and what enforces them. The rules themselves are in
`docs/guides/ux-guidelines.md`.

## Behaviour

**Tokens** (`resonance-app/src/theme.rs`)

- **Colours.**
  - Backgrounds `BG_0`–`BG_3`, lines `LINE` / `LINE_2`, text `TEXT_1`–`TEXT_4`.
  - Lavender `ACCENT` with `_SOFT` / `_DIM` / `_LINE`.
  - Amber `WARM`, mint `GOOD` and pink `BAD`, each with `_DIM` / `_LINE`
    variants.
  - `ON_ACCENT_TEXT`, `INSTRUMENT_TINT`, and the frost treatment for frozen
    tracks (`FROST_WASH`, `FROST_EDGE`, `FROST_ICON`).
  - Drum-group hues `GRP_*`.
  - Ten track colours (`TRACK_PALETTE`).
  - Legacy aliases (`BG`, `PANEL`, `TEXT`, …) are kept for old call sites.
- **Fonts.** Geist (`UI_FONT`, `_MEDIUM`, `_SEMIBOLD`), Geist Mono
  (`MONO_FONT`), Instrument Serif italic (`SERIF_ITALIC_FONT`) and a renamed
  Font Awesome (`ICON_FONT`, shaped with `Shaping::Basic` by `icon()`). All
  are bundled and registered in `main.rs`.
- **Iced theme.** `resonance_theme()` is just `Theme::Dark`. Every widget
  that matters gets an explicit style function.
- **Layout.** Track, ruler, shelf and mixer-strip sizes, `BROWSER_WIDTH`,
  the `RADIUS_*` scale and `RADIUS_PILL`.
- **Styles.**
  - Buttons: `transport_`, `record_armed_`, `tab_`, `section_chip_`,
    `toggle_`, `mono_`, `small_`, `ghost_`, `primary_`, `floating_` and
    `destructive_button_style`.
  - Containers: `base_bg`, `panel_bg`, `panel_outlined`,
    `panel_dark_outlined`, `separator_bg`, `card_selected`, `card_warm`.
  - The rest: `fader_style`, `borderless_text_input_style`, and the keycap
    kit (`kbd`, `keycap`, `keycap_row`, `KeycapTone`, `active_row_style`,
    `edited_pill_style`, `conflict_ring_style`).

**Tick rate** (`update/tick.rs`)

- `tick_interval` runs at `TICK_INTERVAL_MS` (16 ms, `update.rs`) when
  something needs it:
  - playing or recording;
  - unsettled meters;
  - monitored inputs;
  - audition, renders, loads, export, the reference rail, control jobs.
- Otherwise it runs at `TICK_INTERVAL_IDLE_MS` (200 ms).
- Meter levels decay by `PEAK_DECAY` (0.85) per tick and snap to zero below
  `PEAK_SETTLE_EPSILON`, so an idle app really does go idle.

**View-performance mechanisms**

- **Cached pick-list options.** `state/ui_caches.rs::UiViewCaches` holds
  `Rc<[T]>` option lists (MIDI ports, outputs, channels, plugin lists,
  input devices, banks, programs, device presets, the compose rail memo).
  They are rebuilt only when the source data changes.
- **`lazy` regions**, each keyed on a fingerprint of what it shows:
  - the mixer strips' non-live bodies and the inspector;
  - the track headers;
  - the compose page and rail;
  - the Files listing;
  - the Keyboard page;
  - the preset lists and the plugin-window parameters;
  - the Performance stage, lane and footer.
- **Canvas with `canvas::Cache`** for live visuals: `controls::meter_v`
  (`StereoMeterCanvas`), knobs (`view/knob.rs`), the transport meter,
  timeline and piano-roll canvases, audition scrub, and the Performance
  diagrams and beat ring.
- **Stable tree shape.**
  - The status area, the plugin-window layer, the preset-drag wrapper and
    the chrome's view-specific slots are always present (zero-size when
    empty).
  - The timeline and piano-roll canvases have a fixed pixel width inside a
    horizontal `Scrollable`, rather than `Length::Fill`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Tokens and styles | `resonance-app/src/theme.rs` | see above |
| Fonts and window | `resonance-app/src/main.rs` | `.font(...)`, `.default_font(UI_FONT)`, `.antialiasing(false)` |
| Tick | `resonance-app/src/update.rs`, `update/tick.rs` | `TICK_INTERVAL_MS`, `TICK_INTERVAL_IDLE_MS`, `tick_interval`, `needs_fast_tick`, `PEAK_DECAY` |
| Option caches | `resonance-app/src/state/ui_caches.rs` | `UiViewCaches`, `RevisionMemo` |
| Live canvases | `resonance-app/src/view/controls.rs`, `view/knob.rs` | `meter_v`, `StereoMeterCanvas`, `KnobState`, `collapse_caret` |
| Plugin editors' palette | `plugin-gui-core/src/theme.rs` (`lavender`) | shared egui tokens, see [../plugins-instruments/plugin-editor-runtimes.md](../plugins-instruments/plugin-editor-runtimes.md) |

## Surfaces

- **GUI:** everything.
- **Control API / MCP:** none.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Text tiers reach AA contrast on every panel; error-banner contrast | `resonance-app/tests/mixer/theme_contrast.rs` | `./scripts/run-tests.py -p resonance-app` |
| No `.size(8)` text outside the glyph allow-list | `tools/arch-invariants/tests/architecture.rs::view_text_never_shrinks_below_the_9px_floor` | `cargo test -p arch-invariants` |
| No `Color::WHITE` / `Color::BLACK` in `view/` | `…::view_layer_never_hardcodes_pure_white_or_black` | same |
| The view never reads the engine | `…::view_layer_never_reads_the_engine` | same |
| Idle vs fast tick, meter settle, peak polling | `resonance-app/tests/mixer/tick_gating.rs` | `./scripts/run-tests.py -p resonance-app` |
| `lazy` fingerprints | `resonance-app/tests/mixer/inspector_lazy_fingerprint.rs`, `mixer/plugin_window_fingerprint.rs`, `compose/compose_rail_lazy.rs`, `compose/compose_canvas_cache_fingerprints.rs`, `io/files_listing_fingerprint.rs` | same |
| Tree shape survives a banner | `resonance-app/tests/timeline/status_area_keeps_widget_state.rs` | same |
| Keycap kit | `resonance-app/tests/performance/keycap_styles.rs` (golden `keycap_styles_sample`) | same |

Not covered: there is no frame-time benchmark. View cost during a resize
is only checked by hand.

## Decisions and deviations

- `resonance_theme()` uses iced's stock `Theme::Dark` palette as the base
  and overrides styles per widget. There is no custom `iced::Theme`.
- Antialiasing is off (it was the dominant per-frame cost on radv).
- Keycaps use a uniform 1 px border. iced 0.14 can't draw the design's
  2 px bottom edge.
- `ux-guidelines.md` used to point at `.claude/skills/ui-work.md` §11 for the
  view-performance rules. That file is gone, and the rules now live in the
  guide itself.

## Known gaps

- None tracked as intents.
