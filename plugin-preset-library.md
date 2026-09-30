# Plugin preset library — one browser, tags, favourites and genre for every plugin

Status: **design, not started** (2026-09-30). Nothing in here is built. Build it
as vertical slices (§12): each slice lands its data change, both surfaces (the
plugin's own editor and the host), the control method + MCP tool, and tests
together. That is the rule `plugin-audit-plan.md` §0 sets and the rule
`project_control_api_vertical_slices` records.

Sibling spec: `nam-model-library.md` (manager UI for NAM amp models: delete,
favourite, search, import, re-download). The two were written at the same
time. This one was reconciled against that spec's §4.3, §8 and §10. The marks
store, key scheme, kind name and content-hash model ids here are that spec's
choices, adopted as-is. §10.2 lists the few changes this spec asks the NAM
spec to make.

## 0. Why

Every first-party plugin can load and save presets now. Wave 4.1 of the plugin
audit shipped `PresetBank`, `PresetSession` and the shared `preset_bar` (§1).
What is still missing is everything that makes a preset library usable after
the first twenty entries:

- A preset is identified only by its **display name**. Rename one and every
  reference to it breaks: the loaded-preset identity saved in a project, and
  any favourite you might want to keep on it.
- There is no metadata at all. There are no tags, genre, author, description,
  category or favourites. Factory banks fake a category in the name
  (`"Bass — Reese"`, `"Kick — Punch"`), and agents fake a collection in the
  name (23 user wavetable presets on this machine are called `Ferrous …`,
  `PM_…`, `IPM_…` for the songs they were made for).
- There is no browser. The bar is a combo box with ◀/▶. It cannot search,
  filter, sort or audition, and it has no form for anything beyond a name.
- The host has no preset UI at all. Presets reach the host only through the
  control API (`*.plugin_presets`, `*.load_plugin_preset`,
  `*.save_plugin_preset`). The "dual-surface" rule is met for load/save by
  MCP plus the plugin editor. The iced app shows nothing.
- A preset does not capture the whole sound for the four plugins whose sound
  lives partly outside their parameters (amp's NAM model, IR's impulse,
  wavetable's user tables, drums' kit). Details are in §2.
- Third-party CLAP plugins can't save a preset through the host, and the
  failure is silent (§2, gap 5).

## 1. What already exists (verified against master @ 4f7a6baf)

### 1.1 Plugin-side (the SDK every first-party plugin uses)

| Piece | Where | Relevant facts |
|---|---|---|
| Preset store | `resonance-plugin/src/presets.rs` | Module doc `:1-42`. User presets are `$XDG_DATA_HOME/resonance/plugin-presets/<clap-id>/<file>.json` (`:22`), and `RESONANCE_PLUGIN_PRESET_DIR` overrides the root (`:57`, `:510-517`). |
| Identity | `presets.rs:99-159` | `PresetSource::{Factory, User}` + `PresetRef { name, source }`. **"The pair is the identity"** (`:128-130`). No id field. |
| Factory entry | `presets.rs:166-169` | `FactoryPreset { name: &'static str, json: &'static str }`. It has a name and a body and nothing else. |
| Bank | `presets.rs:199-507` | `list()` (`:252`) is the factory bank in declared order, then user presets sorted by lowercase name. `list_user()` (`:263-299`) runs `read_dir` and **parses every file** to recover its stored `"name"`. `save()` (`:332-359`) writes `params_to_json(params)` + `"name"`. That is **params only** (`:341`). `write_user_preset()` (`:371-396`) writes whatever document the host hands it. rename/delete are for user presets only (`:399-447`). Every write is a plain `std::fs::write` (`:357`, `:394`, `:431`), so writes are **not atomic**. The app's track presets use `atomic_write`. |
| Session | `presets.rs:569-693`, `:906-951` | Holds the loaded `PresetRef` + an `AtomicBool modified` set by `mark_modified()` (`:624`), which editors call from their param-write path. It is persisted into the plugin state under `"preset": {name, source, modified}` through `ExtraStateSaver`. A preset loaded again *by name* after the file was renamed is silently lost. |
| Bar behaviour | `presets.rs:699-904` | `PresetEditor` holds naming state. "Save" over a factory preset pre-fills `"<name> (edit)"` (`:758-770`). `step()` clamps and does not wrap (`:854-884`). The model is GUI-agnostic and covered by `resonance-plugin/tests/presets.rs`. |
| Bar skin | `resonance-plugin/src/preset_ui.rs:33-122` | egui (`plugin_gui_core::egui`), behind `editor-widgets`. It is ◀ combo ▶, then Save/Rename/Delete, then a `•` modified dot. **It calls `bank.list()` every frame** (`:58`), so every repaint of every open editor does a `read_dir` plus a parse of every user preset. The combo calls `list_user()` again while open (`:152`). |
| Plugin trait | `resonance-plugin/src/plugin.rs:316`, `:338`, `:492` | `VERSION` (from `CARGO_PKG_VERSION`), `FACTORY_PRESETS` (default empty), and `extra_state_saver()`. |
| Host-readable factory bank | `resonance-plugin/src/lib.rs:85-108`, `resonance-common/src/factory_presets.rs` | `export_clap!` exports `resonance_factory_presets()`, a JSON array of `{name, json}`. It is first-party only. The module doc says CLAP's `preset-discovery` "is not implemented here". |
| State format | `resonance-plugin/src/state.rs:4-57` | `{ "version": 1, "params": {id: value} }` plus the extra-state keys merged at top level (`clap_bridge/state.rs:49-80`). `ParamRename` migration exists. |
| Adoption guard | `resonance-plugin/tests/fleet_preset_adoption.rs` | A source-text check that all 13 editors call `preset_bar`. Every plugin in `plugins/` does (amp, color, compressor, delay, drums, eq, gate, granular-delay, ir, mastering, reverb, stereo, wavetable). |

**Factory banks today.** color 5, compressor 11, delay 6 (inline JSON strings
in `presets.rs`, the only crate without `presets/*.json` files), eq 11, gate 9,
granular-delay 8, reverb 10, stereo 7, wavetable 28. That is **95 presets in 9
plugins**. amp, drums, ir and mastering ship none. The only naming convention
is `"<Category> — <Name>"`, and only some banks follow it.

**User presets on this machine.** `~/.local/share/resonance/plugin-presets/`
holds amp 3, compressor 1, mastering 3, wavetable 23. The amp ones show the
extra-state gap directly: `"5051- blue"` is `{"params":{"file_select":0.0,
"input_gain":1.0,"output_gain":0.5}}`. It stores a directory index, not a
model.

### 1.2 Host-side (app, engine, control API, MCP)

| Piece | Where | Relevant facts |
|---|---|---|
| Scan | `resonance-audio/src/clap_host/bundle.rs:196`, `:584-600`; `engine/scan.rs:237`; `types/tempo/mod.rs:106-118` | The scan reads the first-party symbol into `ScannedPlugin.factory_presets: Vec<(String, String)>`. `ScannedPlugin` carries **no plugin version**. |
| Control handler | `resonance-app/src/update/control/plugin_presets.rs` | `bank_for` (`:54-65`) builds a `PresetBank` with **no factory half**, and keeps factory presets beside it from the scan (`:68-74`). `view()` (`:77-106`) always reports `current: None, modified: false` (`:98-104`, "ba todo #1294 is the same gap"). `json_for` (`:114-151`) matches by name with `eq_ignore_ascii_case`, preferring user presets. `load_message` (`:158-208`) applies **params only**, one by one, through `stable_hash(string_id)`. It is one undo entry (`message.rs:204`, `:277`; `update/plugin.rs:147-170`). `write_saved_state` (`:246-259`) requires the blob to be **UTF-8 JSON** (`:252-254`). |
| Save path | `resonance-app/src/update/control/track/presets.rs:175-206`, `chain_presets.rs:193`, `engine_events/plugins.rs:665-693` | The save arms `pending_plugin_preset_save`, sends `SavePluginState`, and writes the blob when `state_saved` echoes it back. The blob is the **full** state, including extra keys and the plugin's own `"preset"` identity key. So a host-saved file carries data that a host load then ignores. |
| No mirror of plugin-side edits | `plugin_presets.rs:22-30`; `resonance-audio/src/types/events.rs:527-531` | There is no plugin→app param-change event. Edits made in a plugin's window do not reach the app's mirror (ba todo #1294, still open). |
| Wire | `resonance-control/src/methods/plugin_preset.rs:31-69`; `track.rs:76-87`, `:821-875`; `bus.rs:50-60`, `:316-365`; `master.rs:46-56`, `:313-355` | `PluginPresetEntry { name, source }` and `PluginPresetsView { plugin_id, presets, current, modified }`. Load takes `preset` (a name) + optional `source`. Save takes `name` + `overwrite`. `PROTOCOL_VERSION = 1`, and additive changes don't bump it (`resonance-control/src/lib.rs:22-24`, `:61`). |
| MCP | `resonance-mcp/src/tools/trackmix.rs:652`, `:673`, `:697`; `bus.rs:324`, `:344`, `:369`; `master.rs:319`, `:339`, `:364` | Nine tools, three per surface. |
| Skills | `resonance-agent-plugin/skills/mixing/references/character.md:106-116`, `spatial/references/depth.md:28-34` | Skills name factory presets (`Bus — Warm Glue`, `Tight Room`, …). `resonance-mcp/tests/agent_plugin_lockstep.rs:470-480` collects **every `name: "…"` literal in `plugins/*/src/presets.rs`** and fails if a skill names one that doesn't exist. |
| App UI | `resonance-app/src/view/mixer/plugin_panel.rs:13-110` | The bottom plugin panel header is name, Open/Close Editor, ×. It has **no preset control**. The media browser (`view/browser/mod.rs`) has Files and Pool tabs and a WARM-star favourite for folders, persisted in `settings.json` (`settings.rs:57-63`). |

### 1.3 Track presets (a different thing with a similar name)

`track_presets` / `track_apply_preset` / `track_save_preset`
(`trackmix.rs:401-446`, `track.rs:92-100`) are **track templates**:
`TrackPreset { name, track_type, volume, pan, mono, instrument_type,
instrument_icon, role, plugins: Vec<PresetPlugin> }`
(`resonance-app/src/presets.rs:21-47`). Each `PresetPlugin` embeds an **opaque
state blob** (`:40-47`), so they work for third-party plugins too. They are
stored in `~/.local/share/resonance/track-presets/` (`:175-183`) with
`atomic_write` (`:304`), and listed in the add-track menu
(`view/menus.rs:29-57`, `:153-172`). They don't reference plugin presets; they
embed whole chains. This spec leaves their format alone and adds them as a
later asset kind for favourites/tags only (§10.3, slice P9).

### 1.4 Content the NAM spec will touch

- `resonance-common/src/registry.rs:34-50`: `installed.json` with
  `ContentType::{Drumkit, AmpModel}` and `InstalledItem { name, type, path,
  installed_at }`. Saves are load-modify-write with **no lock**.
- `plugins/resonance-amp/src/models.rs:9`: downloads go to
  `resonance/amp-models/tone3000/`. `lib.rs:198-205` seeds the ◀/▶ list from
  that directory. `params.rs:10-19`: `model_path` (extra state,
  `lib.rs:338-356`) and `file_select`, an `IntParam` **index into a directory
  listing**.
- The amp editor already has an in-editor overlay browser, the Tone3000 panel
  (`editor/tone3000_panel.rs`, an `egui::Area`). It is the nearest existing
  precedent for the browser overlay proposed here.

### 1.5 Shared widget layer (what the browser can be built from)

- Plugin editors are **egui 0.34** through `plugin-gui-core`. The palette is
  `plugin_gui_core::theme::lavender` (`BG_0..3`, `LINE`, `TEXT_1..4`, `ACCENT`,
  `WARM`, `GOOD`, `BAD`), which matches the app's `theme.rs`. The shared kit
  has `widgets::{chip, segmented, slider}` and knobs. `resonance-plugin`'s
  `editor-widgets` feature binds them to params.
- The app is **iced**. It has the same tokens, and the media browser's WARM
  star and pill chips are the idiom to reuse.
- One widget cannot be drawn by both toolkits. The existing pattern is a
  GUI-agnostic model (`PresetEditor`) with a thin egui skin (`preset_ui.rs`),
  and this spec extends it. The browser **model** is shared, and there are
  two **skins**: egui in plugins, iced in the host.
- Layering (`tools/arch-invariants/tests/architecture.rs:234-239`,
  `:455-468`): `resonance-plugin` may depend on `resonance-common`. Plugins
  may name only the allow-listed `resonance_common` items, and only amp,
  drums and ir depend on it directly. `resonance-app` depends on
  `resonance-plugin` already (`plugin_presets.rs:46`).

## 2. Gaps found while reading. They change the scope.

1. **Identity is the name.** Rename, favourites, project recall and the
   control API all key on `PresetRef { name, source }` or on a
   case-insensitive name. The library needs a stable id first; everything
   else depends on it.
2. **Presets drop extra state.** Editor save writes params only
   (`presets.rs:341`), and both loaders apply params only (`presets.rs:87-93`,
   `plugin_presets.rs:158-208`). The host save writes the full blob, so the
   file holds a model path that no load reads. For amp, ir, wavetable and
   drums a "preset" is therefore the knobs around a sound, not the sound.
   Amp's `file_select` index is worse than nothing: the same preset loads a
   different model after a download changes the directory order.
3. **Per-frame disk scan.** `preset_bar` lists the bank every frame
   (`preset_ui.rs:58`). That is tolerable at 30 files. With metadata parsing
   and hundreds of presets it is not. The library needs an in-memory index
   with change detection.
4. **Non-atomic writes, no cross-instance coordination.** Two instances of a
   plugin, or the app and a plugin, can both write in the same second. A torn
   write gets quarantined by nobody: `list_user` just skips unparsable files.
5. **Third-party save fails.** `write_saved_state` rejects non-UTF-8 blobs
   (`plugin_presets.rs:252-254`), and a third-party state blob is almost
   always binary. The call acks (`track/presets.rs:204`), and the failure
   only shows up as a banner later. The load path also requires a JSON
   `params` object.
6. **The host can't see which preset is loaded.** `current: None` always
   (`plugin_presets.rs:98-104`). The plugin knows its identity, but there is
   no channel to report it. The standard channel is CLAP's `preset-load`
   extension (`clap_host_preset_load::loaded`), which is available in both
   `clap-sys 0.5` (`src/ext/preset_load.rs`) and the pinned clack checkout
   (`extensions/src/preset_discovery`).
7. **The lockstep test parses `presets.rs` source.** Moving factory names
   out of `name: "…"` literals would silently empty the name set the skills
   are checked against. Either the names stay literals or the test's scanner
   changes in the same slice (D7).

## 3. Goals and non-goals

**Goals**

- G1. One preset model and one store in the SDK, used by all 13 first-party
  plugins without per-plugin code beyond declaring factory metadata.
- G2. Stable preset ids, so renaming never breaks a favourite, a tag, a
  project's loaded-preset identity or an agent's reference.
- G3. Metadata: name, author, description, category, tags, genres,
  character, instrument/source type, plugin version at save,
  created/modified, factory vs user. Plus user marks (favourite,
  personal tags, last used) that work on **factory presets too**.
- G4. One browser **model** (search, facets, sort, stepping, audition,
  CRUD, import/export) with an egui skin inside every plugin editor and an
  iced skin in the host.
- G5. A preset is the whole sound. Params **and** the plugin's sound-bearing
  extra state.
- G6. Dual surface: everything a human can do in the browser, an agent can do
  over the control API. Listing and filtering by tag/genre/favourite and
  saving with metadata are the ones agents need most.
- G7. Useful support for third-party CLAP plugins: user presets from state
  blobs, with metadata and favourites, plus their own factory banks where
  they publish them through `preset-discovery`.
- G8. Correct with several instances and two processes touching the same
  library.

**Non-goals**

- Cloud sync, sharing or a preset marketplace. Export/import files are the
  sharing story.
- Merging track presets into plugin presets. They stay a separate kind (§1.3).
- A preset *morph* or A/B slot system. Audition-and-revert is in scope;
  morphing is not.
- Editing a third-party plugin's own factory bank, or integrating with its
  in-plugin browser.
- Backward compatibility with the current user-preset files beyond a one-shot
  converter (`project_no_real_users_yet`).
- Rating stars in v1 (D4).

## 4. Data model

### 4.1 Two layers: content and marks

A preset has two kinds of data. They have different owners and lifetimes:

| Layer | Holds | Owner | Lives in | Travels with an exported preset? |
|---|---|---|---|---|
| **Content** | state + descriptive metadata (name, author, description, category, tags, genres, character, instrument, plugin version, created/modified) | whoever made the preset | the preset file. For factory presets, compiled into the plugin | yes |
| **Marks** | favourite, personal tags, last used, use count | the local user | one per-user index, `library/marks.json` | no |

This split lets a user favourite and tag a **read-only factory preset**:
the mark is keyed by the preset's id and never touches the plugin
binary. Descriptive metadata is embedded so that an exported preset keeps its
tags and genre. A "Deep House / warm / pad" tag on a shared preset is part of
what the author is sharing. "I starred this" is not.

The view a browser shows is the merge. `tags = content.tags ∪
marks.tags`, where only the marks half can be edited on factory
presets. Editing the tags of a **user** preset edits the file, since the user
owns it (D3).

### 4.2 Stable ids and keys

```text
mark key  = "<kind>:<id>"                     (nam-model-library.md §4.3, adopted as-is)
  kind    = plugin-preset | amp-model | (later) ir | drumkit | wavetable | track-preset
  id      = opaque string, stable for the asset's life
            plugin-preset → "<clap id>:<preset id>"   e.g. "plugin-preset:com.resonance.reverb:tight-room"
            amp-model     → sha256 of the file bytes   (NAM spec §4.2, D1)
```

The store only ever sees the key as an opaque string. Each kind decides what
goes after the first `:`.

- **User presets**: `id` is a UUIDv4 minted at save, stored in the file, and
  never changed by rename, re-save or metadata edits. **Duplicate** mints a
  new one. **Import** keeps the file's id unless it collides with a
  different preset already in the library. On collision it mints a fresh id
  and reports it.
- **Factory presets**: `id` is an explicit slug declared next to the entry
  (`"bass-reese"`), unique per plugin and enforced by a fleet test. It is
  explicit, not derived from the name, so a factory rename keeps favourites.
  A factory id is never reused for a different sound (a fleet test pins the
  set, like the pre-W6b colour blobs, `44623b3a`).
- **Third-party discovered presets** (§8): `id =
  hex(sha256(location_kind ‖ location ‖ load_key))[..16]`. This is stable
  for as long as the plugin keeps the location.
- **Files with no id** (hand-dropped `.json`, legacy files): the first index
  scan writes a fresh id into the file (atomic rewrite). The user preset
  directory is ours, so rewriting in place is fine.
- The file name is **not** identity. New files are
  `<sanitised name>-<first 8 of id>.json`: readable, collision-free, and
  never recomputed from the name for lookup. Rename rewrites the name inside
  and moves the file to the new stem. Lookup always goes through the index
  (id → path).

`PresetRef` becomes `{ id, source, name }`, where `name` is only a display
hint for error messages. Equality compares `(source, id)`.

### 4.3 The preset file (format 1)

```json
{
  "format": "resonance.preset",
  "format_version": 1,
  "id": "3f0c9a4e-7a51-4d7e-9b1e-5b2a8f1c0d42",
  "plugin": {
    "id": "com.resonance.wavetable",
    "name": "Resonance Wavetable",
    "version": "0.4.0"
  },
  "meta": {
    "name": "Reese — Smooth",
    "author": "Jorrit",
    "description": "Detuned saw pair through the ladder, slow drift.",
    "category": "Bass",
    "instrument": ["synth-bass"],
    "genres": ["drum-and-bass", "industrial"],
    "character": ["dark", "wide", "evolving"],
    "tags": ["ferrous", "reese"],
    "created": "2026-09-30T14:02:11Z",
    "modified": "2026-09-30T14:10:40Z"
  },
  "state": {
    "encoding": "resonance-json",
    "doc": { "version": 1, "params": { "osc1_level": 0.8 }, "user_wavetables": { } }
  }
}
```

- `state.encoding = "resonance-json"`: `doc` is exactly the plugin's state
  document, parsed, so the existing loaders and `ParamRename` migration apply
  unchanged. The plugin's session key (`"preset"`, `presets.rs:60`) is
  **stripped** before writing. A preset must not claim to be "modified
  version of itself".
- `state.encoding = "clap-state"`: `blob` is base64 of the plugin's opaque
  `clap.state` bytes, used for third-party plugins (§8). There is no `doc`.
- `plugin.version` is `ResonancePlugin::VERSION` for first-party plugins,
  and the descriptor's `version` for third-party ones. `state.doc.version` is
  the state schema version (`STATE_VERSION`). They mean different things and
  both are kept: the first is for display ("saved with 0.3.1") and for
  warnings, the second drives migration.
- The extension stays `.json`. A distinct `.rpreset` extension is reserved
  for **export bundles** (§6.6), so a file manager can tell "one preset" from
  "a pack".

### 4.4 Metadata vocabulary

Free text everywhere is how the `"Bass — Reese"` / `"Ferrous_…"` naming
arose. The fields are split into **controlled facets**, which browsers and
agents can filter on reliably, and **free tags**:

| Field | Cardinality | Vocabulary | Notes |
|---|---|---|---|
| `category` | 0..1 | per plugin *class*: instruments `Bass, Lead, Pad, Pluck, Keys, Arp, Brass, Strings, Drone, FX, Drums, Init`; effects `Utility, Track, Bus, Master, Creative` | Parsed from today's `"X — Y"` names by the converter |
| `instrument` | 0..n | `vocal, lead-vocal, backing-vocal, guitar, electric-guitar, acoustic-guitar, bass, synth-bass, drums, kick, snare, hats, room, keys, piano, synth, strings, violin, mix-bus, drum-bus, master, full-mix` | "What is it for". For effects it names the source it suits. Answers "compressor presets for vocals" |
| `genres` | 0..n | seeded list: `ambient, americana, cinematic, drum-and-bass, electronic, folk, hip-hop, house, indie, industrial, jazz, metal, pop, post-metal, rock, singer-songwriter, techno, …` | Superset of `resonance-mastering-assist::Genre` (`targets.rs:52-59`) and the genre skills in the agent plugin |
| `character` | 0..n | `warm, bright, dark, clean, gritty, saturated, punchy, soft, wide, narrow, lush, dry, subtle, aggressive, vintage, modern, evolving, static, metallic, airy` | Timbre words. Doubles as the vocabulary `warmth-width-depth.md` uses for the colour plugin |
| `tags` | 0..n | free, lowercase, `[a-z0-9-]`, ≤ 32 chars | Collections ("ferrous"), techniques ("sidechain") |

The vocabulary is a `const` table in `resonance_common::library_marks::vocab` (§4.5).
Values outside it are **accepted and kept** (lowercased and slugged) in
`genres`/`character`/`instrument`. They sort after the seeded values and
appear in the facet list only when some preset uses them. The MCP schema
lists the seeded values as `examples`, not as an `enum`, so an agent can
still write `"shoegaze"`.

### 4.5 The marks store

This is the NAM spec's schema (§4.3 there) with one addition, the
`generation` counter its §8 already asks for:

```json
{ "version": 1,
  "generation": 412,
  "items": {
    "plugin-preset:com.resonance.reverb:tight-room": {
      "favorite": true, "tags": ["vocal-chain"], "last_used": "2026-09-30T10:11:00Z", "use_count": 7
    },
    "amp-model:9f2c…": { "favorite": true, "tags": ["djent"] }
  } }
```

- It lives at `$XDG_DATA_HOME/resonance/library/marks.json`, overridable by
  `RESONANCE_LIBRARY_DIR`. The module is `resonance_common::library_marks`,
  which is the NAM spec's name, and it is in `PLUGIN_COMMON_ITEMS`. There is
  one file for every kind, which is the point of §10.
- An item with every field at its default is deleted rather than stored.
- `last_used` / `use_count` are written only for a **user pick**: a pick in
  a browser or bar, or a control-API load. They are not written for a
  project-open restore (NAM spec D10, adopted). Opening an old project
  therefore doesn't reshuffle "Recently used".
- **Orphans** (items whose asset is gone) are kept for 90 days, then pruned
  at store open. The clock is injected for tests, as in the NAM spec's
  `library_marks` test. A deleted-then-reimported preset keeps its star
  within that window, and so does a factory preset missing from one build.
- Tag completion reads the tags of every kind (NAM spec §10 "Tag
  vocabulary"). The seeded facet vocabulary of §4.4 lives in the same module
  as `library_marks::vocab`.

### 4.6 The in-memory index

`resonance_plugin::presets::PresetLibrary` holds, per process (the content
store stays in the SDK, where `PresetBank` is today; only marks and vocab
are in `resonance-common`, see D1):

- the content records of each plugin it has been asked about (id → path, meta,
  source, mtime). Factory records are registered by the caller (plugin:
  `FACTORY_PRESETS`; host: the scan).
- the marks map,
- a query engine: `Query { text, plugins, sources, favorites_only,
  category, instrument[], genres[], character[], tags[], sort }` →
  `Vec<Hit>`. Text search is case- and accent-insensitive and
  token-prefix-based over name, author, description, tags and category, with
  name hits ranked first. Facet counts come back with the hits.

The index is refreshed on demand, not per frame. `refresh_if_stale()`
compares a cheap fingerprint (the plugin directory's mtime + entry count,
plus the marks file's `generation`). It checks every 500 ms while a browser
is open, and at most every 2 s when only the compact bar is visible, which
is the NAM spec's §8 header cadence. With nothing visible it refreshes only
on explicit calls (save, a control-API list). This removes gap 3. The
freshness check is one shared helper in `library_marks`, used by both
libraries.

## 5. Factory vs user presets

| | Factory | User |
|---|---|---|
| Stored | compiled in (`include_str!`), exported to the host by symbol | `plugin-presets/<clap-id>/*.json` |
| Load / step / audition | yes | yes |
| Favourite, personal tags | yes (marks layer) | yes (marks layer) |
| Edit name / meta / descriptive tags | **no** — "Save as…" makes a user copy | yes (rewrites the file) |
| Rename / delete | no | yes (delete asks; §6.5) |
| Duplicate | yes → user preset with a new id, `meta.derived_from = "<factory id>"` | yes |
| Export | yes | yes |
| Shadowing | a user preset may carry the same **name**; ids never collide, so both are listed and both are addressable | |

`derived_from` is kept on "Save as…" from any loaded preset. The browser
uses it to show "based on Tight Room". The converter can't recover it for
legacy files.

**Factory metadata lives in the factory JSON files.** `presets/*.json` gains
the `id` and `meta` blocks of §4.3 (a factory file *is* a preset file, with
`plugin.version` filled in at encode time from `VERSION`). The Rust table
becomes `FactoryPreset { id, name, json }`. `name` stays a literal so the
lockstep scanner keeps working, and a fleet test asserts it equals
`meta.name` (D7). resonance-delay's six inline strings move to
`presets/*.json` in the same slice.

## 6. The preset browser

### 6.1 One model, two skins

```text
resonance-common::library_marks     marks store, key scheme, vocab, tag completion,
                                    freshness poll, reveal launcher (shared with NAM)   (no GUI)
resonance-common::nam_library       NAM header reader + index (NAM spec, not this one)   (no GUI)
resonance-plugin::presets           PresetLibrary / PresetBank / PresetSession: preset
                                    content, ids, query, legacy converter                (no GUI)
resonance-plugin::library_view      BrowserModel: query state, selection, stepping, audition
                                    bracket, naming/meta form, confirm-in-place, errors,
                                    over a LibraryRows trait                             (no GUI, not feature-gated)
resonance-plugin::preset_ui         egui skins (editor-widgets): list widget, preset_bar,
                                    preset browser overlay, metadata form
resonance-app::view::presets        iced skins over the same BrowserModel: plugin_panel bar,
                                    media-browser Presets tab
```

This split is the NAM spec's §10 table plus this spec's host needs. The NAM
spec puts the browser list widget and its pure view-state "in
`resonance-plugin` behind `editor-widgets`, beside `preset_ui`". This spec
keeps the widget there and moves the **view-state** into an ungated
`library_view` module, because the iced app has to drive it too. The app
already links `resonance-plugin` without `editor-widgets`
(`plugin_presets.rs:46`). That is the one change §10.2 asks of the NAM spec.

`BrowserModel` generalises the current `PresetEditor` (`presets.rs:699-904`)
and replaces it. Every behaviour below is a method on it that returns an
event (`Loaded`, `Auditioned`, `AuditionReverted`, `Saved`, `MetaChanged`,
`Renamed`, `Deleted`, `Imported`, `Exported`). It is unit-tested without a
window, as `PresetEditor` is today. It is generic over `LibraryRows`, the
NAM spec's `{title, subtitle, columns, key, marks}` trait, extended with
`apply` / `capture` for audition. That is how one list widget serves both
the preset browser and the NAM manager's Installed tab.

### 6.2 Compact bar (always visible, in every editor header)

```text
┌──────────────────────────────────────────────────────────────────────────┐
│ ◀  ☆  Reese — Smooth •        ▶   [ Browse ⌄ ]   [ Save ]  [ Save as… ]  │
└──────────────────────────────────────────────────────────────────────────┘
  ◀/▶   step within the browser's CURRENT filtered, sorted list (not the whole bank)
  ☆/★   toggle favourite on the loaded preset (WARM when set)
  •     modified since load (hover: "3 parameters changed: cutoff, reso, drive")
  Save  overwrite the loaded USER preset in place (disabled on factory → use Save as…)
```

Rename and Delete move off the bar into the browser. They are rare and
destructive, and they crowd the header. This is also why several editors
wrap the current bar today.

### 6.3 Browser overlay (inside a plugin editor)

It opens from **Browse**. It is an `egui::Area` over the editor body, the
same mechanism as the amp's Tone3000 panel. The plugin's controls stay live
underneath, so audition is audible. Wide layout (editor ≥ 720 px):

```text
┌─ Presets · Resonance Wavetable ────────────────────────────── [×] ───────┐
│ 🔍 [reese____________________]  Sort: [Name ▾]   ☐ Favourites only       │
├───────────────┬───────────────────────────────────────┬──────────────────┤
│ SOURCE        │ ★ Name                  Category  By  │ Reese — Smooth   │
│ ● All     128 │ ─────────────────────────────────────│ by Jorrit · user │
│ ○ Factory  28 │ ☆ Bass — Reese          Bass     F   │ saved with 0.4.0 │
│ ○ User    100 │▶★ Reese — Smooth •      Bass     J   │ based on         │
│               │ ☆ Ferrous Reese         Bass     J   │   Bass — Reese   │
│ CATEGORY      │ ☆ Reese Wide            Bass     J   │                  │
│ ☐ Bass     14 │                                       │ Deep, detuned…  │
│ ☐ Pad      22 │                                       │                  │
│ ☐ Lead     11 │                                       │ genres           │
│               │                                       │ (dnb)(industrial)│
│ GENRE         │                                       │ character        │
│ ☐ industr. 23 │                                       │ (dark)(wide)     │
│ ☐ dnb       4 │                                       │ tags             │
│               │                                       │ (ferrous)(+ tag) │
│ CHARACTER     │                                       │                  │
│ ☐ dark     19 │                                       │ [Edit info…]     │
│ ☐ wide     12 │                                       │ [Duplicate]      │
│               │                                       │ [Rename]         │
│ TAGS          │                                       │ [Export…]        │
│ (ferrous 23)  │                                       │ [Delete]         │
│ (guide 7) …   │                                       │                  │
├───────────────┴───────────────────────────────────────┴──────────────────┤
│ ↑↓ audition · Enter keep · Esc revert to "Init"      [Import…] [Save as…]│
└──────────────────────────────────────────────────────────────────────────┘
```

Narrow layout (editor < 720 px, e.g. gate or stereo): the facet column
collapses into a chip row under the search field, and the detail pane
becomes a disclosure under the selected row:

```text
┌─ Presets ─────────────────────────────── [×] ┐
│ 🔍 [____________]  [Name ▾]  ★               │
│ (Factory)(User) (Bass)(Pad)… (+ filters 2)   │
├──────────────────────────────────────────────┤
│ ☆ Kick — Punch                  Track   F    │
│▶★ Vocal — Lead •                Track   F    │
│   ├ vocal · pop, indie · clean, punchy       │
│   └ [Save as…] [Duplicate] [Export…]         │
│ ☆ Bus — Auto Glue               Bus     F    │
├──────────────────────────────────────────────┤
│ ↑↓ audition · Enter keep · Esc revert        │
└──────────────────────────────────────────────┘
```

Built from existing parts: `widgets::chip` (facet chips, tag pills; COMPACT
style for the narrow row), `egui::ScrollArea` with `show_rows` (virtualised
list), `TextEdit::singleline` (search), WARM star glyph, lavender tokens.
The new shared widgets are `tag_pill` (chip + × to remove) and `star_toggle`.
They go in `plugin_gui_core::widgets`, where the NAM manager can use them.

### 6.4 Behaviours

| Behaviour | Rule |
|---|---|
| **Search** | Live as you type, debounced 120 ms. Tokens are ANDed, and a token like `genre:metal` / `tag:ferrous` / `is:fav` / `by:jorrit` scopes itself, so the same syntax works in the MCP `query` string. |
| **Facets** | Within a facet: OR. Across facets: AND. Counts are computed on the result set with the other facets applied (standard faceted search). A zero-count value stays visible while selected. |
| **Sort** | Name, Category, Recently used, Recently modified, Favourites first (a toggle that applies on top of any sort). Factory-declared order is available as **Bank order**, the default when no filter is active, so today's list order is unchanged by default. |
| **Prev/next stepping** | ◀/▶ and ↑/↓ walk the **current result list**, clamped rather than wrapping (current `step()` rule, `presets.rs:849-853`). With the browser closed they walk the last-used query, remembered per plugin instance in the session. |
| **Audition** | Moving the selection loads the preset *provisionally*. Before the first provisional load, the model captures a **revert snapshot** of the current full state (params + sound-bearing extra). Enter or double-click **commits**. Esc or closing with × **reverts** to the snapshot. Clicking outside the overlay commits, which matches "I played with it and kept it". Host side: the whole audition bracket is **one** undo entry at commit, and no entry on revert (§6.7). For instruments, auditioning a preset doesn't sound anything by itself. D5 covers a preview note. |
| **Save** | Overwrites the loaded user preset in place, same id, `modified` timestamp bumped. |
| **Save as…** | Opens the metadata form (§6.5) pre-filled from the loaded preset's meta (category, instrument, genres, character, tags; author defaults to the last author used). The name is pre-filled as today (`presets.rs:758-763`). The new preset gets a new id and `derived_from`. |
| **Edit info…** | The same form on an existing user preset. For factory presets it opens in "marks only" mode (favourite + personal tags). |
| **Rename** | Inline on the row (F2). The id doesn't change, so favourites, tags and loaded-identity follow. |
| **Duplicate** | "<name> copy", with a new id and `derived_from`. |
| **Delete** | User presets only. It uses the shared confirm-in-place row (NAM spec §7.1): the first click turns the row into "Delete 'Reese Wide'? [Delete] [Cancel]" and the second click deletes. The file moves to `plugin-presets/.trash/` for 30 days rather than being unlinked, so an accidental delete through either surface is recoverable. The mark survives (§4.5). A loaded preset that is deleted keeps sounding; identity is cleared (current `presets.rs:681-692` rule). |
| **Import** | File dialog (rfd, as the amp and IR already use) for `.json` / `.rpreset`. A preset for a different plugin id is refused with a message naming the plugin. The import is validated (parses, and its param ids overlap the plugin's) before it is copied in. |
| **Reveal** | Opens the preset's file in the system file manager, through the shared `library_marks` reveal launcher (NAM spec §10). Factory presets have no file, so the action is disabled for them. |
| **Export** | Single preset: a `.json` copy. Selection or current filter: a `.rpreset` bundle (zip: `manifest.json` + one preset file each). Referenced external files (a NAM model, an IR wav) are **not** bundled in v1; the manifest lists them by path and content hash (D6). |

### 6.5 Metadata form

```text
┌─ Save preset ───────────────────────────────────────────────┐
│ Name        [Reese — Smooth______________]                   │
│ Author      [Jorrit______]                                   │
│ Category    [Bass ▾]                                         │
│ For         (synth-bass ×) (+)                               │
│ Genres      (industrial ×) (dnb ×) (+)      suggestions: …   │
│ Character   (dark ×) (wide ×) (+)                            │
│ Tags        (ferrous ×) (+)                                  │
│ Description [__________________________________________]     │
│                                                              │
│ ⚠ A user preset named "Reese — Smooth" exists. [Overwrite]   │
│                                       [Cancel]  [Save]       │
└──────────────────────────────────────────────────────────────┘
```

`(+)` opens a type-ahead over the seeded vocabulary plus values already used
in this library. The duplicate-name warning keeps today's rule
(`check_save`, `plugin_presets.rs:216-237`). Names are unique per plugin
among user presets, case-insensitively. This is no longer needed for
identity, but it keeps name-addressed loads over the control API
unambiguous.

### 6.6 Where it appears

| Surface | What | Toolkit |
|---|---|---|
| Every first-party plugin editor header | compact bar (§6.2) + Browse overlay (§6.3) | egui |
| First-party plugins **hosted in another DAW** | the same. The library lives in the SDK and reads the same data dir. No host involvement | egui |
| App: bottom plugin panel header (`plugin_panel.rs:95-110`) | compact bar for **any** plugin, including third-party and GUI-less ones, with Browse opening the host browser filtered to this instance's plugin | iced |
| App: media browser, new **Presets** tab beside Files/Pool | library-wide browser across all plugins (a **Plugin** facet is added). Double-click loads onto the selected plugin slot if it is the same plugin. Dragging onto a strip's add-effect area adds that plugin with the preset (slice P8) | iced |
| App: add-effect / add-instrument pickers (`inspector/chain.rs:93`, `track_strip.rs:367`) | "▸ with preset…" submenu listing favourites for that plugin | iced |

A first-party plugin *with* an editor thus has two preset UIs in the app:
its own and the host bar. The host bar stays for three reasons. It is the
one that works for every plugin, it's what the panel shows while the editor
is closed, and ux consistency across first- and third-party plugins matters
more than removing a duplicate. Both read the same library, so they cannot
disagree about content. §7 covers keeping *identity* in sync.

### 6.7 Undo and the host mirror

- Host-initiated load (bar, browser, control API) stays the one-message
  recall of today (`LoadPluginPreset`, one undo entry). It is extended to
  carry the extra state as well (below).
- Audition bracket: provisional loads are sent as a non-recording variant
  (`UndoAction::Skip`). The commit records one entry whose "before" is the
  revert snapshot. Revert re-applies the snapshot, also without recording.
- **Loading the whole sound from the host.** Params stay param-by-param (so
  the mirror stays right, which is the reason given at
  `plugin_presets.rs:20-30`). The sound-bearing extra keys (§9.2) go through
  `LoadPluginState` with a *merge* document: the plugin's current state with
  those keys replaced. Where the plugin implements `clap.preset-load` (all
  first-party plugins after P5), the host instead calls
  `preset_load.from_location(PLUGIN, null, "<id>")`. The plugin then loads
  itself exactly as its own browser does, and the host applies the returned
  param values to its mirror from the preset file it already has. There is
  one code path per side, and host and editor load identically.

## 7. Modified indication and identity across the two sides

- **Plugin side.** Replace the sticky `mark_modified()` flag with a
  **comparison**. At load, the session stores the loaded preset's
  normalized param vector plus a hash of the sound-bearing extra state.
  `is_modified()` compares the live values against it, with a tolerance of
  1e-6 in normalized units. Turning a knob and back is therefore not a
  modification, and changes arriving from the host (automation,
  `set_plugin_param`, the generic panel) count, which `mark_modified` misses
  today. The comparison runs on the GUI thread at most every 100 ms while
  the editor is open, and on `save_state`. The hover text lists the changed
  params by name.
- **Automation.** A parameter under an active automation lane always differs
  from the preset during playback. The plugin can't tell automation from a
  user edit. The host can, and passes the set of automated param ids through
  a first-party main-thread extension call, `resonance.preset-session`
  (`set_ignored_params`). The plugin excludes those params from the
  comparison (D8).
- **Host side.** For our plugins, `clap_host_preset_load::loaded(location_kind,
  location, load_key)` tells the host the identity whenever the plugin loads
  a preset from its own browser. `resonance.preset-session` adds one
  callback, `modified_changed(bool)`, raised on the flag's edge. The engine
  turns both into a new event, `AudioEvent::PluginPresetIdentity {
  instance_id, preset: Option<(source, plugin_id, preset_id)>, modified }`. The app
  stores it on the plugin mirror, and `*.plugin_presets` finally reports a
  real `current` / `modified`. For third-party plugins, `loaded()` is used
  when the plugin calls it. Otherwise identity is what the host last loaded,
  and `modified` is **unknown**. The wire says `"modified": null`; the bar
  shows no dot rather than a lie.
- **Project persistence** keeps today's `"preset"` session key
  (`presets.rs:906-951`), now `{ id, source, name, loaded_hash }`. A
  legacy `{name, source}` key is resolved to an id by name once, at load.

## 8. Third-party CLAP plugins

| Tier | Which plugins | What works |
|---|---|---|
| **T0 — any CLAP with `clap.state`** | all | **User presets**: save via `SavePluginState`, stored as `state.encoding = "clap-state"` (base64 blob). When the plugin implements `clap.state-context`, the save is `save_ex(CLAP_STATE_CONTEXT_FOR_PRESET)` so the plugin can leave out project-only data. Load via `LoadPluginState` (or `load_ex(FOR_PRESET)`), which is **one** undo entry. The mirror is refreshed by re-querying param values after the load echo. Full metadata, favourites, tags, search and export/import all work, because they are host-side. `plugin.version` comes from the descriptor. Modified = unknown (§7). This fixes gap 5 as a side effect. |
| **T1 — plugins shipping a `clap.preset-discovery-factory`** | Surge XT, u-he, Vital-via-wrapper, … | The scan worker (off the engine thread, like input enumeration in `65614aad`) runs the provider, and receives name, creators → `author`, description, `add_feature` → `tags`, flags (`IS_FACTORY_CONTENT` → factory, `IS_FAVORITE` → seeds our favourite once, `IS_USER_CONTENT` → user-but-read-only), timestamps, and `PLUGIN`/`FILE` locations. The results are cached per plugin binary (by file mtime + size) in `library/discovered/<clap-id>.json`, so it doesn't re-index every start. Load via `clap.preset-load` `from_location`. These presets are read-only in our library (the plugin owns them) and can be marked (favourite, tags). |
| **T2 — plugins calling `clap_host_preset_load.loaded()`** | some | The host learns identity when the user loads a preset in the plugin's own GUI. That identity is shown in the bar and reported over MCP. |
| Not supported | — | Reading a proprietary preset directory, or editing a third-party factory bank. |

The host skin (§6.6) is the preset UI for third-party plugins. Their own GUI
may have another; we don't integrate with it.

Resonance's own plugins could *also* publish a `preset-discovery` factory so
Bitwig/Reaper list our factory and user presets. That is optional slice P10,
cheap once the store exists, and not needed for our own host (which uses the
symbol + library directly).

## 9. Plugin API changes (resonance-plugin)

### 9.1 Factory declaration

```rust
pub struct FactoryPreset {
    pub id: &'static str,    // new: stable slug, unique per plugin
    pub name: &'static str,  // unchanged literal (lockstep scanner)
    pub json: &'static str,  // now a format-1 preset file (§4.3)
}
```

The exported symbol grows to `[{id, name, json}]`, and `decode` stays
tolerant (`factory_presets.rs` "malformed entries are skipped").

### 9.2 Sound-bearing extra state

`ExtraStateSaver` gets one defaulted method:

```rust
/// Keys of `save()` that are part of the SOUND and belong in a preset
/// (amp: "model_ref"; ir: "ir_ref"; wavetable: "user_wavetables";
/// drums: "kit_ref"). Keys not listed are session/UI state and stay out.
fn preset_keys(&self) -> &'static [&'static str] { &[] }
```

`PresetBank::save` then writes `params_to_json` + those keys, and
`PresetBank::apply` applies both. Plugins with no saver are unchanged.
mastering's assistant genre/reference (`f2d7e3b3`) is *not* sound-bearing
and stays out.

### 9.3 External-file references

amp and ir (and drums' kit) store a path. A preset that stores only a path
breaks when the file moves. A preset that stores today's `file_select`
breaks when the directory changes (gap 2).

**Amp: adopt the NAM spec's state v2 (§5.2 there) as the preset payload.**
The amp's `preset_keys()` is `["model_path", "model_id", "model_name",
"model_source"]`, so a preset carries exactly what a project does. On load,
the NAM spec's `resolve_model` handles it: path, then `model_id` (sha256)
through the library with auto-relink, then `Missing`, where the reference is
kept and the banner offers Locate / Re-download. There is no separate
`model_ref` shape. `file_select` becomes the NAM spec's stable slot (§5.1
there), and it stays **in** the preset's params. It is correct on the
machine that saved the preset. On another machine the slot can point
elsewhere, so the rule is: **when a preset carries `model_id`, the id wins**.
The amp applies the extra keys after the params and rewrites `file_select`
to the locally resolved slot, which is at most one model load because the
loader only acts on the final value. A params-only amp preset (no
`model_id`, e.g. every legacy file) falls back to the slot, which is the
NAM spec's D2 behaviour.

**IR, drums: the same shape, when they become kinds** (§10.3): `<x>_path` +
`<x>_id` (content sha256) + `<x>_name`, resolved by the same
path → id → missing order. Until then they keep path-only extra state, which
is still better than today (P2 puts the path in the preset at all).

## 10. Relation to `nam-model-library.md`, and NAM models as an asset kind

### 10.1 Recommendation: one kind for marks and the browser, separate content

NAM models are **one kind (`amp-model`) of the shared library** for marks
(favourites, tags, recents), tag vocabulary, freshness polling, the list
widget, the browser view-state and confirm-in-place delete. They are **not**
presets: they have a separate content store, identity scheme and file format.

| Shared (one implementation) | Separate per kind |
|---|---|
| `library_marks`: `marks.json`, `<kind>:<id>` keys, per-item read-modify-write under a lock, `generation`, orphan pruning, vocab + tag completion, freshness helper, reveal launcher | Content and enumeration: preset JSON files (`resonance-plugin::presets`) vs `.nam` files + sidecars + `library.json` (`resonance-common::nam_library`) |
| `library_view::BrowserModel` over `LibraryRows`: search, facets, favourites-first sort, ◀/▶ over the current view, audition bracket, confirm-in-place | Identity: preset UUID / factory slug vs file sha256 |
| egui list widget, `star_toggle`, `tag_pill`, detail-pane frame | Actions: Save-as / Edit info (presets) vs Import-copy, Re-download, relink, slots (NAM) |
| Facet vocabulary for `instrument` / `character` / `genres` (a model is "for" `electric-guitar` and has a character); NAM's `gear_type` / `tone_type` are NAM-only facets | Columns and detail content |

The reasons:

1. The user asks for the same things of both (favourite, search, tag,
   delete, import). Two stores would mean two stars that don't know about
   each other.
2. Presets **reference** models (§9.3). An amp preset's "Missing → Re-download"
   needs the NAM library's identity and actions, and a model's detail pane
   can show "used by 3 presets" only if both share one key space.
3. A `.nam` file isn't a preset. It's a foreign format of 1–50 MB with a
   download lifecycle and no Resonance state. Wrapping it in a preset
   envelope would copy it for nothing.

### 10.2 Reconciliation with the NAM spec

**Adopted from the NAM spec unchanged:** the `library/marks.json` path, the
`library_marks` module name and its place in `PLUGIN_COMMON_ITEMS`; the
`{version, items}` schema with `favorite / tags / last_used / use_count`;
the `<kind>:<id>` key with ids as opaque strings; the kind name `amp-model`
and sha256 content ids (its D1); per-item read-modify-write with atomic
replace; recents only from user picks (its D10); poll-based refresh with no
watcher dependency; confirm-in-place delete; the reveal launcher in common;
state v2 + `resolve_model` + stable slots (§9.3 above); the `set_marks`
method shape (§12.2 below mirrors `amp_models.set_marks`).

**Changes this spec asks the NAM spec to make** (it said it would follow
this spec on path and key scheme; these are the remaining differences):

1. **Take a lock around the marks read-modify-write.** NAM §8 does the
   marks RMW without a lock, which loses one of two updates when two
   processes write inside the same few milliseconds. Use
   `std::fs::File::lock` on `library/marks.lock` (stable since Rust 1.89;
   the toolchain here is 1.96). The OS releases it when a process dies, so
   the same primitive should also replace NAM §8's `create_new` lockfile
   with a 10 s staleness rule for slot allocation. That rule can break a
   live lock under a slow disk.
2. **Add a top-level `generation` counter to `marks.json`** (NAM §8 already
   mentions one, but the §4.3 schema doesn't show it), bumped on every write.
   Both libraries' freshness checks read it.
3. **Split the browser into an ungated view-state and a gated widget.**
   NAM §10 puts both "behind `editor-widgets`". The pure view-state should
   be `resonance-plugin::library_view` (no feature gate), because the iced
   app drives the same model. Only the egui widget stays behind
   `editor-widgets`.
4. **Name the seeded facet vocabulary as part of `library_marks`** (§4.4
   here). This lets the NAM manager offer `instrument` (`electric-guitar`,
   `bass`, …) and `character` chips beside its own `gear_type` /
   `tone_type`, and lets its tag completion suggest the seeded values.
5. **Test binary.** NAM §11 offers `resonance-common/tests/library_marks.rs`
   "or a module in the preset spec's test binary". Recommendation: the NAM
   spec's file name, `library_marks.rs`, owned by whichever slice lands
   first. This spec's §14 uses it too.

Nothing here conflicts with NAM's D4 (permanent model delete) versus this
spec's D10 (preset trash). Models can be re-downloaded or are copies, and
user presets are neither, so the delete *widget* is shared and the delete
*policy* is per kind.

### 10.3 Other kinds later

IR files (`resonance-ir`), drum kits (`registry::ContentType::Drumkit`),
user wavetables and **track presets** (§1.3) are all natural next kinds.
Track presets would gain favourites/tags/genre and a Presets-tab listing in
the add-track menu without changing their file format. That is slice P9 and
not required for the plugin-preset work.

## 11. Concurrency across instances and processes

Actors: N instances of each plugin (in the app, plus possibly a second DAW
process), the app itself (control API + host browser), and the user editing
files by hand.

1. **Preset files: one writer per file, atomic replace.** Every write goes
   through `resonance_common::atomic_write` (temp + fsync + rename). This
   replaces `std::fs::write` at `presets.rs:357/394/431`. Two concurrent
   saves of *different* presets never touch the same file. Two concurrent
   saves of the *same* preset are last-writer-wins, and each file is always
   whole. Unparsable files are quarantined (`quarantine_corrupt`), as the
   track presets already do.
2. **Marks store: lock, re-read, apply, replace.** A mutation takes an
   exclusive advisory lock on `library/marks.lock` (`std::fs::File::lock`,
   stable since 1.89; toolchain here is 1.96), re-reads `marks.json`,
   applies **only its own delta** (set favourite on key K), bumps
   `generation`, and atomic-replaces the file. A star set in instance A and
   a tag added in instance B a second later both survive. The lock is held
   for milliseconds, and it is never taken on the audio thread (marks
   are GUI/main-thread only). This is the NAM spec's per-item
   read-modify-write plus the lock (§10.2 item 1).
3. **Change propagation.** Within one plugin `.so`, instances share one
   `Arc<PresetLibrary>` through a `OnceLock` (every cdylib has its own
   statics, which is fine; the NAM spec does the same for its index). A
   write updates it immediately, and other instances see it on their next
   frame. Across `.so`s and processes, the shared freshness poll (§4.6:
   500 ms with a browser open, 2 s from the bar) picks it up. There is no inotify dependency; the
   poll is portable and runs only while a browser or bar is visible.
4. **Save/rename race on the same id.** Rename is "write new file, then
   delete old". A concurrent reader might briefly see both; the index
   dedups by id and keeps the newer mtime.
5. **Stale loaded identity.** Instance A has "Reese Wide" loaded, and B
   deletes it. A keeps sounding, and its bar shows `Reese Wide (deleted)` in
   `TEXT_3` until A loads something else. Saving from A then offers
   "Save as…" only.
6. **Host save vs plugin save.** The host still writes via the pending-save
   + `state_saved` echo. Only one `pending_plugin_preset_save` exists today
   (`state/presets.rs:29`). It becomes a map keyed by instance id, so two
   agent saves on different plugins in the same tick can't drop one.

## 12. Control API and MCP extensions

Rules: additive fields on existing methods (no `PROTOCOL_VERSION` bump,
`resonance-control/src/lib.rs:22-24`), one MCP tool per new method, and each
method lands wire + handler + tool + skill-doc line + tests together.

### 12.1 Existing per-surface methods (extended)

`track|bus|master.plugin_presets` params gain optional filters:

```text
query?: string          # same syntax as the browser: "reese tag:ferrous is:fav"
favorites_only?: bool
source?: factory|user|discovered
category?: string
instrument?: [string]   genres?: [string]   character?: [string]   tags?: [string]
sort?: name|bank|recent|modified
limit?: u32 (default 100)   offset?: u32
```

`PluginPresetEntry` gains `id`, `category`, `instrument`, `genres`,
`character`, `tags` (merged content + personal), `favorite`, `author`,
`description`, `plugin_version`, `modified_at`, `derived_from`. The view
gains `total` (pre-limit) and `facets` (value → count for each facet), and
reports real `current` + `modified: Option<bool>` (§7).

`*.load_plugin_preset` gains `preset_id?`. When present it wins over
`preset` (name). Name lookup stays, case-insensitive, user-before-factory,
and errors if two user presets would match (impossible under §6.5's
uniqueness, kept as a guard). It also gains `extra: bool = true`: load the
sound-bearing extra state too (§6.7).

`*.save_plugin_preset` gains `meta?: { author, description, category,
instrument, genres, character, tags }`, `favorite?: bool`, and
`overwrite_id?` to update a specific preset in place. The reply gains the
`id`. The ack is still "capture armed"; `id` is minted up front so the agent
can refer to it before the file lands.

### 12.2 New library methods (no plugin instance needed)

| Method | Params → result | Why |
|---|---|---|
| `presets.search` | `{ plugin_id?, query?, …filters as above }` → `{ total, hits: [PluginPresetEntry + plugin_id], facets, library_generation }` | "Find a warm vocal compressor preset" before the plugin is even on a track. Agents pick a plugin *and* a preset in one read. Presets only: amp models are listed by the NAM spec's `amp_models.list`, which has its own fields, and a cross-kind search can come later |
| `presets.set_marks` | `{ plugin_id, preset_id, favorite?, tags? }` → `{ entry }` | Same shape and semantics as the NAM spec's `amp_models.set_marks`: favourite + personal tags on **any** preset, factory included. Per-user state: no undo entry, no project `revision` bump, and the description says so |
| `presets.update_meta` | `{ plugin_id, preset_id, set?: {…meta}, add_tags?, remove_tags? }` → `{ entry }` | Edits a **user** preset's content metadata (name excluded, see rename). It is refused on factory presets with a pointer to `set_marks`. Not undoable (library, not project) |
| `presets.rename` | `{ plugin_id, preset_id, name }` → `{ entry }` | User presets only |
| `presets.delete` | `{ plugin_id, preset_id, confirm?: bool }` → `{ trashed_path }` | Destructive: refused with a summary until `confirm: true`, per convention. Trash, not unlink |
| `presets.vocabulary` | `{}` → `{ categories, instrument, genres, character }` (seeded + in-use) | So an agent tags consistently instead of inventing near-duplicates |

Also additive: `track.add_instrument` / `*.add_effect` gain
`preset?: {id | name}`, so "add the wavetable with *Pad — Juno Chorus*" is one
call and one undo entry.

MCP tools: `presets_search`, `presets_set_marks`, `presets_update_meta`, `presets_rename`,
`presets_delete`, `presets_vocabulary` (`resonance-mcp/src/tools/presets.rs`,
new file in the tools module; not a test target). The skills (`mixing`,
`spatial`, `resonance-synth-design`) gain one line each: "prefer
`presets_search` with `instrument`/`character` over guessing names; save
what you design with `meta` so it's findable next session". The
agent-made `Ferrous …` presets show why this matters.

## 13. Migration

No real users, so this is a converter, not a compatibility layer.

- **User presets** (`plugin-presets/<id>/*.json` without `"format"`):
  `presets::migrate::convert_legacy_dir` runs once per plugin directory on first index.
  For each file it wraps `{version, params, …extra}` into `state.doc`, strips
  the `"preset"` session key, and mints `id`. It sets `meta.name` from
  `"name"` (or the stem), and `category` from a `"<X> — <Y>"` / `"<X> -
  <Y>"` / `"<X>___<Y>"` prefix when X is in the category vocab. `plugin.version`
  is `null` (unknown). The file is renamed to `<stem>-<id8>.json` and the
  original kept as `.legacy` beside it until the next start. It is
  idempotent: files with `"format"` are skipped. Amp presets with only
  `file_select` are converted as-is, and the converter logs that they carry
  no model reference.
- **Factory banks**: one mechanical commit per plugin adds `id` + `meta` to
  each JSON (converter-generated, hand-reviewed for genre/character), and
  moves delay's inline strings to files.
- **Project state** `"preset": {name, source}` → resolved to an id by name
  at load (§7), and written back in the new shape on the next save.
- **Control API**: name-addressed calls keep working, so nothing in
  `resonance-agent-plugin` changes in the migration slice itself.
- Track presets are untouched.

## 14. Tests

Layout per CLAUDE.md: `resonance-app/tests` and `resonance-audio/tests` get
**modules in existing group binaries**, never new top-level files. Nothing
goes in inline `#[cfg(test)]` modules (`feedback_no_inline_tests`). App tests
build with `Resonance::new_for_test()` and set the preset root through
`test_set_plugin_preset_root` / `RESONANCE_LIBRARY_DIR`.

| Where | Module | Covers |
|---|---|---|
| `resonance-common/tests/library_marks.rs` (new, shared with the NAM spec, owned by whichever slice lands first; that crate has one file per area, e.g. `registry.rs`) | — | two writers with different items both survive **under concurrent processes**: spawn the test binary as a child to hold the lock; same item is last-writer-wins; `generation` bumps; kinds don't collide; orphan pruning at 90 days with an injected clock; recents not written for restores; vocab + tag completion across kinds |
| `resonance-plugin/tests/presets.rs` (extend: store part) | — | id minting and persistence across rename; marks merge (factory favourite); atomic write leaves no torn file (write to temp, never rename); query: tokens, `tag:`/`genre:`/`is:fav`, facet counts with the other facets applied, sorts, accent-insensitivity; legacy converter on fixture copies of today's shapes (editor-saved, host-saved with `"preset"` key, hand-dropped without name); trash and purge |
| `resonance-plugin/tests/library_view.rs` (new; the view-state is its own module) | — | `BrowserModel` over a fake `LibraryRows`, so the NAM manager is covered by the same tests: stepping walks the filtered list and clamps; audition → revert restores the exact prior state including extra keys; audition → commit; save-as copies meta and sets `derived_from`; rename keeps id and session identity; delete of the loaded preset keeps the sound; comparison-based `is_modified` (knob and back = clean; host-side change = modified; ignored params excluded) |
| `resonance-plugin/tests/preset_bar_render.rs` (extend) | — | headless egui frames (`egui::__run_test_ui`) of the compact bar, the wide and narrow browser overlays, and the metadata form: id collisions, borrow conflicts, enabled/disabled states |
| `resonance-plugin/tests/fleet_preset_adoption.rs` (extend) | — | every factory entry has a unique `id`, a `meta` block with `category`, and `meta.name == name`; the factory id set is pinned; every editor draws `preset_bar` (unchanged) |
| `resonance-plugin/tests/state.rs` (extend) | — | `preset_keys` round-trip: amp/ir/wavetable/drums presets carry their asset reference; `file_select` is excluded |
| `resonance-audio/tests/clap_host/` | `preset_load.rs`, `preset_discovery.rs` | host calls `preset-load.from_location` on a first-party cdylib and receives `loaded()`; the discovery indexer against a first-party provider (P10) or a fixture provider; `save_ex(FOR_PRESET)` is used when `state-context` is present |
| `resonance-app/tests/control/` | extend `control_plugin_presets.rs`, `control_chain_presets.rs`; add `control_presets_library.rs` (module in `control.rs`) | filters and facets; load by `preset_id`; save with `meta` returns an id that `presets.search` finds; `update_meta` on a factory preset; `delete` refused without `confirm`; third-party blob save/load round-trip through a non-UTF-8 fake blob; `current`/`modified` after an identity event |
| `resonance-app/tests/mixer/` | `plugin_panel_preset_bar.rs` (module in `mixer.rs`) | iced golden PNG of the panel header bar (clean, modified, deleted-identity states); the audition bracket is one undo entry and revert records none |
| `resonance-app/tests/io/` | extend `preset_name_collisions.rs`; add `preset_legacy_convert.rs` | project `"preset": {name}` resolves to id; converter idempotency against the real-world shapes in §1.1 |
| `resonance-mcp/tests/agent_plugin_lockstep.rs` | — | unchanged scanner (names stay literals); if D7 goes the other way, the scanner reads `meta.name` from `presets/*.json` in the same slice |

Plugin editor overlays have no golden coverage (`plugin-audit-plan.md` §3),
so each egui slice also gets a human look from a Wayland session. Guard any
per-scenario silence in audio goldens as usual
(`feedback_silent_goldens_are_vacuous`).

## 15. Build plan (vertical slices)

Each slice is mergeable on its own and leaves both surfaces consistent.

| Slice | Contents | Surfaces |
|---|---|---|
| **P0 — store** | `resonance_common::library_marks` (marks with lock, generation, vocab, freshness, reveal), which is the same module as the NAM spec's L4. Whichever lands first builds it, to §10.2's shape. Add it to `PLUGIN_COMMON_ITEMS`; plugins reach it via `resonance-plugin` re-exports, so `PLUGINS_ON_COMMON` doesn't grow. `resonance-plugin::presets::PresetLibrary` (format 1, ids, index, query, atomic writes, trash); `PresetBank`/`PresetSession` re-based on it with `PresetRef {id,…}`; legacy converter; the bar stops scanning per frame | editor bar unchanged visually; control API reports `id` |
| **P1 — factory metadata** | `FactoryPreset.id`; `id` + `meta` in all 95 factory files; delay to files; fleet tests; symbol carries id | factory metadata visible over `*.plugin_presets` |
| **P2 — whole-sound presets** | `preset_keys`, asset refs for amp/ir/drums (§9.3), `Param::preset_excluded`, host load merges extra state | editor + host load/save identical |
| **P3 — favourites + filters over MCP** | `*.plugin_presets` filters/facets/meta fields; `presets.set_marks`, `presets.update_meta`, `presets.vocabulary`; save with `meta`; editor bar ☆ toggle | agent can tag/favourite/filter; human can star |
| **P4 — plugin browser** | `library_view::BrowserModel` + the egui list widget (shared with NAM L3); egui overlay (wide + narrow), metadata form, rename/duplicate/delete/import/export, audition bracket; `star_toggle`/`tag_pill` in `plugin_gui_core::widgets`; `presets.rename`, `presets.delete`, `presets.search` | full browser in all 13 editors + the matching MCP tools |
| **P5 — identity to host** | `clap.preset-load` in the bridge; `resonance.preset-session` extension (modified edge, ignored params); `AudioEvent::PluginPresetIdentity`; comparison-based modified | `current`/`modified` real over MCP |
| **P6 — host UI** | iced bar in `plugin_panel.rs`; Presets tab in the media browser; audition as one undo entry; "with preset…" in add pickers + `preset` on `add_effect`/`add_instrument` | host has what the editor has |
| **P7 — third-party T0** | `clap-state` encoding, `state-context` for presets, blob load, mirror refresh | third-party user presets on both host surfaces + MCP |
| **P8 — third-party T1/T2 + drag-to-add** | discovery indexer on a worker, cache, `preset-load` for discovered presets, `loaded()` identity; drag preset onto a strip | — |
| **P9 — more kinds** | NAM models need no slice here: they arrive as the NAM spec's L3/L4 on the shared modules. Then track presets, IRs and kits as kinds | shared stars/tags across kinds |
| **P10 (optional)** | our plugins publish `preset-discovery` for other DAWs | — |

P0 → P1 → P2 are on the critical path. P3 can go right after P1. P4 needs
P0–P2. P5 and P7 are independent of P4, and P6 needs P4's model and P5.
**NAM coordination points:** (1) `library_marks` is built once, by P0 or
NAM L4, whichever comes first, to the §10.2 shape. (2) `library_view` +
the list widget are built once, by P4 or NAM L3. If NAM L3 comes first it
builds the ungated view-state split of §10.2 item 3. (3) This spec's P2
(amp `preset_keys`) needs NAM L2 (state v2 + slots) and must not land
before it.

## 16. Open decisions

- **D1: where each piece lives (recommended: marks + vocab in
  `resonance_common::library_marks`; preset content store in
  `resonance-plugin::presets`; view-state in ungated
  `resonance-plugin::library_view`).** Marks must be reachable by the amp
  (which depends on `resonance-common`) and by the app. Preset content is an
  SDK concept that the app already reaches through `resonance-plugin`
  (`plugin_presets.rs:46`), so it doesn't need to move down a layer. The cost
  is one `PLUGIN_COMMON_ITEMS` entry, which the NAM spec takes as well.
  *Alternative:* the whole preset store in `resonance-common`. That puts more
  in the lowest layer for no reader that needs it.
- **D2: content metadata embedded vs sidecar (recommended: embedded, marks in one per-user index).**
  Embedded meta travels with export and needs no second file per preset.
  User-only state never pollutes a shared file. A per-preset sidecar would
  double the file count and still need an index for search.
- **D3: tags on user presets: file or marks? (recommended: file.)**
  The user owns the file, so their tags are content and export with it.
  Factory presets get mark tags. *Alternative:* all user tagging in
  marks. That is simpler, but an exported preset would lose its tags.
- **D4: rating stars (recommended: not in v1).** Favourite is binary and
  already covers "the good ones". Ratings add a column and a sort for a
  single user with a few hundred presets. The marks schema reserves
  `rating: u8?` so adding it later is additive.
- **D5: instrument audition sound (recommended: host-side "preview" button
  that loops the track's selected clip / a held C3 through the engine, P6;
  none inside plugins in v1).** A plugin can't inject notes into its own
  input portably, and a note generator in every instrument is fleet-wide
  work for one convenience.
- **D6: bundle referenced files in export (recommended: no in v1; reference
  by kind/id/hash).** NAM models are several MB and have their own download
  source, so the importer can offer "Re-download" through the NAM library.
  Revisit with IRs, which are small and often personal.
- **D7: factory names as Rust literals vs JSON only (recommended: keep the
  literal, assert equality with `meta.name`).** This keeps
  `agent_plugin_lockstep.rs` untouched and costs one duplicated string per
  preset, guarded by a test. *Alternative:* the name only in JSON, with the
  scanner rewritten to read it, in the same slice.
- **D8: modified under automation (recommended: host passes automated
  param ids; the plugin ignores them).** *Alternative:* show modified and
  let the hover text explain. That is simpler, but every automated synth
  preset then shows the dot forever.
- **D9: host bar for first-party plugins whose editor is open (recommended:
  keep both).** Rationale in §6.6. *Alternative:* hide the host bar while
  our editor is open, to avoid two pickers on screen.
- **D10: trash retention (recommended: 30 days in `.trash/`, purged at
  library open).** Both surfaces can delete, one of them an agent, so a
  recoverable delete is cheap insurance.
- **D11: user preset name uniqueness (recommended: unique per plugin,
  case-insensitive, among user presets).** Ids make it unnecessary for
  identity, but name-addressed MCP loads and humans reading a list both
  want it. Duplicates are still allowed *across* factory/user, as today.

## 17. Out of scope

A cloud preset exchange; preset morphing/A-B slots; per-project "preset
collections" beyond tags (a `project:<name>` tag convention covers the
`Ferrous …` use); editing third-party factory banks; importing other
vendors' native preset formats (`.fxp`, `.vstpreset`, `.nksf`); MIDI program
change → preset mapping (the external-instrument device presets of epic #40
are a different system).
