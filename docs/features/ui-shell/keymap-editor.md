# Keymap editor and DAW presets

> Verified against master @ `8eac71b3` on 2026-10-07.
> Spec history: `docs/history/specs/command-palette.md` §8 (P5); prototype
> `docs/history/prototypes/keyboard-shortcuts-command-palette/index.html`
> (Preferences › Shortcuts).

Settings › Keyboard lists every command with its chord. Users can switch
the whole keymap to a Logic Pro or Pro Tools preset, rebind a command by
pressing the new chord, or reset it. The result is saved in
`settings.json` and takes effect immediately for the global key handler,
the canvases, the palette, tooltips and menu hints.

## Behaviour

**Presets** (`KeymapPreset`). Each preset is a short list of overrides
applied on top of `BindingMap::resonance_default`:

- **Resonance (default).**
- **Logic Pro:** C toggles the loop, ⌘U loops the selection, ⌘T splits at
  the playhead, Return goes to the start.
- **Pro Tools:** 0 plays/stops (an alternate, so Space still works), 3
  records, 4 toggles the loop, 7 toggles the metronome.

When a preset takes a chord from a default command, that command ends up
with no chord. `KeymapPreset::unbound` lists those commands, and the panel
shows them in a "<preset> takes some default chords — Left unbound: …"
box.

**Overrides.** `settings.keymap` is `{ preset, overrides: [{ command, chord:
Option<tokens> }] }`. `update::keymap::resolve` builds the live map from the
preset and then replays every override in order:

- A chord replaces the command's primary chord (`set_primary`). The
  alternates stay.
- `None` unbinds the command.
- An override with an unknown command key or an unparsable chord is
  skipped.
- A malformed `keymap` section, or one bad override inside it, falls back to
  defaults without affecting the other settings (`lenient`, `lenient_list`).

**The panel** (`view/settings_keyboard.rs`):

- The header has a preset `pick_list` and "Reset all".
- A "Filter shortcuts…" field filters with `fuzzy_match` on the display
  name.
- Commands are grouped by category, each with a count.
- Each row shows:
  - the command name;
  - an "edited" pill when the row differs from the preset;
  - "unbound by preset" where that applies;
  - its chords as keycaps, with alternates after a "·", or "—" when there
    are none;
  - a Rebind button, plus a Reset button on edited rows.

  The conflicting chord gets a `KeycapTone::Conflict` ring in its owner's
  row.
- The list is wrapped in `lazy`, keyed on the filter, the capture state, the
  conflict and the keymap settings.

**Rebinding.** Rebind sets `keymap_editor.capturing`. The next key press,
whatever it is and even if a widget captured it, is the new chord. This
happens in `update::keymap::key`, which runs before any other key handling.
Esc cancels the capture. Key repeat is ignored.

If the chord already belongs to another command in the same scope, a
conflict banner says "<chord> is already <owner>" and offers Cancel or
Replace. Replace gives the chord to the new command and takes it from the
owner.

While a capture or a conflict is open, the canvases ignore keys
(`canvas_keys_blocked`). Closing Settings cancels both.

**Persistence.** Every change calls `apply`. That rebuilds `r.ui.keymap`,
refreshes the panel's baseline and unbound list, and writes `settings.json`
straight away.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Presets | `resonance-app/src/commands/bindings.rs` | `KeymapPreset::{ALL, bindings, unbound, overrides}` |
| Persisted shape | `resonance-app/src/settings.rs` | `KeymapSettings`, `KeymapOverride`, `lenient`, `lenient_list` |
| Reducer | `resonance-app/src/update/keymap.rs` | `KeymapMsg`, `resolve`, `apply`, `push_override`, `handle`, `key` |
| Panel state | `resonance-app/src/state/keymap_editor.rs` | `KeymapEditorState`, `KeymapConflict`, `SettingsTab`, `preset_of` |
| View | `resonance-app/src/view/settings_keyboard.rs` | `view_keyboard_page`, `command_list`, `binding_row`, `PAGE_WIDTH` |
| Startup | `resonance-app/src/lib.rs` | `keymap: resolve(&settings.keymap)`, `KeymapEditorState::for_settings` |

## Surfaces

- **GUI:** Settings (⌘, or the ☰ button in the chrome) › Keyboard.
- **Control API / MCP:** none.
- **Agent skills:** none.

## How it's tested

| What | Where | Run |
|---|---|---|
| Override replay, rebind takes the next key, a taken chord asks first, Esc cancels only the rebind, reset and preset switching | `resonance-app/tests/performance/keymap_settings.rs` | `./scripts/run-tests.py -p resonance-app` |
| Preset tables resolve every command, the overrides take effect, presets differ from the defaults | `resonance-app/tests/performance/commands_registry.rs` | same |
| A corrupt keymap entry keeps the other settings; rebinding keeps alternates | `resonance-app/tests/performance/shortcut_review_fixes.rs` | same |
| Goldens | `resonance-app/tests/snapshots/keymap_settings_panel-wgpu.png`, `keycap_styles_sample-wgpu.png` (`performance/keycap_styles.rs`) | `RESONANCE_BLESS=1 cargo test -p resonance-app --test performance` |

Not covered: the conflict banner and the "unbound by preset" box have no
golden. Rebinding is only exercised through injected `ShortcutKey`s.

## Decisions and deviations

- The Ableton Live and FL Studio presets from the prototype were dropped.
  Their old tables were wrong or did nothing (Ableton put Record on Enter).
- A conflict can only replace. The prototype's "Keep both" option was not
  built, because the scope invariant forbids a chord that resolves to two
  commands.
- There are no "Done" or "Restore defaults" buttons. Edits save
  immediately, and "Reset all" restores the defaults.
- Command ids are variant names (`CommandId::key()`), not dotted paths.

## Known gaps

- The panel has no Unbind button. `KeymapMsg::Unbind` exists in the reducer,
  but nothing in the GUI sends it. A user can only free a chord by giving it
  to another command, or by choosing a preset
  ([intent](../../intents/ui-shell/settings-unexposed-options.md)).
- The prototype's Audio & MIDI, Appearance and Compose preference pages do
  not exist. Settings has General, Keyboard and MIDI
  ([settings.md](settings.md)).
