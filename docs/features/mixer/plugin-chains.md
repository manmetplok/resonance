# Plugin insert chains (track, bus, master)

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/mixer-cleanup.md` §3.2 / §4 (CHAIN row, generic window); code review ARCH2-02 (one `ChainOwner` API); ba doc #275 P5 / X3 (replace, per-slot bypass).

Every track, bus and the master owns an ordered insert chain of CLAP
plugins; the index is the processing order. A user edits a chain in the
inspector's CHAIN group (add, remove, drag to reorder, bypass, replace,
presets, open) and sees its state as slot lines on the strip. An agent uses
the same verbs under `track.*`, `bus.*` and `master.*`. All three chains
share one engine command set and one app code path, parameterised by
`ChainOwner`.

## Behaviour

### Chain rules

- **Owners.** `ChainOwner::{Track(id), Bus(id), Master}`. Plugin instance
  ids are unique across all three, so bypass, replace, params and presets
  are addressed by instance id alone.
- **Instrument floor (tracks only).** On an instrument track the
  instrument slot is structural: effects sit after it, nothing moves above
  it, and `remove_effect` cannot remove it (`track.add_instrument`
  replaces it). The rule lives once in `plugin_chain.rs` and is applied by
  drag, ☰ Move up/down and `track.move_effect` alike. Sub-tracks and
  external-instrument tracks have no instrument slot.
- **Add** appends at the end under an app-allocated instance id. The GUI
  waits for the engine's `PluginAdded` echo; the control API mirrors a
  placeholder slot at once (`chain_edit::add_with_id`) so the reply can
  name the slot.
- **Remove** and **move** mirror at once and the echo is owed, so an undo
  pressed before the echo lands sees the edit.
- **Replace** keeps the slot's position. Naming the plugin already in the
  slot (at its current catalog path) is a *relocate*: same instance id,
  saved state and parameter overrides pushed back in. A different plugin
  is a *swap*: fresh instance id, old state dropped. Remove-then-add is
  never used, because it would move the slot to the end.
- **Bypass.** A whole-chain bypass (`SetFxBypass { owner }`, the strip's
  FX switch) and a per-slot bypass (`SetPluginBypass`, the row's `●`) are
  independent and both crossfade wet/dry over `BYPASS_FADE_MS` (5 ms), so
  a reverb tail fades instead of being cut. Per-slot bypass is saved
  (`ProjectPlugin::bypassed`).
- **Missing plugins** keep their slot, saved state and parameter
  overrides. The row shows `Replace / Rescan / Remove` under it; only
  Remove discards the preserved settings.

### Inspector CHAIN row

`⠿ ● Name  ↗ ☰ ×`:

- `⠿` drag to reorder; only the drop changes the chain; drop targets obey
  the instrument floor and stay within the owner.
- `●` per-slot bypass (`○` bypassed, pink when missing).
- Name: click focuses the slot, double-click opens it.
- `↗` opens the plugin window and is lit while the plugin's own editor is
  open (driven by the engine's `PluginEditorState` echo, never set
  optimistically).
- `☰` inline menu: Parameters…, Browse presets…, Previous / Next preset,
  Save preset… (presets need an available plugin), Replace…, Move up,
  Move down, Remove.
- `×` removes.
- Under the rows: the `+ Add` picker (FX, or instrument on an instrument
  track without one). In replace mode the picker offers replacements for
  the chosen slot instead.

### Opening a plugin

`PluginMessage::OpenPluginWindow` always yields a window: a plugin with
its own GUI opens its editor (`OpenPluginEditor`, see
[plugin-editor-runtimes.md](../plugins-instruments/plugin-editor-runtimes.md) for the editor runtime); a
plugin without one, a missing plugin, or one whose editor refused to open
gets the **generic plugin window** — an in-app floating panel (460 px)
with the plugin name and owner in a draggable title bar, the preset bar,
and the generic parameter list. One generic window is open at a time; Esc
closes it; removing its plugin or loading a project closes it.
"Parameters…" opens it for a GUI plugin too. Opening any window focuses
the slot and selects its channel.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Owner type | `resonance-audio/src/types/chain_owner.rs` | `ChainOwner` |
| Engine handlers | `resonance-audio/src/engine/chain.rs` | `handle_add_plugin`, `handle_remove_plugin`, `handle_move_plugin`, `handle_set_fx_bypass` |
| Bypass crossfade | `resonance-audio/src/bypass.rs` | `BypassFade`, `BYPASS_FADE_MS`, `run_faded` |
| Chain rules | `resonance-app/src/plugin_chain.rs` | `instrument_slot`, `displayed_instrument_slot`, `resolve_effect_move`, `engine_slot_index` |
| Shared app edits | `resonance-app/src/update/chain_edit.rs` | `add`, `add_with_id`, `remove`, `move_plugin` |
| Replace | `resonance-app/src/update/plugin_replace.rs` | relocate vs swap `classify` |
| Per-owner messages | `resonance-app/src/message.rs`, `update/bus.rs`, `update/master.rs` | `PluginMessage`, `BusMessage`, `MasterMessage` |
| CHAIN UI gestures | `resonance-app/src/update/chain_ui.rs` | `ChainUiMessage`, `settle` |
| Generic window | `resonance-app/src/state/plugin_window.rs`, `update/plugin_window.rs`, `view/plugin_window.rs` | `PluginWindowState`, `view_plugin_window` |
| CHAIN view, reorder | `resonance-app/src/view/mixer/inspector/chain.rs`, `view/mixer/reorder.rs` | `slot_menu_entries`, `chain_moves`, `drop_move` |
| Control addressing | `resonance-app/src/update/control/{effect_addressing,plugin_target,replace,bypass,chain_presets}.rs` | `resolve_effect` (slot **or** `plugin_id` + `occurrence`) |

Invariant: the arch-invariants suite forbids per-owner chain commands or
events (`AddPluginToBus` and the like); new chain verbs take a `ChainOwner`.

## Surfaces

- **GUI:** inspector CHAIN group; strip slot lines (click to focus,
  double-click to open) and FX switch; generic plugin window.
- **Control API / MCP** (each exists as `track.*`, `bus.*` and `master.*`
  → `mcp__resonance__{track,bus,master}_<verb>`):
  `add_effect`, `remove_effect`, `move_effect`, `replace_effect`,
  `set_fx_bypass`, `set_plugin_bypass`, `plugin_params`,
  `set_plugin_param`, `plugin_presets`, `load_plugin_preset`,
  `save_plugin_preset`. Track only: `track.add_instrument` →
  `mcp__resonance__track_add_instrument`. Effects are addressed by `slot`
  or by `plugin_id` (+ `occurrence`), never both. Presets themselves are a
  plugins topic ([plugin-preset-library.md](../plugins-instruments/plugin-preset-library.md)).
- **Agent skills:** `mixing`, `mastering`, `spatial` (add effects, set
  params, read `*_plugin_params`).

## How it's tested

| What | Where | Run |
|---|---|---|
| Engine chain edits | `resonance-audio/tests/engine/{track_plugin_chain,track_plugin_move,bus_plugin_move,master_plugin_move}.rs` | `./scripts/run-tests.py -p resonance-audio` |
| Bypass fade | `resonance-audio/tests/clap_host/plugin_bypass.rs`, `resonance-audio/tests/mixer/bypass_settle_race.rs` | same |
| CHAIN row, reorder, focus | `resonance-app/tests/mixer/{inspector_chain_row,mixer_chain_reorder,plugin_focus_window,plugin_removal_prunes_lanes}.rs` | `./scripts/run-tests.py -p resonance-app` |
| Generic window, preset bar | `resonance-app/tests/mixer/{mixer_generic_param_panel,generic_panel_display,mixer_preset_bar,plugin_window_fingerprint}.rs` | same |
| Bypass persistence | `resonance-app/tests/mixer/plugin_bypass_persistence.rs` | same |
| Missing plugins | `resonance-app/tests/plugins/{missing_plugin_slot,missing_plugin_state_preserved}.rs` | same |
| Control | `resonance-app/tests/control/{control_bus_effects,control_master_params,control_replace_effect,control_plugin_bypass,control_track_move_effect,control_track_remove_effect,control_track_add_plugin_result}.rs` | same |
| Goldens | `mixer_inspector_chain_focus_menu-wgpu.png`, `mixer_inspector_chain_missing_drag-wgpu.png`, `mixer_inspector_plugin_bypassed-wgpu.png`, `plugin_generic_window-wgpu.png` | `RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer` |

Not covered: a plugin's own editor window is only exercised by the
`#[ignore]`d Wayland/Cocoa editor tests listed in CLAUDE.md.

## Decisions and deviations

- The bottom plugin panel is gone; the generic window is an in-app
  floating panel, not an OS window (mixer-cleanup.md §4, option (a)).
- Missing-plugin recovery says "Rescan" where the spec said "Locate".
- ☰ menu and colour palette are inline cards, not popovers.
- The chain is read with `*.plugin_params`; the old `track.plugins` name
  meant the installed catalog, was renamed `plugins.catalog`, and its alias
  was removed (todos #1236 / #1240).

## Known gaps

- No sidechain controls in the CHAIN row —
  [`../../intents/mixer/sidechain-gui.md`](../../intents/mixer/sidechain-gui.md).
