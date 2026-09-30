# NAM model library: managing installed amp models

Status: **design, not built** (2026-09-30). Build it as the vertical slices in
§12. Each slice lands its storage change, plugin behaviour, editor UI and tests
together, and the control-API slices follow the one-tool-per-method rule
(`project_control_api_vertical_slices`). Touches `resonance-common` (new
library module), `plugins/resonance-amp` (state, selector, editor) and, in the
MCP slices, `resonance-control` / `resonance-app` / `resonance-mcp`.

## 0. Why

The amp plugin can **get** NAM models (the Tone3000 browser, `Load Model…`)
but it cannot **manage** them. After a download the model is a file with a
mangled name in a directory the user never sees. There is no way to delete it,
star it, see who captured it or what gear it is, find it again by searching,
or tell which models a project depends on. The only way to move through the
downloads is ◀/▶ in alphabetical file-name order. The user's words: "Once
they are downloaded we cannot remove them, or favorite them or something."

This spec adds a per-user **model library**: an index of installed models with
real metadata, favourites, tags and recents, plus a **Library** panel in the
amp editor for searching, deleting, revealing, importing and re-downloading. It
also fixes the missing-model case, which today reads as a raw I/O error
(§2 G5).

## 1. What already exists (verified against master @ 4f7a6baf)

| Piece | Where | Relevant facts |
|---|---|---|
| Download source | `src/tone3000/{client,worker,auth}.rs` | Tone3000 REST API (`https://www.tone3000.com/api/v1`), OAuth 2.0 PKCE with a loopback redirect. Search is `/tones/search` with the gear filter `amp_amp-cab` (`client.rs:25`) and `format=nam`. A tone's models come from `/models?tone_id=` (`client.rs:266`). The bytes are fetched from a **pre-signed `model_url` that rotates** (`types.rs:83-87`), so a re-download has to re-list the tone first. Tokens are stored at `config_dir()/resonance/tone3000.json` (`auth.rs:29`). |
| Where downloads land | `src/models.rs:9-16`, `worker.rs:421-448` | `data_dir()/resonance/amp-models/tone3000/` (on Linux `~/.local/share/…`). The file name is `sanitize_filename(model.display_label(), model.id)` (`worker.rs:450`), so it becomes `<model name>_(<size>)_<id>.nam` with non-ASCII characters dropped. **Every piece of tone metadata is discarded** at download: tone title, author, gear, description and tone id (`Tone` at `types.rs:31`). No sidecar or index is written. |
| Installed registry | `resonance-common/src/registry.rs` | `installed.json` has a `ContentType::AmpModel` variant (`:37`), but **the amp never calls `mark_installed`**. Only drums uses the registry. Its entries are keyed by name and carry only a date. |
| Model list / picker | `params.rs:19-23`, `resonance-plugin/src/loader.rs:55` | `file_select: IntParam` (0..=999, **visible**, `params.rs:81-89`) is an index into `file_list: Arc<Mutex<Vec<String>>>`, the sorted `.nam` paths of **one directory**: the directory of the loaded model (`rescan_directory`), or the downloads directory for a fresh amp (`lib.rs:197-206`). `Load Model…` replaces `file_list` with the chosen file's own directory (`header.rs:172-195`), so the ◀/▶ set silently switches directories. |
| Editor | `src/editor/{app,header,tone3000_panel}.rs` | egui via `plugin-gui-core`, 960×620 (min 760×520). The header strip holds `Load Model…`, `Browse Tone3000…` (a solid-accent button), the shared preset bar, ◀/▶, the model name, an `n / N` counter and the sample-rate warning (`header.rs:27-160`). Tone3000 is a full-window overlay `egui::Area` over a dimmed editor (`tone3000_panel.rs:111-135`, drawn from `app.rs:62`). The model name shown is the **file stem** (`lib.rs:90-94`, `loader.rs` success arm). |
| Persistence | `lib.rs:339-358` | `AmpExtraState` saves one key, `model_path` (an absolute path string), chained behind `PresetSession::with_extra` (`lib.rs:127-131`). `initialize()` re-derives `file_select` from the path (`lib.rs:185-196`), so **in a project the path wins over the index**. |
| Presets | `resonance-plugin/src/presets.rs:33-40, 516` | User presets are `data_dir()/resonance/plugin-presets/<clap-id>/*.json`. A saved preset holds **params only**: for the amp that means `file_select`, which is an index into whatever directory is current. The extra state (`model_path`) is not stored in presets. |
| Missing model at load | `lib.rs:78-101`, `nam/parse/mod.rs:64-66` | If `model_path` no longer exists, `rescan_directory` returns index 0 and the sync load fails. `model_name` becomes `"Error: Failed to read file: No such file or directory (os error 2)"`, which the header prints verbatim. No model is installed, so the processor passes the signal through dry × gain (`dsp/processor.rs:194-196`): **a clean DI signal with no other warning**. `model_path` keeps the dead path, so a re-save keeps the reference, which is good. |
| NAM metadata | `.nam` JSON `metadata` object; `nam/parse/schema.rs:17` | Real files carry `name`, `modeled_by`, `gear_type`, `gear_make`, `gear_model`, `tone_type`, `input_level_dbu`, `output_level_dbu`, `loudness`, `gain`, `date` and `training.validation_esr` (see `tests/fixtures/a1/wavenet.nam`). **The parser ignores all of it.** Only `sample_rate` is read. |
| Per-instance workers | `lib.rs:133-145` | Every `ResonanceAmp::new()` spawns its own Tone3000 worker thread (`worker.rs:202`), even when no editor is ever opened. N amps means N workers, each with its own token copy. |
| Delete precedent | `plugins/resonance-drums/src/editor/download_panel.rs:24-26, 247-290` | Drums deletes an installed kit with a two-click `Delete` → `Confirm?` / `Cancel` in the row, then `remove_dir_all` and `registry::remove_installed`. This is the fleet's only content-delete UI. The preset bar's `Delete` (`preset_ui.rs:105-110`) has **no** confirm. |
| Reveal precedent | `resonance-app/src/update/external_instrument.rs:480` | `reveal_path_in_file_manager` (xdg-open / open / explorer) is `pub(crate)` in the app, so plugins cannot reach it. |
| Shared widget kit | `plugin-gui-core/src/widgets.rs`, `theme.rs` | `chip_button`/`chip_styled`, `segmented`, sliders, knobs; the lavender tokens (`PANEL`, `PANEL_LIGHT`, `BORDER`, `ACCENT`, `TEXT`, `TEXT_DIM`, `WARN`, `DANGER`, `GOOD`). Everything else in the amp editor is stock egui (`TextEdit`, `ComboBox`, `ScrollArea`, `Frame`, `Button`, `selectable_label`). |
| MCP surface | `resonance-control/src/methods/track.rs:624-719` | The only way in is `file_select` through `*.plugin_params` / `*.set_plugin_param`. It reports a bare index: `file_select` has no `value_to_string`, and its 1000 steps are past `MAX_CHOICE_STEPS = 64` (`resonance-audio/src/clap_host/param_meta.rs:27`), so there is neither `text` nor `choices`. `ParamValue::Label` resolves **only** against `choices` (`track.rs:726-750`). An agent therefore cannot tell which model is loaded, and cannot pick one by name. The host already calls CLAP `value_to_text` (`clap_host/instance.rs:475`). The plugin bridge implements `text_to_value` (`resonance-plugin/src/clap_bridge/params.rs:75`), but the host never calls it. |
| Layering | `tools/arch-invariants/tests/architecture.rs:456-468` | Plugins may name only the `resonance_common` items in `PLUGIN_COMMON_ITEMS` (`scan_directory`, `registry`, …). A new library module needs a row there, and the amp builds `resonance-common` with `default-features = false`. |
| Test layout | `plugins/resonance-amp/tests/` | One binary per concern. `model_selector.rs` isolates the downloads dir with `XDG_DATA_HOME` and keeps **one** `#[test]` because the variable is process-global. Editor presentation is tested through pure helpers (`tone3000_browser.rs` → `tones_heading`), not through an egui harness. The group-binary rule in CLAUDE.md applies to `resonance-app/tests` and `resonance-audio/tests`, not to plugin crates. |

In-flight work: `git branch -a` shows nothing library-shaped (the only amp
branch is `amp-block-forward`, which is merged). `git log -i --grep=nam|tone3000|favourite`
turns up the Tone3000 browser (`f84fbf9a`, `1ae34793`, `9854253d`), the
drumkit registry (`96ea1568`) and the app media browser's favourites
(`94bd175d`, `e1c33d77`, which stores folder favourites in `settings.json`,
`resonance-app/src/settings.rs:58`). Nothing duplicates this spec. The sibling
spec `plugin-preset-library.md` is being written at the same time (§10).

## 2. Gaps found while reading

- **G1: metadata is thrown away twice.** Tone3000's title, author and gear
  are dropped at download (`worker.rs:421`), and the `.nam` file's own
  `metadata` is dropped at parse. The UI can only show a sanitized file stem.
- **G2: the selector is a directory listing, not a library.** Its contents
  depend on where the last model came from, and `Load Model…` quietly swaps
  the set.
- **G3: the index is unstable.** `file_select` is a position in a sorted
  listing. Adding or deleting any file shifts every index after it. A
  `file_select` automation lane and every saved amp **preset**, which stores
  only the index, then recall a different model. Projects survive this
  because the path wins at `initialize`. Presets and automation do not.
- **G4: nothing is deletable,** and deletion would make G3 worse today.
- **G5: a missing model is a raw error string plus a silent dry signal.**
  There is no relink, no re-download, and no way to learn which model it was
  beyond the dead path.
- **G6: no identity beyond the path.** A model that was moved, renamed or
  downloaded twice has no stable id, so neither favourites nor relink can
  follow it.
- **G7: MCP is blind** to which model is loaded and cannot choose one (see the
  MCP row in §1).
- **G8: N workers for N amps** (§1). Harmless today, but a shared library
  should not multiply it.

## 3. Goals / non-goals

**Goals**

1. A per-user library of every installed `.nam`, with name, author, gear,
   capture type, sample rate, size, architecture and source.
2. Favourites, free-form tags and recently-used models, stored per user and
   shared with the plugin-preset library's store (§10).
3. A Library panel in the amp editor with search, filters, favourites first,
   delete with confirm, reveal in the file manager, import of a local `.nam`,
   and re-download for Tone3000 models.
4. A missing model is shown as missing, with the model's name, and can be
   fixed in one click (auto-relink by content hash, re-download, or locate).
5. A model selection that stays stable: adding or deleting models does not
   change what a preset or automation lane recalls.
6. Both surfaces (plugin-audit-plan.md §0): what the GUI can do to the
   library, an agent can read, and can do where that is safe.

**Non-goals**

- Embedding `.nam` files in projects, or a "collect project assets" step. The
  path/hash reference stays. That is a pool feature for another day.
- Browsing more of Tone3000's catalogue (cab/pedal gear types; the audit's
  §2 "not doing now" list keeps the gear filter).
- Loudness normalisation from `metadata.loudness`. The index stores it, but no
  behaviour uses it.
- A library for the IR plugin. The module is written so IR could adopt it
  (IR has the same `file_select` shape), but IR is not migrated here.
- Syncing favourites across machines, and Tone3000 account favourites.

## 4. Library data model

### 4.1 On disk

```
$XDG_DATA_HOME/resonance/amp-models/          ← library root (env override: RESONANCE_AMP_MODEL_DIR)
  tone3000/<name>_<model_id>.nam              ← downloads (existing dir, existing names kept)
  tone3000/<name>_<model_id>.nam.meta.json    ← NEW provenance sidecar, written at download
  imported/<original file name>.nam           ← NEW: copies made by Import
  library.json                                ← NEW: index + slot table (a cache, rebuildable)
$XDG_DATA_HOME/resonance/library/marks.json   ← NEW: favourites/tags/recents, shared with presets (§10)
```

- The **file and its sidecar are the truth**. `library.json` is a cache.
  Deleting it costs a rescan (and the slot table, §5.1), nothing else.
- The sidecar travels with the file, so a user who copies the `tone3000/`
  folder to another machine keeps the provenance. It holds only what the file
  cannot say about itself:
  `{ "source": "tone3000", "tone_id", "model_id", "tone_title", "author", "gear", "model_name", "size", "downloaded_at" }`.
- `RESONANCE_AMP_MODEL_DIR` overrides the root. It is the same pattern as
  `RESONANCE_PLUGIN_PRESET_DIR` (`presets.rs:57`), and it lets tests use a
  temporary directory without setting the process-global `XDG_DATA_HOME`.

### 4.2 Entry

| Field | Source | Notes |
|---|---|---|
| `id` | sha256 of the file bytes, hex | **Identity.** It survives renames and moves and makes duplicates visible. Computed once per (path, size, mtime) and cached in `library.json`. |
| `slot` | slot table (§5.1) | The `file_select` value. Stable across adds and deletes. |
| `path` | scan | Absolute. |
| `name` | sidecar `tone_title` › `metadata.name` › file stem | Display name. A Tone3000 tone with several sizes shows as "Title · size". |
| `author` | sidecar `author` › `metadata.modeled_by` | |
| `gear` | `metadata.gear_make` + `gear_model` › sidecar `gear` | e.g. "Darkglass Microtubes 900 v2". |
| `gear_type` | `metadata.gear_type` | `amp` / `amp_cab` / `pedal` / … as the file says. Filter chip. |
| `tone_type` | `metadata.tone_type` | `clean` / `crunch` / `hi_gain` / `fuzz` / … ("capture type"). Filter chip. |
| `architecture` | top-level `architecture` + version | `WaveNet A1`, `WaveNet A2`, `LSTM`, … |
| `sample_rate` | `sample_rate` (default 48 000) | A mismatch with the host rate is badged in the row (reuses the header's `format_khz`). |
| `size_bytes`, `mtime` | stat | |
| `source` | sidecar › `imported` › `external` | `tone3000 {tone_id, model_id}` enables Re-download. |
| `esr` | `metadata.training.validation_esr` | Shown in the detail pane only. |
| `loudness_db` | `metadata.loudness` | Stored, unused (non-goal). |
| `status` | scan / last load | `ok`, `unreadable` (parse failed, with the reason), `duplicate_of: <id>`. |

Reading metadata must not build the model. `nam::parse` gains a
`read_header(path) -> NamHeader`. It deserializes the top-level fields with
`weights` as `serde::de::IgnoredAny`, so it costs a JSON scan of 1–50 MB but
no allocation for the weights. It lives in `resonance-common::nam_library`, so
the app can index without linking the amp crate. The amp keeps its own full
parser.

### 4.3 Marks (favourites, tags, recents)

These are per user, never per project, and keyed by **content id** so they
follow the model across renames:

```json
{ "version": 1,
  "generation": 412,
  "items": {
    "amp-model:9f2c…": { "favorite": true, "tags": ["djent", "rhythm"], "last_used": "2026-09-30T14:02:11Z", "use_count": 12 }
  } }
```

The key is `<kind>:<id>`, so presets can live in the same file
(`plugin-preset:<clap_id>:<preset id>`). §10 covers how this is shared.
`generation` is bumped on every write; both libraries' freshness checks
read it (plugin-preset-library.md §10.2 item 2). The store lives at
`$XDG_DATA_HOME/resonance/library/marks.json`, overridable with
`RESONANCE_LIBRARY_DIR`. An item whose fields are all at their defaults is
deleted rather than stored, and the schema reserves `rating: u8?` for later.

The same module carries the **seeded facet vocabulary**
(`library_marks::vocab`, plugin-preset-library.md §4.4): `instrument`,
`genres` and `character`. The Library panel offers `instrument`
(`electric-guitar`, `bass`, …) and `character` values beside its own
NAM-only `gear_type` / `tone_type` facets, and `+ tag` completion suggests
the seeded values as well as every tag already used across kinds.
Recents are derived from `last_used`, written when the user picks a model:
a Library row, ◀/▶, an import, a Tone3000 download or a relink from the
missing banner (every editor action that points `file_select` at a slot).
It is not written by project-open restores, so opening an old project does
not reorder recents, nor by a host or automation moving `file_select` (the
plugin cannot tell those from a restore). The marks prune pass (§7.2) runs
after every rescan.

## 5. Selection, state and the missing model

### 5.1 `file_select` becomes a stable slot

`file_list` stops being a directory listing. It becomes the library's **slot
table**: `slots[n] = Some(path) | None`, shared by every amp instance.

- A newly installed model takes the slot after the high-water mark. Deleting
  a model frees its slot, but the slot is **not reused while any other slot is
  free** above the high-water mark. In practice slots behave as append-only
  until 1000 models, and only then are freed ones reused. So a preset or an
  automation lane recalls the same model after other models are added or
  removed. A model whose bytes come back (a re-download or re-import of the
  same content) gets its old slot back while that slot is still free.
- Every distinct file gets a slot, including one whose header does not
  parse, because the old directory listing counted it too; a load of it
  fails with the parse reason. A byte-identical duplicate gets none: its
  canonical copy (downloads first, then imports, then path order) holds it.
- Migration: the first build of the index assigns slots to the existing
  `tone3000/` files in today's sorted order. So `file_select` values that
  point into the downloads directory keep their meaning.
- `file_select` pointing at an empty slot loads nothing, and the loader
  does not clamp to the last entry (`loader.rs:98` before). It also
  **unloads nothing**: whatever was playing keeps playing (unloading would
  mean dropping a model on the audio thread), and the status reads
  "empty slot N". A missing *reference* is the §5.2 step 3 state.
- ◀/▶ step through the **Library panel's current view** (search + filters +
  sort, favourites first). They do not step through slot order, so "next" is
  what the user sees next. They write the slot number.
- A model loaded from outside the library (legacy state, or Link mode if §13
  D3 goes the other way) is **external**. It plays normally, `file_select` is
  parked at its current value, and the header offers "Add to library".

### 5.2 Plugin state v2

`AmpExtraState` saves:

```json
{ "model_path": "/…/tone3000/Friedman_BE100_(standard)_48121.nam",
  "model_id": "9f2c…",
  "model_name": "Friedman BE-100 · standard",
  "model_source": { "tone3000": { "tone_id": 1934, "model_id": 48121 } } }
```

`model_path` stays, so old projects load unchanged. The new keys are optional
on load (`project_no_real_users_yet`: no migration machinery).

**Resolution at `initialize` / `load_state`** is one pure function,
`resolve_model(state, &library) -> Resolved`, which makes it unit-testable:

1. `model_path` exists and its hash matches `model_id` (or there is no
   `model_id`) → load it.
2. The path is missing (or the hash differs) and the library has an entry
   with `model_id` → load that path, **rewrite `model_path`**, and show a
   one-line notice "Relinked: <name> (file had moved)". This is silent
   auto-relink.
3. Otherwise → `Resolved::Missing { name, path, source, file_changed }`. No
   model is loaded, the whole reference is **kept verbatim** (an unparsable
   `model_source` included), so a save does not lose it, and the editor shows
   the missing banner (§6.4). `file_changed` marks a path that exists but
   holds other bytes; the banner then also offers "Use the file at this
   path".

The reference is written by the loader **after** a load succeeds, so what
`save_state` persists is always what plays. `initialize` brings the shared
index up to date first: a full scan once per process, then only a `stat`
of `library.json`. The index's cached id is used for a file whose size and
mtime are unchanged; only an unknown file is hashed.

### 5.3 What "missing" sounds like

This is unchanged: the processor passes dry × gain. The spec does **not** mute
the output. A muted track in a mix is worse than a clean one, and the user
may be fixing it while playback runs. What changes is that the state is
visible: the header name turns `WARN` and reads "Missing: <name>", and the
scope/curve area shows the banner. MCP sees it too (§9.1).

## 6. UI

### 6.1 Header

`Load Model…` and `Browse Tone3000…` are replaced by one `Library…` button,
which uses the solid-accent style that is `Browse Tone3000…` today
(`header.rs:41-47`). The model name becomes a clickable label that also opens
the Library. A ☆/★ `chip_button` next to the name toggles favourite for the
loaded model. ◀/▶ and the counter stay. The counter now reads "3 / 41 in
view", because it counts the panel's view.

```
│ RESONANCE AMP │ [Library…] │ — preset — ◀ ▶ Save … │ ◀ ▶  ★ Friedman BE-100 · standard   3 / 41   ⚠ 44.1 kHz / 48 kHz │
```

### 6.2 Library panel

This is a full-window overlay, the same mechanism as the Tone3000 panel
(`egui::Area`, dimmed backdrop, `theme::PANEL` frame), with two tabs,
`Installed | Tone3000`, drawn by the shared `segmented` widget. The existing
Tone3000 browser becomes the second tab with no change to its logic. Opening
the Library lands on **Installed**. If the library is empty it lands on
**Tone3000**, or on the empty state with both entry points.

```
┌ LIBRARY  [ Installed | Tone3000 ]                                   41 models · 612 MB   [Close] ┐
│ [search name, author, gear, tag…_____________]  (★ only) (Recent)  Gear ▾  Type ▾  Arch ▾  Sort ▾ │
│ ──────────────────────────────────────────────────────────────────────────────────────────────── │
│ ★ Friedman BE-100 · standard        J. Smith     Friedman BE-100    amp  crunch   A2  48k  4.1 MB │ ← loaded (ACCENT border)
│ ★ Darkglass MT900 · clean           Steve        Darkglass MT900    amp  clean    A1  48k  3.2 MB │
│ ☆ 5150 Block Letter · feather       tonekid      Peavey 5150        amp  hi_gain  A2  44k⚠ 0.4 MB │
│ ☆ my_rig_capture                    —            —                  —    —        A1  48k  2.9 MB │ imported
│ ✕ Old Marshall (unreadable)         parse error: unsupported head "windowed"                    │ DANGER text
│ …                                                                                                │
│ ──────────────────────────────────────────────────────────────────────────────────────────────── │
│ Friedman BE-100 · standard                                                                       │
│ by J. Smith · Tone3000 tone #1934 · A2 WaveNet · 48 kHz · ESR 0.0041 · added 2026-09-12          │
│ tags: (rhythm ×) (djent ×) [+ tag]         used in 2 open amps                                   │
│ [Load]  [Reveal]  [Re-download]  [Delete…]                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
│ [Import .nam…]  [Rescan]                                               status / last error line   │
```

- **Rows**: one `Frame` per row with the whole row as a click target. This is
  the pattern in `draw_tone_row` (`tone3000_panel.rs:405-462`). A click
  selects the row and shows its detail. A double-click, or `Load` in the
  detail, loads the model. The ☆ at the left of the row is its own click target
  and does not select the row. The rows sit in an `egui::ScrollArea` using
  `show_rows`, so 1000 models cost one screen of layout
  (`feedback_view_performance`).
- **Search**: a case-insensitive substring over name, author, gear, tags and
  file name, in a `TextEdit::singleline` with a hint. The filter runs as you
  type, over the in-memory index.
- **Filters**: `★ only` and `Recent` are `chip_button` toggles. Gear, Type and
  Arch are `ComboBox`es fed from the values present in the index, not from a
  fixed list. Sort offers Name / Recently used / Recently added / Author /
  Size. **Favourites always sort first** within any sort. The filter and
  sort state is editor-only runtime UI state and is not persisted. This
  follows the rule ux-guidelines.md:174 states for collapse state.
- **Detail pane actions**:
  - **Load**: sets `file_select` to the slot. This goes through the param, so
    it is automatable, undoable in the host, and visible to MCP.
  - **Reveal**: opens the containing folder, and selects the file where the
    platform supports it (`open -R` on macOS, the FileManager1 D-Bus
    `ShowItems` with an `xdg-open <dir>` fallback on Linux). The app's
    `reveal_path_in_file_manager` moves into `resonance-common` so the app
    and the plugins share one launcher.
  - **Re-download**: enabled only for `source = tone3000`. It re-lists the
    tone (`/models?tone_id=`), finds `model_id`, and downloads to the same
    file name. It is offered in two places: on a row whose file failed to
    parse, and on the missing banner.
  - **Delete…**: see §7.
  - **Tags**: removable chips, plus a `+ tag` inline `TextEdit`. Completion
    offers tags already used in `marks.json`.
- **Import .nam…**: an rfd multi-file dialog. Each file is header-checked
  (§4.2) and **copied** into `imported/`. A copy of an existing `id` is not
  duplicated; the existing row is selected with the notice "already in
  library". Invalid files are refused with the parse reason. With a single
  file, the import also loads it. This replaces `Load Model…`.
- **Tone3000 tab**: model rows gain an `Installed` label (`theme::ACCENT`,
  as in drums' download panel) when their `model_id` is already in the
  library, and `Download` becomes `Load`. After a download the panel switches
  to Installed with the new row selected. Downloads write the sidecar (§4.1).
- **Usage**: "used in N open amps" counts instances in this process (§8). It
  never claims to know about saved projects.

### 6.3 Empty state

```
No models installed yet.
[Browse Tone3000]   [Import .nam…]
Models live in ~/.local/share/resonance/amp-models  [Reveal]
```

### 6.4 Missing model

The banner sits over the scope/curve area, which has nothing real to draw with
no model. It uses a `WARN`-bordered `Frame`:

```
⚠ Missing model: "Friedman BE-100 · standard"
  was /home/…/tone3000/Friedman_BE100_(standard)_48121.nam — the amp is passing the signal through clean.
  [Re-download from Tone3000]   [Locate file…]   [Choose another model]
```

- **Re-download** appears only when the state carries a Tone3000 source. It
  needs a connected account, and a disconnected user gets `Connect…` first.
  After download, identity is checked by `model_id`. If the hash differs from
  the saved `model_id` (the author re-uploaded), the model loads with the
  notice "re-downloaded model differs from the one this project was saved
  with".
- **Locate file…** opens an rfd dialog. A picked file whose hash matches
  relinks silently. A mismatch asks "Use this file anyway?". Both go through
  import (copy into the library) unless the file is already inside the root.
- **Choose another model** opens the Library.

## 7. Deletion semantics

1. **Confirm in place.** `Delete…` turns into
   `Delete "<name>" (4.1 MB)? [Delete] [Cancel]`, in the drums
   two-click pattern (`download_panel.rs:247-290`). There is no modal
   window. When instances in this process are using the model, the line says
   so: `Used by 2 open amps — they keep playing until reloaded`.
2. **What is removed**: the `.nam`, its sidecar, and its slot, which is freed
   under the no-reuse rule of §5.1. **Marks are kept** for 90 days, keyed by
   content id (the shared store's one orphan policy, plugin-preset-library.md
   §4.5). So a delete followed by a re-download or re-import keeps the
   star and tags. A prune pass at index build drops marks older than that
   whose id is in no library entry.
3. **Instances already playing it** keep the model in memory. Deleting does
   not interrupt audio. Their header gains a `WARN` "(deleted)" suffix, and
   their next activation or reload resolves to Missing (§5.2 step 3), with
   Re-download offered if it was a Tone3000 model.
4. **Saved projects** are not scanned. A project that referenced the model
   hits the §6.4 banner on open. That is the whole contract, and the confirm
   line says it: "Projects that use it will show it as missing."
5. **Real delete, not trash.** Tone3000 models can be re-downloaded. Imported
   files are copies, so the user's original is untouched. See §13 D4.
6. The **file on disk is authoritative**: a file deleted in the file manager
   is dropped at the next refresh (§8), exactly as if it had been deleted in
   the panel.

## 8. Concurrency: many instances, one library

There are two scopes:

- **In one process** (Resonance hosts every amp in-process, from one loaded
  `.clap` image), a process-wide `static LIBRARY: OnceLock<Arc<Library>>` in
  the amp crate holds the index, the slot table and the marks behind one
  `RwLock`, plus a `generation: AtomicU64`. Every instance shares it:
  `file_list` becomes a view of it, and the "used in N open amps" count is a
  registry of live instance → id that each instance updates on load and drop.
  This also replaces the per-instance Tone3000 worker (G8) with one shared
  worker that is created lazily on first editor open. Each editor factory
  that opened it holds it, so it is joined when the last such plugin goes
  (a static that outlived the plugins would leave a thread running into an
  unloaded `.clap`). A download carries a callback from the requesting
  editor, which points that instance's `file_select` at the new slot.
- **Across processes** (a second host, or a second Resonance), there is no
  watcher dependency (the workspace has no `notify` crate, and a plugin
  should not add inotify threads). The design instead:
  - Writes are atomic whole-file replaces (`resonance_common::atomic_file::atomic_write`)
    of `library.json` and `marks.json`, each carrying a `generation` counter.
  - Marks writes are **read-modify-write of the single item under a lock**.
    The writer takes an exclusive `std::fs::File::lock` on
    `library/marks.lock`, reloads `marks.json`, applies its one change, bumps
    `generation` and atomic-replaces the file. So two processes starring
    different models both win, even inside the same few milliseconds, and
    the same item is last-writer-wins. The OS releases the lock when a
    process dies. It is never taken on the audio thread.
  - **Refresh** is poll-based and cheap. Every 500 ms while the Library
    panel is open, and at most once per 2 s from the header, the editor
    frame stats the root and its two subdirectories (mtime + entry count),
    `library.json` and `marks.json` (`library_marks::FreshnessPoll`, one
    helper shared with the preset library). A marks change re-reads
    `marks.json`; a library change starts a rescan on a helper thread
    (joined when the editor closes, so nothing outlives the plugin image),
    which hashes only files whose (size, mtime) are new. With no editor
    open, a refresh runs only at `initialize` and when a load is requested
    for a slot this process sees empty, or whose file has gone.
  - Slot allocation (the rescan that writes `library.json`) is taken under
    the same primitive: an exclusive `File::lock` on `library.lock` in the
    library root. It replaces the earlier `create_new` lockfile with a 10 s
    staleness rule, which could break a live lock under a slow disk. This is
    the one place where two processes racing would produce two models in one
    slot.
- **The audio thread** never touches any of this. `process()` still only
  compares `file_select` with its baseline and stores into `load_request`. The
  loader thread resolves slot → path through the shared library, taking a read
  lock off the audio thread. The `model_selector.rs` invariant (no load the
  user did not ask for) must keep holding.

## 9. Control API / MCP

Param-first where it fits (plugin-audit-plan.md §0 rule 1), vertical slices
for the rest (rule 2).

### 9.1 Free with the param (slice L2)

- `file_select.with_value_to_string(slot → name)` makes `*.plugin_params`
  report `text: "Friedman BE-100 · standard"`, `"(empty)"` or
  `"Missing: <name>"`. The closure reads the shared library with `try_read`
  and falls back to `"slot N"`. The host calls it on the main/engine thread,
  never on the audio thread.
- `file_select.with_string_to_value(name | id prefix → slot)`, for hosts and
  for 9.2.

### 9.2 Pick a model by name (slice L6, fleet-wide)

`ParamValue::Label` today resolves only against `choices`. The change: when a
parameter has **no** choices, the app asks the plugin through a new host call,
`PluginInstance::param_from_text(id, text)` (CLAP `text_to_value`, the mirror
of `param_text` at `instance.rs:475`), before it rejects the label. Then
`track_set_plugin_param {param: "Model Select", value: "Friedman BE-100"}`
works, and every stepped parameter in the fleet that has a `string_to_value`
gains the same thing. It is one generic slice with no amp-specific method.

As built: `ClapInstance::param_from_text` is the host call; the app reaches
it through `AudioCommand::ResolvePluginParamText`, answered on the engine
thread under the instance lock (re-enqueued, never blocking, while the
audio thread holds it), with `AudioEngine::param_from_text` waiting at
most 250 ms for the reply. The control reply stays synchronous, so the
one-revision-per-call contract holds. A text the plugin rejects (or no
answer in time) keeps the old "names no choices" error and says what the
plugin answered.

Both conversions also had to work on an **active** plugin: the CLAP
bridge answered `value_to_text` / `text_to_value` only while the plugin
object was on the main thread, and printed a bare number (and parsed
nothing) once it moved into the audio processor. `ResonancePlugin` gains
an optional `param_text_source()` (a `ParamTextSource` harvested at
construction, like `extra_state_saver`), which the bridge falls back to
while active; the amp returns one over its shared `AmpParams`. So §9.1's
`text` is the model name on a live instance too.

### 9.3 Library methods (slices L7a/L7b)

| Method | Params | Returns | Notes |
|---|---|---|---|
| `amp_models.list` | `query?`, `favorites_only?`, `gear_type?`, `tone_type?` | `[{slot, id, name, author, gear, gear_type, tone_type, architecture, sample_rate, size_bytes, source, favorite, tags, last_used, status, error?}]` + `library_generation` + `total` | Read-only, and answered above the mutation gate: no project needed, no undo entry, no `revision` bump. The app reads (and rescans) the library via `resonance_common::nam_library` and the shared marks store, the same code and files as the plugin, so it needs no running amp instance. `query` is the Library panel's own search (the shared `BrowserModel` over the same rows). Favourites first, then slot order. The agent then sets `Model Select` to `slot` (or to the name, per 9.2). |
| `amp_models.set_marks` | `id` (or a unique 8+ character prefix), `favorite?`, `tags?` (replaces the personal tags, normalised; `[]` clears) | the updated entry | Mutates per-user state, not the project: no undo entry, and it does not bump the project `revision`. The description says so. At least one of `favorite` / `tags`; an unknown id is `not_found`. Written through the shared store's lock, so it cannot lose a concurrent star from the amp's panel. |

**Not on MCP** in this spec: delete, import and download. Delete removes user
files that are outside the project and cannot be undone, and the agent loses
nothing without it. Download needs the plugin's OAuth session. §13 D6 records
the decision.

`amp_models.*` is a new namespace. Protocol evolution is additive within a major
version (`resonance-control/src/lib.rs:22-24`), so adding the methods does not
bump `PROTOCOL_VERSION`.
The `mixing` skill in `resonance-agent-plugin` should gain one line on
picking an amp model by name. The lockstep test
(`resonance-mcp/tests/agent_plugin_lockstep.rs`) then checks that the named
tools exist.

## 10. Relation to the preset library

`plugin-preset-library.md` is being written at the same time for a shared,
reusable plugin-preset library (tags, favourites, genre, a browser component).
It did not exist when this spec was finished, so the alignment below is a
proposal to reconcile, not a settled decision. What should be **one thing**
across the two:

| Shared | Owner | Used here as |
|---|---|---|
| **Marks store** (`$XDG_DATA_HOME/resonance/library/marks.json`, `<kind>:<id>` keys, favourite / tags / last_used / use_count, reserved `rating`, per-item read-modify-write under `File::lock`, generation counter, orphan pruning, freshness helper) | `resonance-common` (a `library_marks` module; plugin-safe, in `PLUGIN_COMMON_ITEMS`) | §4.3. Kind `amp-model`, id = content sha256. |
| **Tag vocabulary** (completion reads all tags across kinds) and the **seeded facet vocabulary** (`instrument`, `genres`, `character`) | same module (`library_marks::vocab`) | the `+ tag` completion, and `instrument` / `character` facets beside `gear_type` / `tone_type` |
| **Browser view-state**: search, facets, favourites-first sort, ◀/▶ over the current view, the audition bracket and confirm-in-place delete state, over a `LibraryRows` trait of `{title, subtitle, columns, key, marks}`, testable without egui | `resonance-plugin::library_view`, **not** feature-gated, because the iced app drives the same model (plugin-preset-library.md §10.2 item 3) | §6.2's Installed tab and the ◀/▶ stepping over the view |
| **Browser list widget**: the egui skin over that model, with a ★ toggle, tag chips and a detail pane frame; `star_toggle` / `tag_pill` in `plugin_gui_core::widgets` | `resonance-plugin` behind `editor-widgets` | §6.2's rows |
| **Delete-confirm row** (two-click in place) | state in `library_view`, skin beside the list widget | §7.1, and the preset bar's unconfirmed `Delete` should adopt it |
| **Reveal launcher** | `resonance-common` | §6.2 Reveal |

What stays amp-specific: the NAM header reader, the slot table, content
hashing, the Tone3000 source, and relink. The preset spec adopted this
spec's path and key scheme for marks as-is; ids are opaque strings, so a
sha256 fits. Its §10.2 asked for five changes, all applied here: the lock
(§8), `generation` in the schema (§4.3), the ungated `library_view`
view-state (this table), the seeded vocabulary in `library_marks` (§4.3),
and the shared `library_marks.rs` test binary (§11).

## 11. Tests

Plugin crates are not under the group-binary rule. Even so, add **one** new
binary to the amp crate and extend existing ones rather than adding one per
slice. No inline `#[cfg(test)]`.

| Where | What |
|---|---|
| `resonance-common/tests/nam_library.rs` (new; the crate already has `tests/atomic_file.rs`) | `read_header` on the three fixture families (a1, a2, lstm), which reads metadata without allocating weights. Index build, then rescan: only changed files are re-hashed. Slot allocation: append, free, no reuse below the high-water mark, reuse only past 999. Migration assigns today's sorted order. Duplicate detection by id. A corrupt `library.json` is quarantined and rebuilt (`quarantine_corrupt`). Each test uses its own temp root through the explicit-root API, not env vars. |
| `resonance-common/tests/library_marks.rs` (new; shared with the preset spec and owned by whichever slice lands first) | Per-item read-modify-write under the lock: two writers with different items both survive, including two processes (the test binary re-spawns itself as a child to hold the lock), and the same item is last-writer-wins. `generation` bumps on every write. Kinds do not collide. Prune after deletion with a clock injected. Vocabulary and tag completion across kinds. |
| `resonance-plugin/tests/library_view.rs` (new; shared with the preset spec) | `BrowserModel` over a fake `LibraryRows`: search, facets, favourites-first sort, ◀/▶ over the view (clamped), the audition bracket, confirm-in-place. |
| `plugins/resonance-amp/tests/model_library.rs` (new) | `resolve_model`: path ok; moved and found by id (rewrites path); missing with the reference kept verbatim; hash mismatch. Import copies and dedupes. Delete removes the file, sidecar and slot, keeps marks, and lets live instances keep playing. Sidecar written by `finalize_download`, with `sanitize_filename` unchanged. View-state logic: search, filters, favourites-first sort, ◀/▶ over the view. `file_select` `value_to_string` / `string_to_value` round-trip. |
| `plugins/resonance-amp/tests/model_selector.rs` (extend) | The baseline invariant still holds with slots. An empty slot never loads. Adding a model to the library while an instance is active does not move its model. |
| `plugins/resonance-amp/tests/state.rs` (extend) | v2 keys round-trip. A v1 state (only `model_path`) loads. A missing model re-saves byte-identical extra state. |
| `plugins/resonance-amp/tests/tone3000_browser.rs` (extend) | The "Installed" / `Load` row decision given a library. Re-download finds the model in a re-listed tone (pure function over `Vec<Model>`). |
| `resonance-app/tests/control` group (new module, not a new file) | `amp_models.list` / `set_marks` against a temp library root. `set_plugin_param` with a label on a choice-less param resolves through `param_from_text` (built plugin binary, as the `clap_host` tests do). |
| `resonance-mcp/tests` (existing binaries) | Tool registration and schema hygiene pick up the new tools. Lockstep, if the skill names them. |
| `tools/arch-invariants` | Not a new test, but it fails until `nam_library`, `library_marks` and `reveal` are added to `PLUGIN_COMMON_ITEMS`. That is intended. |

GUI: the plugin editors have no golden-image harness. As with
`PresetEditor`, the panel's behaviour lives in a GUI-agnostic state struct
that the tests above drive, and the egui file only turns clicks into its
methods. Check by hand from a Wayland session with the two `editor_open` /
`editor_size` ignored tests (CLAUDE.md) after the panel lands, because it
changes the editor's first frame.

## 12. Build plan (vertical slices)

Each slice is shippable on its own and leaves the plugin better than before.

| Slice | Delivers | Depends on |
|---|---|---|
| **L0: header reader + index** | `resonance-common::nam_library`: `read_header`, scan, hash cache, `library.json`, `RESONANCE_AMP_MODEL_DIR`, arch-invariants row. No UI change yet. | — |
| **L1: provenance at download** | `finalize_download` writes the sidecar. The Tone3000 tab shows `Installed` / `Load` for models already present. | L0 |
| **L2: slots + state v2 + missing** | `file_list` → shared slot table (process `OnceLock`). `AmpExtraState` v2, `resolve_model` with auto-relink. The missing banner with Locate / Choose. `file_select` `value_to_string`/`string_to_value` (MCP now *reads* the model name). One shared Tone3000 worker. | L0 |
| **L3: Library panel (browse)** | `Installed` tab: rows, search, filters, sort, detail, Load, Reveal (with the launcher moved to common). The header `Library…` button. ◀/▶ over the view. Import replaces `Load Model…`. | L2 |
| **L4: marks** | `library_marks` in common, built to plugin-preset-library.md §10.2's shape (the shared foundation slice F builds it first, together with `library_view`, the list widget and the reveal launcher). ★ in rows and header, tags, recents, favourites-first. | L3; F |
| **L5: delete + re-download** | Confirm-in-place delete, in-use count, the "(deleted)" suffix. Re-download from the detail pane and the missing banner. Poll-based cross-process refresh. | L3 (L1 for re-download) |
| **L6: labels via `text_to_value`** | `param_from_text` host call and the `ParamValue::Label` fallback. The fleet gets it; MCP can now *pick* the amp model by name. | L2 |
| **L7a: `amp_models.list`** | Wire type + app handler + MCP tool + test. | L0 (L4 for the marks fields) |
| **L7b: `amp_models.set_marks`** | Wire type + app handler + MCP tool + test. The skill line. | L4, L7a |

L2 and L3 are the user-visible core. L5 is the user's headline ask (delete),
and it cannot come earlier: deleting before L2's slots would shift every
preset and automation index (G3).

## 13. Open decisions (each with a recommendation)

| # | Question | Options | Recommendation |
|---|---|---|---|
| D1 | Identity for favourites/relink | path · Tone3000 model id · content sha256 | **sha256.** It covers imported files, moves and duplicates. Tone3000 ids are kept in the sidecar as the re-download key, not as identity. |
| D2 | `file_select` semantics | keep sorted-listing index · stable slots · replace with a hidden param + path-only | **Stable slots** (§5.1). It is the only option that keeps amp presets (params-only) and automation lanes correct across adds and deletes, and the migration preserves today's indices. |
| D3 | Import: copy or link | copy into `imported/` · reference in place · "watched folders" | **Copy**, which makes delete safe and self-contained. Reconsider "watched folder" (reference a user's own NAM folder of hundreds of files) as a later slice if the user keeps such a folder. **Question for the user: do you keep NAM files outside Resonance that you want browsed in place?** |
| D4 | Delete: permanent or trash | permanent · OS trash · library `.trash/` with undo | **Permanent with confirm-in-place.** Every deletable model can be re-downloaded or is a copy. An OS-trash dependency per platform is not worth it for this. |
| D5 | Missing model: pass through or mute | dry passthrough (today) · mute · configurable | **Keep passthrough,** and make it loud visually (§5.3). A silent track mid-session hides the problem differently rather than fixing it. |
| D6 | Library writes on MCP | list only · + marks · + delete/import/download | **list + set_marks.** Delete stays GUI-only (irreversible user files outside the project). Download needs the plugin's OAuth session. Revisit if an agent workflow needs it. |
| D7 | Where the Library lives in the editor | overlay (like Tone3000) · a side drawer beside the viz · a separate editor page | **Overlay with two tabs.** It reuses the existing mechanism, fits 760×520, and folds the two model entry points into one. A persistent drawer would squeeze the scope at minimum size. |
| D8 | `installed.json` `AmpModel` variant | start writing it · leave it · remove it | **Remove the unused variant** in L0. `library.json` supersedes it for amps, and two registries that disagree are worse than one. Drums keeps the registry. |
| D9 | Marks store location/format | shared `library/marks.json` (§10) · per-plugin files · inside app `settings.json` | **Shared file in `resonance-common`,** because plugins cannot reach app settings and the preset library needs the same thing. Path and schema are agreed with plugin-preset-library.md (§4.3 here, §4.5 there). |
| D10 | Do recents come from project restores? | yes · only user picks | **Only user picks** (and loads with an editor open), so opening an old project does not reshuffle Recent. |
