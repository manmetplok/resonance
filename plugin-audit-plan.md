# Plugin audit — resolution plan

**Date:** 2026-08-15 · **Input:** `plugin-audit.md` (findings register, nothing fixed)
· **Goal:** resolve the great majority of the register, with the standing constraint
that **every capability we touch ends up reachable from both the GUI and MCP**.

---

## 0. The rule this plan is built on

The audit's three patterns all reduce to one failure: a capability exists on exactly
one surface. So every item below is scored against three surfaces, and none ships
until all three that apply to it are green:

| Surface | What "green" means |
|---|---|
| **DSP / backend** | the capability actually runs |
| **GUI** | a human can reach it — plugin editor, mixer inspector, or menu |
| **MCP / control API** | an agent can read *and* write it over `resonance-control` |

Three mechanical consequences, which is what makes this plan cheaper than the
finding count suggests:

1. **Param-first.** Anything expressed as a `FloatParam`/`IntParam`/`BoolParam` is
   *automatically* on all three surfaces: DSP reads it, the plugin editor draws it,
   and `track/bus/master.plugin_params` + `set_plugin_param` already expose it to
   MCP. So the default fix for "capability with no way in" is **add a parameter**,
   never "add an editor-only widget". This covers G1, G2, G3, M1, M2, M4, M6, D2,
   W3, A2, A4, I1 and X3 with zero new control methods.
2. **Vertical slices for anything that is not a param.** `resonance-mcp` enforces one
   tool per control method, so a wire-types-only todo can never pass the verify gate.
   Wire type + app handler + MCP tool land in **one** todo, branches stacked.
   (Recorded previously as `project_control_api_vertical_slices`.)
3. **Parity runs both ways.** The audit only looked for GUI-missing capabilities. The
   inverse exists too and is in scope — the clearest instance: **`track.set_fx_bypass`
   does not exist**, while `bus.set_fx_bypass` and `master.set_fx_bypass` do and the
   GUI has had the track button since forever (`view/mixer/track_strip.rs:164`).

### MCP ledger

What the control API has today vs. what this plan adds. Anything marked *new* is a
vertical slice per rule 2.

| Capability | MCP today | GUI today | Action |
|---|---|---|---|
| Plugin params (read/write) | ✅ `*.plugin_params`, `*.set_plugin_param` | ✅ editor + generic panel | extend the wire shape (below) |
| Chain reorder | ✅ `track/bus/master.move_effect` | ❌ **none** | GUI only (**P2**) |
| Aux sends | ✅ `track.add_send/set_send/remove_send` | ❌ two hardcoded dead rows | GUI only (**P3**) |
| Sidechain routing | ⚠️ `track.set_sidechain` (track-keyed, **not persisted**) | ❌ none | persistence + bus/master keys + GUI (**P4**) |
| Track FX bypass | ❌ **missing** (bus/master have it) | ✅ | *new* `track.set_fx_bypass` |
| Per-plugin bypass | ❌ | ❌ (dead "BYP" label) | *new* per-slot bypass, both surfaces (**X3**) |
| Save track as preset | ❌ | ❌ (dead backend) | *new* `track.save_preset` / `.presets` / `.apply_preset` (**P1**) |
| Plugin presets (load/save) | ❌ | ⚠️ load-only, 3 different combos | *new* `plugin.presets` / `.load_preset` / `.save_preset` (**X1/X2**) |
| Plugin rescan | ❌ | ❌ (startup only) | *new* `plugins.rescan` (**X10**) |
| Missing-plugin state | ❌ | ❌ (indistinguishable dead slot) | *new* `status` field + `track.replace_plugin` (**P5**) |
| Plugin/chain latency | ❌ | ❌ | *new* `latency_samples` on plugin entries (**X5/F2**) |
| Param units / labels / groups | ❌ bare floats and ints | ⚠️ editor-local label tables | extend `PluginParamView` (**X7/X8/X9/P8**) |
| Per-band GR, key-connected | ❌ | ❌ | editor viz + `meter.*` readout (**M5/C2/C3**) |

**`PluginParamView` gains** (one todo, unblocks a whole class): `text` (the plugin's
own formatter output — 100+ `with_value_to_string` call sites currently pay off only
in third-party hosts), `unit`, `module` (param group), `stepped`, `choices[]` (so an
agent can send `"Low-pass"` instead of `3`), `hidden`. Today an MCP client reading
wavetable's filter type sees `2.0` in `0..=4` with no way to learn what it means —
that is the same class of failure as the GUI findings, just on the other surface.

---

## 1. Waves

Ordered by what unblocks what, not by severity. Wave 0 and Wave 1 are the multipliers:
they make later waves smaller, and skipping them means fixing the same bug 11 times.

### Wave 0 — Stop the lying (all EASY, ~9 todos, no design gate)

Every item here is a control that misinforms. These read as bugs to a user and cost
almost nothing.

- **D3** drums KIT ROUTING label — the plugin declares all 7 ports unconditionally, so
  print the truth ("Multi-out · 7 ports") instead of a toggle showing the opposite.
- **W3** wavetable LFO segmented control — remove the dead third segment, rename
  "Sync" → "Retrig" (real tempo sync is Wave 3).
- **P7** resonance-delay factory presets — full 22-param snapshots; today loading
  "Slapback" leaves the previous patch's trance gate running.
- **P8** mastering `q` — `.with_unit` + `.with_value_to_string`, 4 bands × 2 stages.
- **P6** resonance-gate — call `theme::apply`; it is the only unthemed plugin.
- **X2 (display half)** — every preset combo shows the loaded preset name; unify the
  four plugins that hardcode "— select —".
- **D2 (delay)** gate_rate division label; wavetable glide seconds-vs-ms label; filter
  key-track drawn bipolar over a unipolar param; LFO shape bare integer.
- **Drums status bar** — invented CPU/RAM telemetry, permanently dead OUT meter, faked
  sample filename and waveform: remove or make real. Fake telemetry is worse than none.
- **Disable-with-reason for the inert set** — G2 `diffusion`, G3 `density_sync`,
  D1 `pad_N_articulation`, D5 "▶ Audition", W2 destinations, W1 sources: greyed with a
  tooltip until their Wave 3 todo lands, so nothing ships that silently does nothing.
  Delete their preset writes at the same time (`eighth_triplet_echo.json`).

### Wave 1 — Framework multipliers (`resonance-plugin`, `wayland-plugin-gui`, control wire)

Nine todos that fix classes rather than instances.

| # | Item | Kills |
|---|---|---|
| 1.1 | `float_knob(param)` reads the `FloatParam` it is handed; add `FloatRange::denormalize` and route knobs through it. Copy the pattern already in `granular-delay/src/editor/widgets.rs:112-140`. Migrate reverb/compressor/amp/IR/wavetable/gate. | **F4**, **W4**, **C5** (mind `key_hpf` min 0.0 vs the log path's 0.001 clamp) |
| 1.2 | Exact-value entry: double-click a knob → text field through the already-unit-tested `Param::parse` / `text_to_value`. | **F5** |
| 1.3 | Delete `FloatParam::smoother` / `with_smoother` — it can never advance behind an `Arc`; 14 call sites read like configured smoothing and are no-ops. | **X9** |
| 1.4 | `Param::module()` + populate the CLAP module path; group mastering's ~60 params across its 8 stages, wavetable's 87. | **X7** |
| 1.5 | `IntParam::with_value_to_string`; move the editors' label tables into the params; **extend `PluginParamView`** with `text`/`unit`/`module`/`stepped`/`choices`/`hidden` (vertical slice: wire + handler + MCP tool). | **X8**, **X9**, **P8**, **A6**, and the MCP half of **X7** |
| 1.6 | Output parameter events + gesture begin/end in both `flush` impls and `process()`; app re-queries on `ParamValueChanged` instead of trusting the one `query_params()` at instance creation. | **F1** — today the mixer panel shows the value the plugin had when it loaded, forever |
| 1.7 | MIDI CC / aftertouch / pitch-bend through the bridge (`clap_bridge/process.rs:102-127` ends in `_ => {}`). | **F3**, unblocks **W1** |
| 1.8 | Host handle out of `#[allow(dead_code)]` + `request_latency_change`; carry `latency_samples` on the add-echo event → chain-row badge (GUI) + plugin entry field (MCP). | **F2**, **X5**, unblocks **M6**, **I1** |
| 1.9 | `FEATURES` as `&[&CStr]` so an unmapped feature is a compile error; fix all 11 crates' categories and IR's `cabinet_simulator` typo. | **X6** |

### Wave 2 — Host surfaces (backend complete, one surface missing)

The cheapest class in the whole register. Several are literally "emit a message that
already exists".

- **2.1 P2 — chain reorder.** ▲▼ on the inspector chain row emitting
  `PluginMessage::MovePluginInTrack` (and the bus/master equivalents), which the
  control handler at `update/control/track/chain.rs:121` already dispatches, undo
  classification and instrument-floor rule included. GUI-only; MCP already green.
- **2.2 P1 — save track as preset.** `SaveTrackAsPreset` message setting the declared,
  never-written `pending_preset_save` (`lib.rs:334`), plus a track-header/inspector
  menu item. MCP slice: `track.save_preset` / `track.presets` / `track.apply_preset`,
  so the preset menu stops listing presets only hand-written JSON can create.
- **2.3 X3 + bypass parity.** Per-plugin bypass on all three chains (wires up the dead
  "BYP" label), `track.set_fx_bypass` to close the inverse gap, and a short crossfade
  in `run_fx_chain` so bypassing a reverb mid-playback stops truncating the tail.
- **2.4 X4 — generic param panel for GUI plugins too.** Drop the `has_gui == false`
  gate at `view/mixer/mod.rs:237-249`, and stop setting `editor_open = true`
  optimistically so a failed window doesn't leave a "Close Editor" button over nothing.
- **2.5 X10 — `plugins.rescan`.** Control method + MCP tool + a menu item; installing a
  plugin currently needs an app restart.
- **2.6 P5 — missing plugins.** *Split in two, the first is urgent:*
  **(a)** the data-loss path — save-as and template capture write only cached blobs, so
  a missing plugin's opaque state is silently destroyed on the first Save As;
  **(b)** the UX — a `missing` status on `PluginSlotState`, a badge, Relocate/Replace,
  and `status` + `track.replace_plugin` on the wire.
- **2.7 P3 — aux-send UI.** Replace the two hardcoded read-only rows
  (`inspector/routing.rs:73-74`) with real send/return controls emitting the eight
  `MixerMessage` variants that already document themselves as raised from there.
  MCP already green. Check epic #31 / doc #172 for the existing design before drawing.
- **2.8 P4 — sidechain.** Persistence (it is currently not saved at all), bus/master as
  key targets (`SetSidechainParams` is keyed by `track_id` only), plus the key-source
  picker and key chip. **Read `ba/epic-22` first** — a fuller unmerged implementation
  exists there (`project_sidechain_epic22_duplicate`); reconciling beats rewriting.
  Closes **C1** for compressor and gate at the same time.

### Wave 3 — Per-plugin capability exposure

Grouped by crate so each component dev owns one lane and they run in parallel. Almost
every item is "add a parameter", which per rule 1 lands GUI + MCP for free.

- **amp** — A3 drop the `ARCHITECTURE_A2` pin (both endpoints) and add an arch filter
  chip, so the A1 catalogue the engine has run reference-exact since epic #197 is
  visible; A4 request pagination the client already decodes; A6 unhide `file_select`
  (IR's identical param is not hidden); A1 accept `HeadParams::Windowed` at
  `parse/config.rs:185-189`; A2 slimmable size as a param instead of `FULL_SIZE`.
- **mastering** — M1 per-band attack/release/knee/mix (the defining multiband move is
  unreachable today, automation included); M2 band gain independent of compressor
  enable; M3 a bypass control for the param that already has a latency-matched dry
  path; M4 a `target_lufs` control; M5 per-band GR metering; M6 lookahead as a param
  (needs 1.8 for the latency change).
- **granular-delay** — G1 `align` bool + chip, exposing the finished, metered WSOLA
  alignment that only a test reaches; G2 write the `diffusion` stage; G3 tempo-locked
  grain rate for `density_sync`.
- **wavetable** — W1 mod wheel / aftertouch (needs 1.7); W2 osc balance (easy) and
  unison detune (needs per-voice re-detune, currently baked at note-on); W3 real tempo
  sync (`_tempo` is ignored today).
- **drums** — D1 make `bridge.articulations` and the 30 params one source of truth so
  automation and MCP work like the chip does; D2 polyphony param + velocity curve +
  round-robin randomization; D4 fold or delete the two "Coming soon" tabs that hide
  features already shipping in the Pads inspector; D5 an editor→audio-thread audition
  trigger; D6 unpack the round-robin index/depth both consumers reduce to a bool.
- **compressor / gate** — C2 publish the key-connected flag (the IN meter silently
  changes meaning today); C3 give the gate a viz object and surface its already
  computed `key_connected`; C4 gate presets (it ships none against EQ's and
  compressor's 11, and already depends on an otherwise-unused `serde_json`).
- **delay** — apply `stereo_offset` on all three routing modes, or disable it with a
  hint on the two where it does nothing.
- **ir** — I1 convolution block size / low-latency tracking mode as a param, with the
  imposed ~2.7 ms shown (needs 1.8).

### Wave 4 — Preset system, consolidation, tests

- **4.1 X1 — user preset save, once, in `resonance-plugin`** (0 of 11 plugins can save
  a preset today): user-preset directory, save/rename/delete, plus the
  `plugin.presets` / `.load_preset` / `.save_preset` vertical slice so an agent can
  capture and recall a sound too.
- **4.2 X2 (persistence half)** — preset identity in `save_state`, and a **version
  field on plugin state**: without one, renaming a param's string id silently restores
  that parameter to its default across every saved project and factory preset.
- **4.3 LOW consolidation** — promote `chip`/`segmented`/`slider` into the shared kit
  (drums and wavetable carry near-duplicates, granular a third, EQ a fourth idiom);
  delete 11 byte-identical copies of `RuntimeEditorHandle`; feed compositor resizes
  back into `PluginEditor::size()` so restored editor sizes are right; finish the
  lavender palette migration; showcase the five never-demonstrated granular modes in
  the preset JSON.
- **4.4 Tests** — DSP goldens beyond the current 2 of 11; `save_state`→`load_state`
  round-trips for the 6 crates missing them (amp, delay, gate, granular, reverb and
  the 87-param wavetable); drive `process()` through the ABI at least once (it holds
  the crate's only `unsafe`); a concurrency test for the load-vs-editor-pushback CAS.

---

## 2. Explicitly not doing now

Recorded so it is not re-litigated: amp's A=440 tuner switch and gear filter; IR's
left-channel-only 4096-sample response plot and index-999 cap; mastering's fixed
crossover slope, dither curve, tape voicing, imager side-HPF Q and capture window;
mastering assistant genre/reference persistence; drums' hi-hat-hardcoded choke groups;
the EQ's display-only Q on cut bands and its one-of-two spectra; delay's mirrored echo
view. All are real, none blocks a shipped capability, and every one of them is
cheaper after Wave 1 than before it.

**W2 unison detune** and **W1** are kept in scope but are the two most likely to slip —
both need structural change (per-voice re-detune; a new plugin event type).

---

## 3. Shape, sequencing and risks

- **Roughly 5 epics / ~65 todos.** Wave 0 ≈ 9, Wave 1 ≈ 9, Wave 2 ≈ 10, Wave 3 ≈ 28,
  Wave 4 ≈ 9.
- **Waves 0 and 1 are on the critical path.** 1.1 alone closes three findings across
  six crates; 1.5 closes four and is the single biggest MCP improvement in the plan.
  Wave 3 lanes are parallel *after* 1.1, 1.5, 1.7 and 1.8 land.
- **Design gate.** P3 (aux sends), P4 (sidechain), P5b (missing-plugin UX) and 4.1
  (preset browser/save) are new user-facing surfaces and should go through ba-designer;
  everything else is inline widget work against the existing design system.
- **Known collisions.** Epic #31 (aux sends) and epic #22 (sidechain) already exist,
  and epic #22 has an unmerged fuller implementation on its branch — check both before
  a line is written. Epic #65 ("Consistent bundled-plugin editors with true UI/DSP
  parity", deferred) is this plan's Waves 0/4 under an older name; reuse it rather than
  filing a new one.
- **Verify gate** is component-scoped now, so plugin-crate todos run their own suite;
  only the batch merge runs the full workspace. Plugin editors have no snapshot
  coverage — egui editor changes need a human look, unlike app-side iced goldens.
- **Do not run crate-wide `cargo fmt`** (local rustfmt reformats ~130 files); format
  only changed files.

---

## 4. Suggested first cut

If only one wave ships: **Wave 0 + items 1.1, 1.5, 2.1, 2.2, 2.6a.** That is a
week-ish of work, removes every control that lies, makes editor/param drift impossible
rather than fixing four instances of it, gives MCP clients real units and choice
labels for the first time, gives humans chain reorder and preset capture, and closes
the one finding in the register that destroys user data.
