# Host preset browser and track presets

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/plugin-preset-library.md` (host slices
> P5–P8, §18–§19 as-built). The status line at the top of that spec ("round 1
> built on `feat/plugin-presets`") is stale: rounds 1 and 2 are on master.

This is the app-side (iced) face of the plugin preset library. It covers:

- a preset bar in the generic plugin window;
- a preset-browser overlay where ↑/↓ audition presets live, ↵ keeps one
  and Esc reverts;
- a **Presets** tab in the media browser;
- dragging a preset onto an Arrange track header;
- the add-effect / add-instrument pickers' "with preset…" lists.

The library itself (format, factory banks, marks, trash, the plugin's own
egui browser, CLAP preset discovery) is documented in
[../plugins-instruments/plugin-preset-library.md](../plugins-instruments/plugin-preset-library.md).
Whole-track presets are covered at the end of this doc.

## Behaviour

**Library access** (`plugin_preset_library.rs`)

- `library(app)` returns a cached `resonance_plugin::presets::PresetLibrary`
  on the preset root. The root is `presets.plugin_preset_root`, else
  `user_preset_root()`, and tests get a hermetic one.
- Factory banks from the plugin scan are registered into the library.
- Favourites come from the shared `resonance_common::library_marks::SharedMarks`
  store, kind `plugin-preset`.

**Preset bar** (`view/preset_browser.rs::preset_bar`). It appears in the
generic plugin window and reads "◀ name • ▶ ☆ Presets…":

- ◀ and ▶ step through presets (`PresetUiMessage::Step`).
- The name and "Presets…" open the browser.
- ☆ toggles the favourite, once the current state has a preset identity.

The CHAIN row's ☰ menu in the mixer inspector offers the same actions:
Browse presets…, Previous/Next, Save preset….

**Overlay** (`view_preset_browser_overlay`, `Overlay::PresetBrowser`)

- A 520 px card holds:
  - a "PRESETS" header with the plugin name;
  - a search field, focused on open, and a "★ only" toggle;
  - a 320 px list of rows showing the category, "user" for user presets,
    and a star;
  - Revert and Load buttons.
- Clicking a row or pressing ↑/↓ **auditions** it. The preset loads into
  the plugin without an undo entry, and the first audition captures an
  `AuditionOrigin`.
- To keep the preset: ↵, double-click, Load, or a backdrop click. The
  browser restores the origin and then does one recorded load, so the whole
  browse is one undo entry.
- To revert: Esc, ×, or Revert. This loads the captured state blob plus the
  origin's parameter values.
- These keys are handled in `plugin_preset_ui::key`, which runs before the
  search field's own handling (`update/shortcuts.rs`).
- If the plugin goes away, the browser closes (`forget_instance`).
- While the overlay or the Presets tab is visible, its list re-queries the
  library once a second (`poll_visible_lists`).

**Commands.** Previous Plugin Preset, Next Plugin Preset and Browse Plugin
Presets target the generic window's plugin or the slot focused in the
inspector (`plugin_window::preset_target`). They have no default chord. They
are unavailable with "Select a plugin first", and never available in
Performance mode.

**Media Presets tab** (`media_presets_body`)

- Search, ★ only, and a plugin `pick_list` ("All plugins").
- The rows are a `lazy` region.
- Double-clicking a row loads it into the preset target, but only if the
  target is the same plugin. Otherwise a banner asks the user to open that
  plugin's window.
- Pressing a row arms a **drag**:
  - Releasing it on an Arrange track header (`DropOnTrack`) adds the
    plugin with that preset.
  - The drop counts only after the pointer has moved more than 4 px.
  - It is refused with a banner on a sub-track, on an instrument dropped
    anywhere but an empty instrument track, and on an effect dropped on an
    empty instrument track.
  - Esc, a press, leaving the window or losing focus ends the drag.

**"With preset…" pickers.** The CHAIN, bus and master inspectors each have a
second pick list next to "add effect" / "add instrument", listing
**favourite** presets only (`rebuild_add_picks`). Picking one adds the
plugin and loads the preset when the `PluginAdded` echo arrives
(`pending_plugin_presets`).

**What the host UI does not do.** It has no rename, delete or trash, no
metadata form, and no facets or sort. Those exist in the plugin-side egui
browser and over the control API (`presets.*`).

**Track presets** (`resonance-app/src/presets.rs`)

- A `TrackPreset` stores a track type plus its plugin chain, with state
  blobs.
- There are ten built-in presets (`default_presets`), and user presets are
  stored as JSON in `$DATA/resonance/track-presets/` (override with
  `RESONANCE_PRESET_DIR`).
- The add-track popover lists "Presets" and "User Presets". A user preset
  has a two-step inline delete. Deleting removes the file and is not
  undoable.
- The track context menu's **Save as preset…** opens a name prompt. If the
  name exists, the button reads "Overwrite" and a warning shows. Saving
  asks the engine for every plugin's state (`SaveAllPluginStates`) and
  writes the file when the states arrive (`engine_events/presets.rs::finish_preset_save`).
- Track presets are not a `library_marks` kind, so they have no stars or
  tags (spec P9).

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Library glue | `resonance-app/src/plugin_preset_library.rs` | `library`, `marks`, `preset_root`, `register_into` |
| State | `resonance-app/src/state/presets.rs` | `PresetState`, `HostPresetBrowser`, `HostPresetList`, `AuditionOrigin`, `PresetDrag`, `PresetAddPick` |
| Reducer | `resonance-app/src/update/plugin_preset_ui.rs` | `key`, `close_browser`, `revert`, `media_load`, `drop_on_track`, `add_with_preset`, `poll_visible_lists`, `rebuild_add_picks` |
| Messages | `resonance-app/src/message.rs` | `PresetUiMessage` |
| View | `resonance-app/src/view/preset_browser.rs` | `preset_bar`, `view_preset_browser_overlay`, `media_presets_body` |
| Track presets | `resonance-app/src/presets.rs`, `view/menus.rs`, `update/track.rs`, `engine_events/presets.rs` | `TrackPreset`, `default_presets`, `save_user_preset`, `view_add_track_menu`, `preset_save_overlay`, `handle_save_track_as_preset` |

## Surfaces

- **GUI:**
  - the generic plugin window's preset bar;
  - the CHAIN ☰ menu;
  - Media › Presets;
  - the add pickers' "with preset…";
  - the add-track popover;
  - the track context menu.
- **Control API / MCP:**
  - Library: `presets.search`, `presets.rename`, `presets.delete`,
    `presets.set_marks`, `presets.update_meta`, `presets.vocabulary` →
    `mcp__resonance__presets_search`, `mcp__resonance__presets_rename`,
    `mcp__resonance__presets_delete`, `mcp__resonance__presets_set_marks`,
    `mcp__resonance__presets_update_meta`, `mcp__resonance__presets_vocabulary`.
  - Per chain: `track.plugin_presets` / `track.load_plugin_preset` /
    `track.save_plugin_preset`, with `bus.*` and `master.*` twins →
    `mcp__resonance__track_plugin_presets`,
    `mcp__resonance__track_load_plugin_preset`,
    `mcp__resonance__track_save_plugin_preset` and the bus/master tools.
  - Track presets: `track.save_preset`, `track.presets`,
    `track.apply_preset` → `mcp__resonance__track_save_preset`,
    `mcp__resonance__track_presets`, `mcp__resonance__track_apply_preset`.
- **Agent skills:** none of the shipped skills drive the preset browser.

## How it's tested

| What | Where | Run |
|---|---|---|
| Bar, overlay, audition / keep / revert, Presets tab, "with preset…" | `resonance-app/tests/mixer/mixer_preset_bar.rs` | `./scripts/run-tests.py -p resonance-app` |
| Presets tab and overlay through `iced_test::Simulator`, double-click load, drag onto a header | `resonance-app/tests/io/preset_tab_widgets.rs` | same |
| CHAIN ☰ browse/save | `resonance-app/tests/mixer/inspector_chain_row.rs` | same |
| Control surface | `resonance-app/tests/control/control_presets_library.rs`, `control_plugin_presets.rs`, `control_chain_presets.rs`, `control_track_presets.rs` | same |
| Track preset prompt and filenames | `resonance-app/tests/plugins/track_preset_save_prompt.rs`, `io/preset_name_collisions.rs` | same |
| Goldens | `plugin_preset_bar`, `plugin_preset_browser`, `track_preset_save_prompt`, `add_track_menu_preset_delete_armed` | `RESONANCE_BLESS=1 …` |

Not covered: the Media › Presets tab and the drag preview have no golden.

## Decisions and deviations

- The host lists presets by calling `PresetLibrary::query` directly. It does
  not use `resonance_plugin::library_view::BrowserModel`, although that
  module's docs say it serves the app's iced skin too. In the app,
  `BrowserModel` is used only by the amp-model and drum-kit control
  handlers.
- "With preset…" is a second pick list, not a submenu. The drop target is
  the Arrange track header, not a mixer strip (spec §19 deviations).
- When a preset is auditioned on top of a state that was not a library
  preset, revert restores parameters plus the captured state blob.

## Known gaps

- Track presets and IRs are not library kinds (spec P9). Our plugins don't
  publish CLAP preset discovery to other hosts (P10). Both are tracked on
  the plugin side; see
  [../plugins-instruments/plugin-preset-library.md](../plugins-instruments/plugin-preset-library.md).
- The host UI has no rename, delete or metadata editing.
