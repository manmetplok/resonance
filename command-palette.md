# Command palette, keyboard shortcuts and transport control commands

Status: **building** on `feat/command-palette` (2026-09-30), one phase per
slice (§9). The open decisions in §11 were never answered, so the build
takes the **recommended** option for each (D1–D7). Where the code and this
spec disagreed, the build followed the code and this document was corrected
to match; those notes are marked *(as built)*.

- **P0 landed.** The registry is the live dispatch.
- **P1 landed.** Transport and playhead control, with Space bound.

## 0. Why

Most of Resonance is reachable only by mouse. There are about 15 live
shortcuts, and Space, the most basic DAW key, does nothing. The Performance
footer even advertises "Space play · R record", and neither is bound. Some
features have **no GUI entry point at all**: the Export modal, Save as
Template, Add Marker at Playhead, Add Control Track, and the chord-track
actions.

This spec adds three things, all built on one command registry:

1. **A command palette** (⌘K / Ctrl+K). Every action can be found by typing,
   and each row shows its shortcut when it has one.
2. **A complete default keymap**, including Space for play/stop.
3. **Transport and playhead control commands** that don't exist yet. For
   example: move the playhead to the loop start, set the loop from the
   playhead, nudge by bar or beat, loop the selection.

On macOS "⌘" means Command. On Linux it means Ctrl: iced's
`Modifiers::command()` already maps it, and `KeyChord::from_iced` follows
that.

## 1. What already exists (verified against master @ 4f7a6baf)

| Piece | Where | Relevant facts |
|---|---|---|
| Command registry | `resonance-app/src/commands.rs` | Built for this purpose (ba todo #630, epic #58), but **nothing uses it outside tests**. It has 37 `CommandId`s in 6 `CommandCategory`s, with `display_name`, `breadcrumb`, `glyph` and `to_message()`. `KeyChord` handles parse/format and `from_iced`. There is `BindingMap::resonance_default()` and 5 `KeymapPreset`s (Ableton, Logic, Pro Tools, FL). `fuzzy_match` returns a score plus highlight ranges. `to_message` is parameterless by design. |
| Registry tests | `resonance-app/tests/performance/commands_registry.rs` | They pin names, uniqueness, the parse/format round-trip, that no chord maps to two commands, and fuzzy ranking. They check only a **subset** of the default table, so the conflicts below go uncaught. |
| Live shortcuts | `resonance-app/src/update.rs:318-405` `key_press_message` | A hand-written `match`: ⌘S, ⇧⌘S, ⌘O, ⌘Z/⇧⌘Z/⌘Y, ⌘G (group selection), ⌘F / ⇧⌘F (freeze selected / all), Enter (open the selected MIDI clip), F (Performance), Esc (exit Performance), `.` / `,` (next/prev marker). **The registry is not consulted.** |
| Momentary audition | `update.rs:409-440` | Holding B auditions the reference. Its subscription is attached only in the Mix view with the reference rail open and a reference selected. It uses press and release, so it is not a command. |
| Canvas-local keys | timeline `view/timeline/input/keyboard.rs`, MIDI editor `view/midi_editor/input.rs`, vocal roll `vocal_roll/canvas_program.rs`, expanded editor `compose/expanded_editor/mod.rs` | Delete/Backspace deletes the selection: automation point, then global event, then MIDI clip, then audio clip. In the MIDI editor, ⌘A selects all and ⇧⌘A selects the notes in view. In the vocal roll, `s`/`+` toggles slur. In the expanded editor, `+`/`-` zooms Y and Esc collapses. Each is gated by `KeyFocus::owns_keys()`: the canvas owns the keys only if the last mouse press landed inside it (`focus.rs:66-104`). Handlers `.and_capture()`, so the global subscription never sees those keys; `keyboard::listen()` only yields `Ignored` events. |
| Typing gate | `focus.rs`, `update/ui.rs:240-251` | `UiMessage::RequestShortcut(msg)` probes `any_text_input_focused()` and re-dispatches only when no text field is focused. `RequestPerformanceToggle` and `RequestMarkerNav` are older hand-rolled copies of the same pattern. |
| Key repeat | `update.rs:232` | The subscription ignores `KeyPressed.repeat`, so **holding F toggles Performance mode over and over**. iced 0.14's `key` is the key without modifiers (`modified_key` has them applied), so Shift+`[` arrives as `[` + shift. That is what we want for chords. |
| Transport | `update/transport.rs:13-43` | `Play`, `Pause`, `Stop`, `Record`, `SkipBack`/`SkipForward` (**±5 s**, although the registry names them "Skip to Start/End"), `SeekToSample(u64)`, `ToggleLoop`, `SetLoopRange{loop_in, loop_out, enabled}`, `ToggleMetronome`, `CycleTimeSignature`. **There is no play/pause toggle.** The button chooses based on `transport.playing`. `Stop` always sets `playhead = 0`. `Record` silently does nothing unless a track is armed. In Compose view, `Play` with a selected placement loops that section. |
| Transport state | `state/transport.rs` | `playing`, `playhead: u64`, `loop_enabled`, `loop_in`, `loop_out` (samples). |
| Bars ↔ samples | `resonance-audio/src/types/tempo/bars.rs:314, 351` | `TempoMap::bar_to_sample(bar)` and `sample_to_bar(sample, sr) -> (bar, frac)`, both meter-aware. `view/timeline/snap.rs` has grid snapping. |
| Markers | `update/marker.rs` | `AddAtPlayhead` (**no GUI emitter**), `JumpToNext` / `JumpToPrev` / `JumpTo(id)`, `LoopToRegion(id)`. |
| Overlays | `view/mod.rs:104-209`, `state/modal.rs` | One if/else priority chain renders `stack![base, overlay]`, so **exactly one** root overlay shows at a time. Open state is scattered across `modals.*`, `ui.mixer.*_open` and others. **No overlay closes on Esc**, and **an open overlay does not gate global shortcuts**: F toggles Performance mode underneath the Settings dialog. |
| Keycap UI | `theme.rs:997-1140` | `kbd` glyph consts, `keycap` / `keycap_row` with `KeycapTone`, `active_row_style`, `edited_pill_style`, `conflict_ring_style`. Built for this palette and unused so far. |
| Design | `design/keyboard-shortcuts-command-palette/index.html` | The epic #58 prototype: a 640 px card anchored 96 px from the top, a search row, grouped results with glyph, name, breadcrumb and keycaps, Recent and Suggested sections when the query is empty, an empty state, and a footer with ↑↓ ↵ Esc hints. It also covers the Preferences › Keyboard rebinding panel. **This spec adopts that visual design as-is.** |
| Settings | `settings.rs` | `settings.json` under the config dir, `#[serde(default)]`. It has no keymap or recents fields. The Settings UI is a single 420 px column with no tabs. |
| Menu hints | `view/menus.rs:446, 522-547` | `track_menu_item(…, shortcut: Option<&str>)`. The only hints it shows are ⌘F / ⇧⌘F, as hardcoded strings. |

### 1.1 Conflicts between the registry and live behaviour

| Chord | Registry (`resonance_default`) | Live | Resolution |
|---|---|---|---|
| ⌘G | `ToggleGlobalTracks` | Create group from selection | Live wins. Global tracks moves to ⌥⌘G. |
| ⌘Y | — | Redo | Add it as a second binding for Redo (§3.3). |
| ⌘F / ⇧⌘F | — | Freeze selected / all | Add as commands. |
| `.` / `,` | — | Marker next/prev | Add as commands. |
| ⌘E | Export Chord Sheet | — | Reassign to Split at Playhead. The chord sheet becomes palette-only. |
| Enter | Open MIDI clip; the Ableton preset rebinds it to Record | Open MIDI clip | Enter stays Open. Presets are deferred (§8). |
| `Undo.to_message()` | Raw `Message::Undo` | Focus-gated | The registry carries the gate (§3.2). |

## 2. Goals and non-goals

**Goals**

- One registry is the single source for every command's name, category,
  search keywords, shortcut, typing gate, availability and message. The
  global key handler, the palette, tooltips, menu hints and the Performance
  footer all read from it. None of them hardcodes a chord.
- Every action that doesn't need a free-form argument can be found in the
  palette. That includes the actions that currently have no GUI entry point.
- A default keymap that covers transport, playhead and loop control,
  editing, views, tracks and project I/O (§5).
- Single-key shortcuts never fire while the user types, never fire under a
  modal, and never fire from key repeat unless the command wants repeat.

**Non-goals (this spec)**

- A user rebinding UI and DAW keymap presets. The prototype designs these,
  and the registry's types support them. They are deferred to §8 as phase 5.
- Exposing the palette over the control API. The agent already has typed
  MCP tools for every one of these actions.
- Menu bars. Resonance has none, and the palette replaces them.

## 3. Registry changes (`commands.rs` → `commands/` module)

### 3.1 Command metadata

Each `CommandId` gains:

| Field | Type | Purpose |
|---|---|---|
| `keywords()` | `&'static [&'static str]` | Search aliases. For example, "Play / Stop" has `["start", "transport", "spacebar"]`, and "Bounce to WAV…" has `["export", "render", "mixdown"]`. `fuzzy_match` runs over the name first, then the keywords. |
| `gate()` | `KeyGate::{Always, NotWhileTyping}` | See §3.2. |
| `repeat()` | `bool` | Whether key repeat re-fires it. It is **true only for nudges and zoom**. Toggles are never true. |
| `scope()` | `Scope::{Global, Timeline, MidiEditor, VocalRoll, ExpandedEditor}` | Where the binding is live. Canvas-scoped entries document the canvas-local keys, so the palette and a future cheat sheet can list them. Their handlers stay in the canvases, but they read the chord from the registry instead of matching literals (§4.3). |
| `availability(&Resonance)` | `Available::{Yes, No(&'static str)}` | Computed while the palette renders. The reason is shown on a dimmed row, e.g. "No track is armed" or "Select a clip first". It never hides the command, because a discoverable disabled command teaches the user what it needs. |
| `to_message(&Resonance)` | `Option<Message>` | This replaces the parameterless `to_message()`. Commands that act on the selection resolve it here (§3.4). `None` means not available. |

`CommandId::ALL` stays the palette order. The hardcoded length check in
the tests (`ALL.len() == 37`) turns into a check derived from the enum.

### 3.2 Gating rules (a test enforces them)

- **Any chord without ⌘ or Ctrl is `NotWhileTyping`**: single letters,
  Space, Enter, arrows, `[`, `]`, `.`, `,` and Delete. Undo and Redo are
  `NotWhileTyping` too, since iced has no text undo (UPD-11). An invariant
  test checks this across the whole default map, so a new single-key
  binding cannot land ungated.
- Gated commands go through **one** `RequestShortcut` path. The copies in
  `RequestPerformanceToggle` and `RequestMarkerNav` are deleted and
  their commands route through `RequestShortcut` like everything else.
- **While any root overlay is open, only ⌘/Ctrl chords and Esc dispatch.**
  To support this, add `Resonance::root_overlay() -> Option<Overlay>`. It
  is derived from the same priority order as `view/mod.rs`, so the gate and
  the renderer can't disagree. Both use it. This fixes the "F under
  Settings" bug.
- **Esc resolves in this order:** close the palette, then close the topmost
  root overlay (a new generic dismiss for each overlay, the same effect as
  a backdrop click), then exit Performance mode. Canvas-local Esc (cancel a
  drag, collapse the expanded editor) captures first, as it does now.
  *(As built:)* Esc does not close the startup screen or the bounce,
  mixdown and freeze progress modals; stopping a render takes the explicit
  Cancel button. The non-modal "Group selected" bar is a root overlay that
  gates nothing (`Overlay::blocks_keys`).
- *(As built:)* `view()` now draws the root overlay over the Performance
  shell too. Before, Performance mode returned early and hid every overlay,
  so the gate and the renderer would have disagreed there.

### 3.3 Several chords per command

`BindingMap` currently allows one chord per command. It needs a primary
chord (shown in the palette and tooltips) plus alternates. ⌘Y for Redo is
the only alternate in the defaults. The invariant that a chord never
resolves to two commands **within one scope** still holds. Keep
`entries: Vec<(CommandId, KeyChord)>` and allow duplicate ids;
`chord_for(id)` returns the first match, which is the primary.

### 3.4 Selection-based commands and undo

A command must produce **one undo entry**, as a click does. Two cases:

- **Single target** (the selected clip, section placement or marker): resolve
  the id in `to_message(&Resonance)` and emit the existing id-carrying
  message, e.g. `ClipMessage::SplitClipAt{clip_id, at: playhead}`.
- **Multi-target** (every selected track): add one `…Selected` reducer
  message that loops inside a single reducer call, e.g.
  `TrackMessage::ToggleMuteSelected`. Don't fan out N messages; that
  would create N undo entries. The existing `FreezeSelectedTracks`
  already follows this pattern.

## 4. Key dispatch

### 4.1 Global handler

`key_press_message` is replaced by *(as built)*:

```
event::listen_with → KeyPressed{key, modifiers, repeat, ..} + capture status
  → KeyChord::from_iced(&key, modifiers)
  → Message::Ui(UiMessage::ShortcutKey{chord, repeat, captured})
reducer (update/shortcuts.rs):
  → drop if captured (a focused field or a key-owning canvas used it)
  → Esc + modal root overlay → that overlay's dismiss message
  → modal root overlay + no ⌘/Ctrl → drop
  → r.ui.keymap.command_for(Scope::Global, chord)
  → drop if repeat && !cmd.repeat()
  → run_shortcut: availability, then the typing gate, then to_message(r)
```

The subscription closure can't capture state, so it forwards every key
and the map lookup runs in the reducer. It uses `iced::event::listen_with`
rather than `keyboard::listen`, because the palette needs Esc, which a
focused `text_input` captures (it unfocuses itself on Esc). The reducer
drops captured keys otherwise, which is what `keyboard::listen` did.

The typing gate is `UiMessage::ShortcutProbed{command, editing}`: the
focus probe resolves to it, and the command's message is built only then.
A bare chord is typing-gated whatever `gate()` says, so a preset or a
rebinding can't put an ungated letter on a command. `RequestShortcut`
stays only for the held-`B` audition, which is not a command.
`RunShortcut(cmd)` runs a command as a shortcut without a chord.
The active `BindingMap` is `r.ui.keymap`; phase 5 loads it from settings.

The test seam is `update::shortcuts::key_press_command(&BindingMap, &Key,
Modifiers) -> Option<CommandId>` (renamed from `key_press_message`,
because it returns a command, not a message).

`to_message(r)` returns `None` only when a command has no target to name.
`availability(r)` is the authority on whether a command can run; the
shortcut path drops an unavailable command without dispatching.

### 4.2 New named keys

`NamedKey` gains `Home`, `End`, `PageUp` and `PageDown`. `[`, `]`, `;`
and `'` already work as `ChordKey::Char`. Formatting becomes
**platform-aware**, so a Linux user never sees a ⌘ they can't press:

| | macOS | Linux / Windows |
|---|---|---|
| `format_for_platform()` | `⇧⌘S` (glyphs, current `format_glyphs`) | `Ctrl` `Shift` `S` as separate keycaps |
| Tooltip text | `Save (⌘S)` | `Save (Ctrl+S)` |

### 4.3 Canvas-local keys

The canvases keep their own handlers. Ownership depends on `KeyFocus`,
which the app subscription can't see. Each handler replaces its literal
key match with `bindings.command_for(Scope::Timeline, chord)`, so a
canvas-local binding is listed in the palette and a later rebinding UI
covers it. Two scope rules, each enforced by a test:

- A canvas-scoped chord **may** shadow a global chord. It captures, and the
  global handler never sees it. For example, Esc in the expanded editor
  collapses it rather than exiting Performance mode. The palette shows the
  global meaning and lists the canvas meaning under the canvas's section.
- Running a canvas-scoped command from the **palette** needs a target
  without keyboard ownership: "Delete Selected Notes" uses the open MIDI
  editor's selection. It is available only when that surface is open and
  has a selection.

## 5. Default keymap

Legend: **L** = live today and unchanged. **N** = new chord for an existing
message. **NEW** = needs a new message or reducer (§6). All chords without
⌘ are `NotWhileTyping`.

### 5.1 Transport

| Command | Chord | | Notes |
|---|---|---|---|
| Play / Stop | `Space` | NEW | `TogglePlay`. If stopped, play. If playing, stop and **return to where playback started** (D1). |
| Play / Pause (stop in place) | `⇧Space` | NEW | Stop without moving the playhead (existing `Pause`). *(As built:)* resolved in `to_message`: `Pause` while playing, `Play` when stopped, so no new message. |
| Play from Loop Start | `⌥Space` | NEW | Seek to `loop_in`, then play. |
| Stop and Return to Zero | — (palette) | L msg | The existing `Stop`. The ■ button keeps this behaviour. |
| Record | `R` | N | Not available when no track is armed ("Arm a track to record"). |
| Toggle Loop | `L` | N | |
| Toggle Metronome | `K` | N | Matches Logic. M goes to mute (D2). |
| Cycle Time Signature | — (palette) | L msg | Rarely used, so it doesn't need a single key. |
| Tap Tempo | — | — | Out of scope. It is noted here so nobody spends `T` on it. |

### 5.2 Playhead and loop control (the requested "control commands")

| Command | Chord | | Behaviour |
|---|---|---|---|
| Playhead to Project Start | `Home` | NEW | Seek to 0. Also `⌘←`, replacing the ±5 s skip. |
| Playhead to Project End | `End` | NEW | Seek to the end of the last clip, section placement or marker, whichever is last. Also `⌘→`. |
| **Playhead to Loop Start** | `[` | NEW | Seek to `loop_in`. Works whether or not the loop is enabled. Not available when the loop range is empty (`loop_in == loop_out`). |
| **Playhead to Loop End** | `]` | NEW | Seek to `loop_out`. |
| **Set Loop Start at Playhead** | `I` | NEW | `SetLoopRange{loop_in: playhead, …}`, snapped to the grid (D6). If the result has `in >= out`, `out` becomes `in + 1 bar`. Does not enable the loop. |
| **Set Loop End at Playhead** | `O` | NEW | The mirror of Set Loop Start. If `out <= in`, `in` becomes `out − 1 bar`, clamped at 0. |
| **Loop Selection** | `⌘L` | NEW | Sets and enables the loop from the current selection, trying these in order: selected clips (their union), the selected section placement, the selected marker region. Not available with no selection. |
| Loop Section at Playhead | `⇧L` | NEW | The section placement under the playhead. |
| Nudge Playhead Back / Forward 1 Bar | `←` / `→` | NEW | Bar-aligned through the tempo map, so it is meter-aware. An off-grid playhead first snaps to the bar line **in the direction of travel** *(as built: "nearest" would move → backwards)*. Repeats while held. |
| Nudge Playhead Back / Forward 1 Beat | `⌥←` / `⌥→` | NEW | Uses the beat unit of the signature in force at the playhead (a 7/8 beat is an eighth). Same off-grid rule. Repeats while held. |
| Previous / Next Marker | `,` / `.` | L | |
| Previous / Next Section Start | `⇧,` / `⇧.` | NEW | Jumps across section placement starts. |
| Add Marker at Playhead | `⇧M` | N | Gives the existing orphan `MarkerMessage::AddAtPlayhead` its first user-facing entry point. *(As built: lands in P1 with the other §5.2 keys.)* |
| Go to Bar… | `⌘J` | NEW | Opens the palette in `:` mode (§7.4). |
| Rewind / Fast-forward 5 s | — (palette) | L msg | The existing `SkipBack` / `SkipForward`, renamed to match what they actually do. |
| Toggle Follow Playhead | — (palette) | L msg | |

All seeks go through `TransportMessage::SeekToSample`. While playing, the
engine seeks live, which is already the case. Seeks and nudges are
`UndoAction::Skip`. Setting the loop is `Record`, as `SetLoopRange` is
now. A held `→` coalesces into one undo entry by default because it only
seeks; it creates no entry.

### 5.3 Editing

| Command | Chord | | |
|---|---|---|---|
| Undo / Redo | `⌘Z` / `⇧⌘Z` (+ `⌘Y`) | L | |
| Delete Selection | `⌫` / `Del` | L (canvas) | The timeline, MIDI editor and vocal roll entries become registry-scoped entries. |
| Split Clip at Playhead | `⌘E` | N | `SplitClipAt` currently has no GUI emitter. For takes, it uses `SplitCompAtPlayhead`. |
| Duplicate Selection | `⌘D` | NEW | Clip/placement duplicate placed right after the original. Check first whether the control API already has a primitive to reuse. |
| Select All Notes | `⌘A` | L (MIDI editor) | |
| Quantize Selected Notes | `Q` | N | Uses the current quantize panel settings. |
| Open Selected MIDI Clip | `↵` | L | |
| Close MIDI Editor | `⌘Esc` | N | |
| Toggle Slur | `S` / `+` | L (vocal roll) | Canvas-scoped. It shadows the global `S` (solo) only while the vocal roll owns the keys. |

### 5.4 Tracks and mixer

| Command | Chord | | |
|---|---|---|---|
| Add Audio / Instrument Track | `⌘T` / `⇧⌘T` | N | |
| Add Track… (menu) | `⌥⌘T` | N | |
| Mute / Solo Selected Tracks | `M` / `S` | NEW (`…Selected`) | D2 |
| Arm Selected Tracks | `⇧R` | NEW (`…Selected`) | |
| Delete Selected Tracks… | `⌘⌫` | N | Goes through the existing confirm dialog. |
| Group Selection | `⌘G` | L | |
| Freeze Selected / All | `⌘F` / `⇧⌘F` | L | |
| Toggle Master FX Bypass | — (palette) | L msg | |
| Add Bus | — (palette) | L msg | |

### 5.5 Views and panels

| Command | Chord | | |
|---|---|---|---|
| Arrange / Mix / Compose | `⌘1` / `⌘2` / `⌘3` | N | |
| Toggle Performance Mode | `F` | L | Now ignores key repeat. |
| Zoom In / Out | `⌘=` / `⌘-` | N | Repeats while held. |
| Toggle Browser | `⌥⌘B` | N | |
| Toggle Global Tracks | `⌥⌘G` | N | Moved off ⌘G (§1.1). |
| Toggle Reference Panel, Markers Overview, Rail Panel | — (palette) | L msg | |

### 5.6 Project

| Command | Chord | | |
|---|---|---|---|
| Command Palette | `⌘K`, `⇧⌘P` | NEW | |
| New / Open / Save / Save As | `⌘N` / `⌘O` / `⌘S` / `⇧⌘S` | L/N | |
| Bounce to WAV… | `⌘B` | N | |
| Export Stems / MIDI… | `⇧⌘E` | N | Gives the currently unreachable `ExportMessage::Open` an entry point. |
| Import MIDI… | `⌘I` | N | |
| Import Audio to Pool… | `⇧⌘I` | N | |
| Save as Template… | — (palette) | L msg | Its first entry point (orphan). |
| Export Chord Sheet… | — (palette) | L msg | |
| Settings… | `⌘,` | N | |
| Rescan Plugins, Relink Missing Media, Show Missing Plugins | — (palette) | L msg | |

### 5.7 Deliberately unbound

Hold `B` (reference audition) stays a subscription, because press/release
doesn't fit the command model. It is still listed in the palette's help
text. `T`, `P`, `U`, `W`, `Y`, `X`, `C`, `V` and the digits stay free for
tools and edit modes later.

## 6. New messages

Build each one with a reducer test. The spec names them; the build may
adjust the names, as long as the undo classification below holds.

| Message | Undo | Notes |
|---|---|---|
| `TransportMessage::TogglePlay` | Skip | Adds `TransportState.play_start: u64`, recorded by Play, Record and TogglePlay. On stop it sends the engine `Stop` (which also ends a recording pass), then `SeekTo(play_start)`. In Compose view the existing Play branch still auto-loops the selected section; `play_start` is recorded after that seek. |
| `TransportMessage::PlayFromLoopStart` | Skip | |
| `TransportMessage::SeekTo(SeekTarget)` | Skip | `SeekTarget::{ProjectStart, ProjectEnd, LoopStart, LoopEnd, NudgeBars(i32), NudgeBeats(i32), PrevSection, NextSection, Bar{bar, beat}}` (1-based, so `:17.3` maps straight onto it). One reducer (`update/transport_nav.rs`), so all the seek maths lives in one place and can be tested. It resolves through `TempoMap` and sends the engine `SeekTo`, as `SeekToSample` does. |
| `TransportMessage::SetLoopPoint{edge: LoopEdge}` | Record | *(As built)* the playhead case only. Snapping and the swap/clamp rules from §5.2 live here. Loop Selection and Loop Section at Playhead set both edges at once, so they resolve their range in `to_message` and emit the existing `SetLoopRange{…, enabled: Some(true)}` (the §3.4 single-target rule) instead of a `LoopAt` variant. |
| `TrackMessage::{ToggleMuteSelected, ToggleSoloSelected, ToggleArmSelected}` | Record | Mixed state resolves to "all on" when any selected track is off, which matches the group macro behaviour. |
| `UiMessage::{OpenPalette(PaletteMode), ClosePalette, Palette(PaletteMsg), RunShortcut(CommandId), DismissOverlay}` | Skip | |

## 7. The palette

### 7.1 Look

Adopt the epic #58 prototype. The card is 640 px, anchored 96 px from the
top, `BG_2` with `RADIUS_XL`. The search row has a large text field and an
`Esc` keycap. Results are grouped by `CommandCategory` with a small caps
header. Each row has a glyph tile, the name with the fuzzy-matched ranges
in `ACCENT_SOFT`, a dimmed breadcrumb, and the shortcut as a `keycap_row`
on the right: `Neutral` normally, `Active` on the selected row. The footer
shows `↑↓ navigate · ↵ run · Esc close` and a result count. All of these
are existing `theme.rs` helpers.

```
┌──────────────────────────────────────────────────────────────┐
│ ⌕  loop st                                              Esc  │
├──────────────────────────────────────────────────────────────┤
│ TRANSPORT                                                    │
│▌⤒  Playhead to Loop Start                               [ [ ]│  ← active row
│    Transport › Playhead                                      │
│ ⟲  Set Loop Start at Playhead                           [ I ]│
│    Transport › Loop                                          │
│ ▶  Play from Loop Start                           [ ⌥ ][Spc] │
│ VIEW & NAVIGATION                                            │
│    Loop Selection                    Select a clip first ⌘ L │  ← dimmed + reason
├──────────────────────────────────────────────────────────────┤
│ ↑↓ navigate   ↵ run   Esc close                   4 results  │
└──────────────────────────────────────────────────────────────┘
```

On Linux the keycaps read `Ctrl` `L`, per §4.2.

### 7.2 States

- **Empty query.** Show *Recent* (the last 8 commands run, from the palette
  or a shortcut, newest first, de-duplicated, persisted in `settings.json`)
  and then *Suggested for this view*. Suggestions are a short static list
  per `ViewMode`: Arrange suggests loop, split, add track; Mix suggests
  add bus, bypass master FX, bounce; Compose suggests new section, loop
  section; Performance suggests play/stop and exit.
- **Query.** `fuzzy_match` on the name, then on the keywords at a lower
  weight. Rank by score, then a recent-use boost, then available before
  unavailable, then registry order. Results group by category in
  `CommandCategory::ALL` order, and categories are ordered by their best
  hit. Ranking must be deterministic, because tests pin it.
- **No match.** Show the prototype's empty state: *No commands match "q"*.
- **Unavailable row.** Shown dimmed with its reason. Enter on it does
  nothing and flashes the reason. It is never hidden.

### 7.3 Behaviour

- **Opening.** ⌘K or ⇧⌘P opens the palette with the query focused, using
  `text_input::focus(id)`. The palette is the top entry in the overlay
  priority chain, beneath only the recovery, startup and progress overlays,
  because running commands from those is not meaningful. Opening it while
  another overlay is open closes that overlay first.
- **Keys.** ↑/↓ move the selection and wrap around. They arrive through
  the global subscription as `Palette(Move)` because `text_input` ignores
  vertical arrows. ↵ runs the selected command via `on_submit`. Esc closes.
  Mouse hover selects a row and click runs it.
- **Running a command.** Close the palette first, then dispatch
  `to_message(r)` **directly, bypassing the typing gate**. The palette's
  own field was focused, and the user picked the command explicitly. It
  still goes through every other gate, such as startup, bounce and freeze.
- **Blocking canvas keys (this is a real hazard).** `focus.rs` assumes the
  app never focuses a field programmatically. The palette breaks that
  assumption. Keyboard events reach every widget no matter how the stack
  is layered, so a timeline that owned the keys before ⌘K would act on
  Backspace typed into the palette and **delete the selected clip**. So
  each canvas `Program` gets a `keys_blocked: bool`, set from
  `r.ui.palette.is_some()`, and its key handler returns early when it is
  set. A test pins this (§10).
- **Keeping state.** Closing the palette keeps the last query, pre-selected
  so typing replaces it. That makes "run it again" cheap.

### 7.4 Argument modes (phase 4)

A leading prefix character switches the palette's data source. Only the
command mode is phase 2.

| Prefix | Mode | Rows | Run |
|---|---|---|---|
| (none) | Commands | the registry | the command |
| `:` | Go to bar | "Go to bar 17" (or `17.3` for a beat), parsed live | `SeekTo(Bar)` |
| `@` | Markers and sections | each marker and placement, with its bar | `MarkerMessage::JumpTo(id)` / seek |
| `#` | Tracks | each track | select and scroll to it |
| `+` | Add plugin | the plugin catalog, filtered to the selected track's kind | add to the selected track's chain |

⌘J opens the palette in `:` mode. The footer shows the prefixes when the
query is empty.

## 8. Shortcut hints everywhere (phase 5, and the deferred items)

- **Tooltips.** Transport buttons, view tabs and header buttons get a
  `tooltip` built from `display_name` plus the platform chord.
  `track_menu_item`'s `shortcut` argument is filled from the registry,
  replacing the literal "⌘F".
- **Performance footer.** `view/performance/mod.rs:797-803` builds its
  hint from the registry, which makes the "Space play" hint true.
- **Rebinding and presets** (the prototype's Preferences › Keyboard panel):
  the settings gain `keymap: { preset, overrides: Vec<(CommandId, Option<KeyChord>)> }`.
  This needs a stable string id per command (`CommandId::key() -> &'static str`,
  e.g. `"transport.toggle_play"`) so saved overrides survive enum reordering.
  The Settings view first needs a nav rail (it is a single column today).
  The DAW presets in `commands.rs` stay in the code, unexposed, until then
  (D5).

## 9. Build plan (vertical slices)

| Phase | Slice | Lands |
|---|---|---|
| P0 | **Registry becomes the live dispatch** | §3 metadata, gates, multi-chord `BindingMap`, `RunShortcut`, the repeat filter, `root_overlay()` gating and generic Esc dismiss. `key_press_message`'s table moves into `resonance_default` with **exactly the live behaviour** (§1.1 resolutions), plus a parity test. The two hand-rolled gates are removed. Nothing visible changes except the F-repeat fix, the fix for shortcuts firing under overlays, and Esc closing overlays. |
| P1 | **Transport and playhead control** | The §6 transport messages, `play_start`, `SeekTarget`, `SetLoopPoint`, the `Home`/`End` named keys, and the §5.1 / §5.2 bindings including Space. Once this lands, the Performance footer hint is true. |
| P2 | **The palette** | The overlay, the §7.1–7.3 states, recents persisted, `keys_blocked` on every canvas, platform formatting, ⌘K / ⇧⌘P, and golden snapshots. |
| P3 | **Selection commands and orphans** | The `…Selected` track messages, split/duplicate at playhead, loop selection, quantize. Entry points for Export, Save as Template, Add Marker, Add Control Track, and the chord-track actions. The rest of the §5.3–5.6 bindings. |
| P4 | **Argument modes** | `:` `@` `#` `+` (§7.4), and ⌘J. |
| P5 | **Hints and rebinding** | Tooltips, menu hints and the footer read from the registry. The keymap in settings, the Preferences › Keyboard panel, and presets. |

Each phase can be merged on its own. P1 does not depend on P2: the
control commands are useful as plain shortcuts before the palette exists.

## 10. Tests

These follow CLAUDE.md. **No new files under `resonance-app/tests/`.**
Registry and dispatch tests go next to `tests/performance/commands_registry.rs`
in the `performance` group. Transport reducer tests go in the group that
already holds transport tests. Palette goldens go with the other iced_test
snapshots, blessed with `RESONANCE_BLESS=1`. Every app test uses
`Resonance::new_for_test()`, and `new_for_test_with_capture()` where a test
asserts an engine `AudioCommand`.

**P0**
- **Parity.** For every chord that the pre-refactor `key_press_message`
  handled, the new dispatch yields the same `Message`, with the same typing
  gate. Write this test against the old function **before** removing it,
  per the memory note that refactor goldens must be checked against
  pre-refactor code.
- **Gate invariant.** Every default binding without ⌘ or Ctrl is
  `NotWhileTyping`. No two commands share a chord in one scope. Every
  toggle has `repeat() == false`.
- **Overlay gate.** With Settings open, `F` and `L` dispatch nothing, while
  `⌘S` still saves and `Esc` closes Settings.
- A repeated `F` toggles Performance mode exactly once.

**P1**
- `TogglePlay`: play, move the playhead, TogglePlay again. The playhead
  returns to `play_start`, and the capture shows `Play`, `Stop`/`Pause` and
  `SeekTo(play_start)`.
- `SeekTo` for each target, on a project with a 4/4 → 7/8 signature
  change. Bar and beat nudges land on the grid lines the ruler draws; the
  expected values come from `TempoMap::bar_to_sample`.
- `SetLoopPoint`, both edges, including the crossing cases (in ≥ out and
  out ≤ in). Each call is one undo entry, and undo restores the prior range.
- `Space` while typing in a text field dispatches nothing.

**P2**
- Reducer: open, type a query, ↓↓ ↵ runs the expected command and closes
  the palette. Esc closes it and keeps the query. An unavailable command
  doesn't run. Ranking for a fixed set of queries is pinned, e.g. "loop st"
  → Playhead to Loop Start first, and "sav" → Save first.
- **Canvas block.** Select a clip, open the palette, send `Backspace`. The
  clip still exists.
- Goldens: the empty state (recents plus suggested), results with
  highlighting and one unavailable row, the no-match state, and Linux
  keycap formatting.

**P3 / P4.** Each new `…Selected` message yields one undo entry for three
selected tracks. Each orphan command reaches its modal or reducer. The
`:17` parse and `@` / `#` row building each get a test.

## 11. Open decisions (each has a recommendation)

| # | Question | Recommendation |
|---|---|---|
| D1 | When Space stops playback, where does the playhead go? | **Back to where playback started** (Logic/Pro Tools style), which suits looping a section. `⇧Space` stops in place; the ■ button still returns to zero. Alternative: stop in place, as Ableton does. |
| D2 | What do `M` and `S` do: mute/solo selected tracks, or metronome/stop as in the current registry? | **Mute/solo**, and metronome moves to `K`. Space covers stop, so a single-key stop is redundant. |
| D3 | Which chord opens the palette? | **Both ⌘K and ⇧⌘P.** They are free and cover both conventions. |
| D4 | Unavailable commands: dim them or hide them? | **Dim them and show the reason.** That is how users learn "arm a track to record". |
| D5 | Should the DAW keymap presets ship with the palette? | **No, only in P5** with the rebinding UI. A preset without a way to view or fix it (Ableton's Enter = Record) just breaks keys silently. |
| D6 | Do Set Loop Start/End at Playhead snap to the grid? | **Snap to the visible grid** (`snap_sample_to_grid_tempo`). Nudges are always exactly one bar or beat. |
| D7 | Should `←`/`→` move the playhead globally, or be kept for future note and clip nudging in the canvases? | **Global playhead nudge.** The canvases can shadow the arrows later under their scope rule (§4.3) when they own the keys. |
