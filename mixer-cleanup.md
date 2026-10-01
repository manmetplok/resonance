# Mixer tab cleanup: slim strips, one editing surface in the inspector

Status: **spec, decisions answered, not started** (2026-10-01, written
against `feat/drums-rework` @ 789cf9a4, whose mixer code matches master).
Q1–Q16 are decided (§9). Delivery is **one branch, one landing**
(`feat/mixer-cleanup`, §8).

Touches `resonance-app/src/view/mixer/**` (strips, inspector, plugin
panel), `resonance-app/src/state/{mixer,tracks}.rs`, `theme.rs` (strip
widths), the project model (a new per-track colour), and the mixer goldens
in `resonance-app/tests/snapshots`. The control-API change is limited to the
colour slice (§7).

## 0. Why

The user said: "Its a bit confusing the plugins show both in the
channelstrip and in the inspector, but you can do different things. Also in
the channelstrip long names break into two lines, creating a sloppy layout.
Most functionality should be in the inspector (adding, open window, move
order, etc). channelstrips can be a bit wider to provide space but only show
if FX is active or not."

That is accurate. Today there are **two editing surfaces with different
verbs**:

| Action | Strip plugin row (`mod.rs::view_plugin_slot_row`) | Inspector CHAIN (`inspector/chain.rs`) |
|---|---|---|
| Open params | click → bottom `plugin_panel` | — |
| Reorder | ▲▼ | ▲▼ |
| Bypass | ⏻ | `BYP` |
| Remove | × | — |
| Preset | preset glyph | — |
| Add | `+ Instrument` picker (empty instrument slot only) | `+ FX` / `+ Instrument` picker |

The other problems:

- **Plugin names wrap.** At 140 px (`MIXER_STRIP_WIDTH`), "Resonance Drums"
  wraps to "Resonance / Dr..", and the ▲▼⏻× cluster gets squeezed against
  the right edge. The name pill and the action icons fight for about 90 px.
- **Strip clutter.** Every strip has a full-width `+ Automation` picker, a
  second utility row (mono / FX bypass / bounce) and a left-labelled pan
  row. Little of that is used per strip, per glance.
- **The inspector repeats the strip.** SIGNAL shows peak / RMS / pan / out,
  all of which the strip already shows.
- **Bottom panel.** Clicking a plugin opens a parameter panel under the bus
  row, which eats the strips' height. It is a third surface for the same
  plugin.
- **Master has no inspector**, and its strip shows a bare "FX" label.
  Busses use their own strip anatomy (trash icon on the strip).

## 1. The principle

> **The strip shows state; the inspector edits.** A strip answers "what is
> on this channel and is it active?" at a glance, and keeps only the
> controls you ride live (M / S / ● / 🎧, the chain on/off switch, pan,
> fader). Every structural edit (add, remove, reorder, replace, preset,
> open, automation lanes, routing, sends, track options) lives in exactly
> one place: the inspector.

Selecting any strip (track, sub-track, bus, master) puts it in the
inspector.

## 2. Track strip anatomy

Width **160 px** (Q2; `MIXER_STRIP_WIDTH` 140 → 160). Top to bottom:

```
┌──────────────────────┐
│▌ ♫  Synth Bass       │  head: colour band (Q12), glyph, single-line name
│  M   S   ●   🎧      │  one button row
│ FX ⏻ ───────────────  │  FX header = chain on/off switch (Q6)
│ ● Resonance Wave     │  instrument line: accent colour, hairline under (Q14)
│ ─────────────────    │
│ ● Comp               │  FX lines: neutral text
│ ○ Granular Del…      │  ○ = bypassed; single line, ellipsis
│                      │  (scrolls if long; the fader never moves)
│        ( ◠ )         │  pan knob, centred (Q7)
│          C           │  pan value under it
│  ▮▮ ┃                │
│  ▮▮ ┃  fader+meter   │  unchanged live block
│  0.0                 │
└──────────────────────┘
```

### 2.1 Slot lines (Q1, Q13, Q14)

- One line per chain slot: a state dot and the plugin name, `Wrapping::None`
  in a clipped `Fill` container, with an ellipsis from `util::short` sized
  for 160 px (about 18 characters at size 10). **Never two lines.**
- Dot states: `●` active, `○` bypassed (dimmed name), BAD-pink `●` for a
  missing or unavailable plugin (`availability.reason().is_some()`). When
  the whole chain is bypassed (`track.fx_bypassed`), every line dims and the
  FX header switch reads off.
- Instrument slot (index 0 on a non-external instrument track): accent
  colour with a hairline under it. An empty instrument slot shows a dim
  `No instrument` line. Clicking it selects the track and focuses the
  inspector's add picker.
- **Click** selects the owner and focuses that slot in the inspector's CHAIN
  (`MixerUiState::focused_slot: Option<PluginInstanceId>`, which replaces
  `selected_plugin`), scrolling it into view and highlighting the row.
- **Double-click** opens the plugin's window (§4).
- No ▲▼⏻× on the strip. The `+ Instrument` pick_list leaves the strip.

### 2.2 Removed from the strip

| Removed | New home |
|---|---|
| ▲▼ / ⏻ / × / preset glyph on slots | inspector CHAIN row (§3.2) |
| `+ Instrument` picker | inspector CHAIN add picker |
| `+ Automation` header (`automation::automation_header`) | inspector AUTOMATION (§3.4) |
| mono button | inspector TRACK (§3.5) |
| bounce button | inspector TRACK; Arrange context menu unchanged |
| FX bypass button in the utility row | becomes the FX header switch on the strip (Q6) |
| `Pan` label | gone; value centred under the knob |

The live tints stay: automated gain on the fader and automated pan on the
knob still show while a Read lane drives them.

### 2.3 Head (Q12)

- A colour band on the left edge of the head, in the track's colour (§6).
- Double-clicking the name starts an inline rename (a `text_input` in place
  of the name; Enter or blur commits, Esc cancels). It dispatches the
  existing rename message, so undo and the `track_rename` control path stay
  shared.
- The external-instrument `Ext` pill and offline flag stay. The three
  summary chips (`ext_summary_chips`) stay too: they are state, not editing.

### 2.4 Sub-track strips (Q15)

Same anatomy, slimmed: single-line name, M / S, FX switch, centred pan,
fader. `MIXER_SUB_STRIP_WIDTH` 92 → **104 px** (scaled with the parent).
The collapsed parent's mini-meter column is unchanged.

## 3. Inspector

### 3.1 Order (Q9, Q10)

For a track:

```
INSPECTOR
Synth Bass        ■ colour  · Inst
─ CHAIN ───────────────────────
─ SENDS ───────────────────────
─ ROUTING ─────────────────────   input · MIDI in · output · MIDI out
─ AUTOMATION ──────────────────
─ TRACK ───────────────────────   mono · bounce · external hardware
```

- **SIGNAL is dropped.** The output destination moves into ROUTING (it is
  already a picker there). `MixerInspectorGroup` becomes
  `{Chain, Sends, Routing, Automation, Track}`. Collapse state carries over
  per group as it does today.
- SENDS is lifted out of ROUTING into its own group (`inspector/sends.rs`
  already is its own module).
- The "External hardware instrument" button becomes the TRACK group's
  external section. The existing `external_instrument.rs` group is
  rendered there and is no longer a free-floating button.
- The header gets the colour swatch (it opens a palette popover) and the
  track type tag. The name in the header is also rename-on-double-click.

### 3.2 CHAIN row (Q4)

```
⠿ ● Resonance Wave            ↗  ☰  ×
```

| Element | Action |
|---|---|
| `⠿` handle | drag to reorder |
| `●` | bypass toggle (replaces `BYP`) |
| name | click = focus; double-click = open |
| `↗` | open window (§4) |
| `☰` | slot menu: presets (load / save / browse, the strip glyph's current actions), **Replace…**, Remove |
| `×` | remove |

- **Drag reorder.** `reorder.rs` currently says "Drag-and-drop is
  deliberately out of scope"; this spec brings it in scope. The drop
  targets obey the same rules as `reorder::chain_moves`: the instrument
  floor (slot 0 is fixed on instrument tracks) and owner-local moves only.
  ▲▼ stay as keyboard/a11y fallbacks inside the `☰` menu (Move up / Move
  down), so the existing reorder tests keep a non-drag path.
- **Replace…** opens the add picker in "replace" mode. It is wired to the
  existing `update/plugin_replace.rs` (slot position kept).
- A focused row (from a strip click, §2.1) gets the accent-line border.
- Add picker at the bottom (`+ Add FX…` / `+ Add instrument…`), unchanged
  in logic.
- **Missing plugin** (Q16): the row shows its recovery actions inline under
  it (Replace / Remove / Locate). These are the actions
  `plugin_panel::missing_plugin_body` offers today.

### 3.3 Bus and master

- **Bus** (Q11): the bus inspector (`inspector/bus.rs`) gets the same
  CHAIN / AUTOMATION groups, and a **Delete bus** action in a BUS group
  (the trash icon leaves the strip).
- **Master** (new): clicking the master strip selects it
  (`MixerUiState::selected_master: bool`, mutually exclusive with track and
  bus selection, like `selected_bus`). The inspector shows CHAIN +
  AUTOMATION + a MASTER group holding **Bounce**, which moves off the
  master strip.

### 3.4 AUTOMATION (Q5)

The group lists the owner's lanes, one row each: target name, Read toggle,
and a "show in Arrange" jump. A `+ Add lane` picker uses the same option
source as today's strip header (`automation_header`'s plugin params +
device params). It reuses the `AutomationMessage`s; nothing new on the wire.

### 3.5 TRACK

Mono toggle, Bounce (instrument tracks; enabled per
`classify_bounce`), external hardware section (§3.1).

## 4. Plugin UI: the bottom panel goes (Q3, Q16)

`view/mixer/plugin_panel.rs` and the `mixer_col.push(panel)` slot in
`view_mixer` are removed. Strips and busses get the full height back.

**"Open" always opens a window:**

- A plugin with its own GUI: the existing `OpenPluginEditor` path.
- A plugin **without** a GUI: a host-drawn **generic parameter window**
  with the preset bar on top. This is today's generic parameter body and
  preset bar, moved, not rewritten.

> **Open question (implementation, not a product decision):** the app is
> single-window today (no `iced::window::open` anywhere). There are two ways
> to build the generic window: (a) a draggable in-app floating panel,
> layered like the palette/preset overlays, or (b) a real second iced window
> (a `daemon` migration). This spec assumes **(a)**: it meets "Open always
> opens a window" without changing the app's window model. Take (b) only if
> (a) can't keep focus or keyboard behaviour sane. Decide before slice S4.

The `TogglePluginPanel` message is retired. Tests that drive it
(`tests/mixer/mixer_generic_param_panel.rs`, `mixer_preset_bar.rs`,
`mixer_automation_controls.rs`) are retargeted at the generic window /
CHAIN row. They are not deleted, because the behaviour they guard still
exists.

## 5. Bus and master strips (Q11)

- **Bus strips** use the track anatomy: head with name, M / S, FX switch,
  slot lines, centred pan, fader. Width 160. Trash goes to the inspector.
  The bus row stays at the bottom (Q8); it now has the height the plugin
  panel used to take.
- **Master strip**: real slot lines instead of the bare "FX" label, an FX
  switch, and the fader. `Bounce` leaves the strip (§3.3). Click selects
  master.

## 6. Track colour (Q12)

New project-persisted field `TrackState::color: [u8; 3]` (the same shape
as `markers.rs`'s `color`).

- Auto-assigned on track creation by cycling a fixed palette in `theme.rs`
  (8–10 hues tuned against `BG_2`).
- Project load: a track without a colour gets one deterministically from
  its order, so old projects open coloured. No migration ceremony, since
  there are no external users.
- Edited from the inspector header swatch. The change is undoable.
- Used in the mixer strip head band and the **Arrange track header** (a
  matching band), so one track reads as one colour everywhere. A group's
  colour still wins as the cluster rail. The track band sits inside it.
- The colour field goes into `track_strip_fingerprint`.

## 7. Control API (Q16b)

No `mixer.*` changes; this cleanup is in the view. The one model addition,
colour, lands as a **vertical slice** in the same branch: a wire field on
the track read model, a `track.set_color` handler, and an MCP tool in
`resonance-mcp/src/tools/trackmix.rs` beside `track_rename` (which already
exists). The `agent_plugin_lockstep` test must still pass. No skill names
the new tool, so no plugin re-read is needed unless `PROTOCOL_VERSION`
bumps.

## 8. Delivery: one branch, one landing (Q16 phasing)

Branch `feat/mixer-cleanup` off master. It lands once, when every slice
below is green. The slices are ordered so that the **branch** never has an
unreachable function at a slice boundary (a review checkpoint):

| Slice | Content | Gate |
|---|---|---|
| S1 | Inspector restructure: groups, SIGNAL out, SENDS lifted, TRACK group, master selection + inspector | inspector goldens re-blessed |
| S2 | CHAIN row: bypass dot, open, `☰` menu (presets, replace, move, remove), missing-plugin inline recovery | every strip slot action reachable from CHAIN |
| S3 | AUTOMATION group | lane add/Read works from inspector |
| S4 | Generic parameter window + preset bar; remove bottom panel and `TogglePluginPanel` | retargeted panel tests green |
| S5 | Strip slimdown: 160 px, slot lines, FX header switch, centred pan, one button row; sub-strips 104 px | `mixer_*` goldens re-blessed |
| S6 | Bus + master strips on the new anatomy | bus/master goldens |
| S7 | Drag reorder in CHAIN | reorder tests incl. instrument floor |
| S8 | Track colour model + strip/arrange band + inspector swatch + `track.set_color` slice; inline rename | round-trip + undo tests |

Testing rules (CLAUDE.md):

- New tests go into the existing `resonance-app/tests/mixer` group binary,
  never as a new top-level file.
- Use `Resonance::new_for_test_on(Tab::Mixer)`.
- Re-bless with `RESONANCE_BLESS=1 cargo test -p resonance-app --test mixer`
  and look at every re-blessed PNG. Visual verification is the point of
  this spec.
- `cargo test -p arch-invariants` must still pass.
- Strip performance: the slot lines and FX switch live in the lazy body.
  Every new input (focused slot, colour, the chain-bypass dims) must be
  hashed in `strip_fingerprint.rs`, or the strip will show stale state
  (ui-work.md §11).

Acceptance (user-visible):

1. No plugin name on any strip wraps, at any name length.
2. Every plugin action (add, remove, reorder, bypass, replace, preset,
   open) is reachable from the inspector, and **none** except open
   (double-click) and focus (click) is on a strip.
3. A strip shows at a glance which plugins are on it and whether each one,
   and the chain as a whole, is active.
4. No bottom panel. "Open" always produces a window.
5. Track, bus and master strips share one anatomy.

## 9. Decisions

| # | Question | Decision |
|---|---|---|
| Q1 | Strip FX area | One line per slot: state dot + single-line name, no actions |
| Q2 | Strip width | 160 px |
| Q3 | Plugin UI location | Inspector "Open" → window; bottom panel removed |
| Q4 | CHAIN row actions | Drag reorder, remove, preset menu, replace/swap (plus bypass, open) |
| Q5 | Automation picker | Moves to an inspector AUTOMATION group |
| Q6 | Utility row | FX on/off stays on the strip as the FX header switch; mono + bounce → inspector |
| Q7 | Pan | Centred knob, value below |
| Q8 | Bus layout | Keep the bottom row |
| Q9 | Inspector order | Chain-first: CHAIN → SENDS → ROUTING → AUTOMATION → TRACK |
| Q10 | SIGNAL group | Dropped |
| Q11 | Extra scope | Bus strips match, master strip, inspector reorganise, rename + colour |
| Q12 | Colour source | New per-track persisted colour, auto palette, editable, shared with Arrange |
| Q13 | Slot click | Click = focus in inspector; double-click = open window |
| Q14 | Instrument slot look | Accent line + hairline divider; dim "No instrument" when empty |
| Q15 | Sub-tracks | Align with the new anatomy (104 px); collapsed meters unchanged |
| Q16 | Generic / preset / missing | Floating generic window with the preset bar; missing-plugin recovery inline in CHAIN |
| Q16b | Control API | No mixer API change; colour as a vertical slice |
| — | Sends on strip | Inspector only |
| — | Phasing | One branch, one landing (slices are review checkpoints only) |

Still open: the in-app panel vs. second OS window for the generic window
(§4). Decide before S4.
