# Drums plugin rework: kit library, plok.org downloads, editor, sampler

Status: **implemented on `feat/drums-rework`** (2026-10-02; branched from
master @ 83bc1321, merged master @ 60de2598). Slices K0–K9 are built,
including K6b (disk streaming; default preload 32k frames — 64k measured
1,074 MiB for the default Drummica setup) and K7b (mic banks; 128 voices +
32 tail slots). K10 is deferred. D1–D10 are decided (§11); D3 is done — the
`registry` module is deleted. Where the build differs from this spec, §12
("As built") says so. D2 (copy on import), D8 (disk streaming now) and D9
(bleed/room/overhead banks now) override the first draft's recommendations.
Touches `plugins/resonance-drums` (download, kit loader, sampler, params,
state, editor), `resonance-common` (a new `drumkit_library`, retiring the drums
use of `registry`), `resonance-plugin` (a `kit_rows` adapter beside
`nam_rows`), `tools/arch-invariants` (whitelist row), and, in the last slice,
`resonance-control` / `resonance-app` / `resonance-mcp` and the
`resonance-studio` skill.

The sibling specs are `nam-model-library.md` and `plugin-preset-library.md`.
This one reuses their foundation: `library_marks`, `BrowserModel`,
`library_ui`, `reveal`. It does not invent a parallel one.
`plugin-preset-library.md` §10.3 already names drum kits as the next kind
(`kind = drumkit`).

## 0. Why

The user said: "The ui is terrible, we cannot download the kits from plok.org
anymore (no ui). The ui doesn't make sense at all. We need the same
tags/favorite etc for managing drum kits as for nam models. and be able to
delete, view etc. But also the plugin itself is not good..."

All of that is accurate. In short:

- **The download UI still exists, but you cannot see it.** It sits behind a
  ghost button labelled `Browse`. Its dimming backdrop is painted on a layer
  *above* the panel, so opening it gives a near-black screen (§1.2).
- **The editor does not fit its own window.** The body needs about 640 px of
  height and gets 302 px. The global controls (master, polyphony, velocity
  curve, round robin) are below the window edge and nothing scrolls (§1.3).
- **Kits cannot be managed.** You cannot favourite, tag, search, inspect,
  reveal or (outside the hidden overlay) delete a kit. Kits copied in by hand
  are invisible. The kit stepper is stuck on the first kit because of a name
  mismatch (§1.4).
- **The sampler is a Drummica player rather than a drum sampler.** It has no
  tune, decay or sample start. Volume is linear. A stolen voice clicks. It
  holds about 3.8 GB of RAM per instance, and every mic change re-decodes the
  whole kit. A newer kit can be dropped silently. The 30 pads are hardcoded to
  Drummica piece names, even though kits ship their own names and labels
  (§1.5).

## 1. What exists today

### 1.1 Download (`src/download.rs`)

| Fact | Where |
|---|---|
| Index is `https://resonance.plok.org/index.json`, `{drumkits:[{name,file,size,description,tags,added}]}`. It is live today and lists one kit: Drummica, 5.3 GiB. | `download.rs:25,39-59` |
| Downloads stream in 256 KiB chunks to `drumkits/.<name>.zip.part`, then unzip into `drumkits/<name>/` and call `registry::mark_installed(Drumkit)` | `download.rs:244-321` |
| One `drums-download` thread **per plugin instance** | `lib.rs:273` |
| No cancel, no read timeout, no checksum, no disk-space check. A failed download leaves the `.part` file behind. | `download.rs` |
| `WorkerHandle::drop` joins the thread, and Shutdown is only read between commands. Closing a project during a 5.3 GiB download **blocks until the download finishes**. | `download.rs:136-143` |
| `did_initial_fetch` is never reset. The index is fetched once per editor lifetime. | `download_panel.rs` |
| A finished download is not loaded and not selected | — |

### 1.2 Why "there is no download UI"

- `ecd063e9` (2026-05-24, "Polish drum plugin editor") replaced the
  `Download Kits` button with a small ghost `Browse` on the pad-list kit card
  (`pad_grid.rs:170`). The doc comment at `download_panel.rs:3` still says
  "Download Kits".
- `3a4eafab` (2026-04-12) moved the backdrop to `egui::Order::Tooltip` "to sit
  between the UI and the Foreground panel" (`download_panel.rs:30-39`). In
  egui, `Tooltip` is drawn **above** `Foreground`, so the 70% black fill
  covers the panel. The amp's library panel does this correctly: it paints the
  backdrop with the parent `ui.painter()` (`resonance-amp/src/editor/library_panel.rs:67-74`).
- The overlay is not modal. Clicks fall through to the pads, and Esc does not
  close it.
- No test reads `pad_grid.rs` or `download_panel.rs`. `editor_honesty.rs`
  covers only `app.rs`, `chrome.rs` and `pad_inspector.rs`.

### 1.3 Editor

- The window is 720×440 (min 560×360, `factory.rs:20`). Chrome, tab bar and
  status bar take 114 px. The top row is `available_height − 200` tall
  (`app.rs:184,199`), which is **102 px** at the default size and 22 px at the
  minimum. The inspector is 430–516 px tall and does not scroll. The KIT and
  GLOBAL cards are off-screen.
- The width does not fit either. The tab bar's contents need about 806 px.
  `pad_grid.rs:56` forces the left column to 320 px even when it is given 292.
- Several controls are fake:
  - The traffic-light dots and the `? A ⚙` labels draw but do nothing (`chrome.rs:24-29,57-63`).
  - There is a single `Pads` tab whose click is discarded (`chrome.rs:84`).
  - The `DRUMS` label and the "N lit" badge are decoration.
  - A pad with fewer than two mics shows a dead "—" Balance knob.
- The same thing appears in two or three places:
  - OH blend is both a knob and the OH AMOUNT slider.
  - Balance is both a knob and the close-mic slider.
  - Mute is both the row's `M` and the Enabled/Muted chip.
  - The kit name is shown three times.
  - The round-robin position is shown three times.
- Several labels mislead:
  - `Browse` opens a download screen.
  - `Load kit` opens a raw JSON picker, and opens it synchronously on the UI thread (`kit_browser.rs:72`).
  - "OVERHEAD BLEND" puts a **global** setup picker in a **per-pad** card.
  - Mic setups are shown as raw keys such as `01_KickIn_e901`, although the
    manifest carries `brand`/`mic`/`channel` (parsed and unused).
- The SAMPLE stage always shows the loudest layer's first take. It never shows
  what was actually played. There is no live velocity display.

### 1.4 Kits as content

- Kits are discovered **only** through `installed.json`
  (`registry::list_installed(Drumkit)`, `kit_browser.rs:125`). The editor
  re-polls it every 60 frames. Folders under `drumkits/` are never scanned.
- The loaded kit's name is the **manifest's parent directory**
  (`kit_loader/mod.rs:248`), while the registry uses the top directory. On this
  machine kits are nested as `Drummica/drummica/drum_samples.json` and
  `IT Techno/ittechno/…`. So `chrome.rs:121` never finds the current kit:
  - the combo never highlights the loaded kit;
  - the pill shows `drummica`;
  - ◀/▶ always reload the first kit.
- Kits carry metadata in `_meta`, and the loader throws it away
  (`kit_loader/mod.rs:563`):
  - `_meta.pieces.<piece>.name` gives display names. In IT Techno,
    "SD Count Stick" is "Perc Conga" and "SD Snare Handtuch" is "Clap".
  - `_meta.articulations[{primary,alt,label}]` gives the toggle labels, such
    as "punch/deep" and "snap/body". The plugin hardcodes "mit/ohne Teppich"
    instead (`articulation.rs:42`).
- Installed today: Drummica (8.5 GB on disk, 35 pieces) and IT Techno (5 MB,
  19 pieces).
- The only content delete in the plugin is the two-click `Delete` →
  `Confirm?` inside the hidden overlay (`download_panel.rs:250-292`).

### 1.5 Sampler

**Format**
- The only format is a Drummica-style `drum_samples.json`:
  `{piece:{setup_key:{brand,channel,mic,position,rounds:{RRnn:{VelNN:file}}}}}`
  (`manifest.rs:12-23`).

**Pads**
- There are 30 fixed pads, mapped from hardcoded Drummica piece names
  (`kit_loader/mod.rs:42-108`).
- A piece the kit lacks is filled with a built-in 12-WAV kit, which is mono,
  about 11 LU louder, and clips at the default levels.

**Mics**
- Each pad has up to 2 close mics plus 1 overhead bank.
- There is one global overhead setup.
- Bleed setups, room mics and per-mic level are not supported.

**Outputs**
- There are always 7 stereo ports.
- Each pad's port is hardcoded.
- Main carries only Count Stick, so a host that reads port 0 hears almost
  nothing.
- Cymbals' overhead takes go to the Cymbals port, not the Overhead port
  (`sampler.rs:392-408`).

**Voices**
- 64 voices, one per bank per hit, so up to 3 per hit. (Since E1/E15: 128
  voices plus 32 tail slots, and up to 8 voices per hit with every bank on.)
- **A stolen voice is overwritten in place with no fade, which clicks**
  (`sampler.rs:421-436`).
- The release is a linear fade of exactly 1024 samples (`voice.rs:6`), so its
  length depends on the sample rate.
- Choke groups work in the engine but only hi-hats are configured
  (`drum_map.rs:18`).

**Velocity**
- The layer is picked from equal-width buckets after a global power curve.
- Gain within a multi-layer pad is a flat 1.0, so dynamics step between
  layers.
- The layer and take indexes come from bank 0. A bank with fewer layers is
  **silently dropped** (`sampler.rs:587-595`).

**Playback**
- Playback steps one frame at a time, so there is no pitch and no
  interpolation path (`sampler.rs:669-683`).

**Params**
- 4 globals + 30 × 6 per-pad params.
- Volume and master are linear 0..1, and the 0.8 × 0.8 defaults give 0.64.
- Close-mic and overhead choices are plugin state only, not params.

**Loading**
- Everything is decoded fully to interleaved stereo f32. Mono mics are
  duplicated to stereo (`resonance-common/src/wav.rs:288`).
- Decoding is sequential on one thread.
- The default Drummica setup is 2,835 files, about **3.8 GB per instance**,
  and about 7.5 GB at peak during a swap. Nothing is shared between instances.

**Reloads**
- Any mic or articulation change re-decodes all 30 pads (`reload.rs`,
  `articulation.rs:116`).
- `initialize()` reloads on every activate, even at the same rate
  (`lib.rs:315`). The built-in kit plays meanwhile.

**Failure handling**
- One unreadable WAV fails the whole kit (`decode.rs:96`).
- **A newer kit can be dropped**: `kit_sender` is `bounded(1)` and the loader
  does `let _ = try_send(...)` (`lib.rs:221`, `kit_loader/mod.rs:261`). If an
  older kit is still waiting in the channel, the new one is discarded while
  `kit_path` and the status claim it loaded.

**State**
- `kit_path` is saved as an absolute path (`lib.rs:516`). The "Drummica Kit"
  track preset hardcodes `/home/jorrit/...`.
- A missing kit falls back to the built-in kit, and the only sign is a status
  string.

**Control surface**
- No kit can be listed, loaded or awaited over MCP. The agent skill applies a
  track preset and checks that correlation is below 1 to guess whether the
  kit has loaded.
- The app's kit picker always reads "Drummica · N pads"
  (`resonance-app/src/view/compose/drum_groups_manager/kit_picker.rs:20`).

The tests skip nothing (no `#[ignore]`). Two real-library tests skip
themselves when `RESONANCE_DRUMMICA_PATH` is unset.

## 2. Goals / non-goals

**Goals**

1. Downloading from plok.org works again: it is discoverable, can be
   cancelled, verified and resumed, and does not hang a closing project.
2. A **kit library** with the same management as NAM models:
   - favourites, tags and recents, in the shared `marks.json` under kind `drumkit`;
   - search and filters;
   - a detail view (pieces, mic setups, articulations, layers/RR, size, source, path);
   - reveal, delete with confirm, import, rescan.
3. An editor that fits its window. One control per parameter, no fake
   chrome, labels that say what happens.
4. A sampler that is decent as a sampler, not only as a Drummica player:
   - click-free stealing and choke;
   - tune, decay and sample start;
   - dB levels;
   - velocity that follows the recorded layers;
   - configurable choke and output;
   - a stereo mode that works on Main;
   - kit-supplied names.
5. Memory and load time that can live with 2–3 drum instances:
   - **disk streaming**: only a short head of each sample stays in RAM (D8, E14);
   - a shared cache of those heads;
   - mono kept mono;
   - parallel decode;
   - reloads that only touch what changed.
6. A fuller multi-mic kit: bleed mics, room mics, and more than one overhead
   setup layered at once, each with its own level (D9, E15).
7. Both surfaces (plugin-audit-plan.md §0). An agent can:
   - list kits and pick one by name;
   - see which kit is loaded;
   - wait for the load to finish.

**Non-goals**

- SFZ, Hydrogen or "folder of WAVs" kit formats. An import that builds a
  manifest from a folder is a later item (§10, K10).
- Hi-hat CC4 openness, and per-hit timing humanize. Only velocity humanize is
  in scope.
- Downloading over MCP, which matches NAM decision D6.
- Embedding kits in projects.

## 3. Kit library

### 3.1 On disk

The root is `data_dir()/resonance/drumkits/`, overridable with
`RESONANCE_DRUMKIT_DIR` for tests, as the amp does.

```
drumkits/
  Drummica/                     ← one kit = a directory with a manifest at depth 0 or 1
    drummica/drum_samples.json
    kit.meta.json               ← sidecar (new), next to the top kit directory
  IT Techno/ …
  .staging/                     ← in-flight downloads/extractions (§4)
  library.json                  ← index cache (new)
  library.lock
```

**Sidecar** `kit.meta.json`:
- `source`: `plok`, `imported` or `local`
- `index_name` and `index_file`
- `sha256`: of the zip, when the index gives one
- `description`
- `index_tags`
- `downloaded_at`
- `size_bytes`: measured once after extraction

A kit with no sidecar (copied in by hand) is `source = local`. Its size is
measured lazily on a background thread, because Drummica is 2,835 files.

### 3.2 Identity (D1)

`id = sha256(drum_samples.json bytes)`.

- **Why the manifest:** it is a few hundred KB, so hashing it is cheap. It
  stays the same through a move or rename, and it changes when the kit's
  contents change. Hashing 8.5 GB of samples to get an id is not an option.
- **Marks** are keyed `drumkit:<id>`, so favourites and tags follow a kit
  through a rename and survive delete-then-re-download. Orphans are kept for
  90 days (`library_marks::ORPHAN_RETENTION_SECS`).

### 3.3 Entry

| Field | Notes |
|---|---|
| `id` | §3.2 |
| `name` | First match of: sidecar `index_name`, `_meta.name` (a new optional field), the top directory name |
| `dir`, `manifest_path` | `manifest_path` is relative to the root when it is inside the root |
| `source`, `added_at`, `size_bytes` | From the sidecar, or measured |
| `pieces` | Count, plus the list of display names (from `_meta.pieces`, else the raw piece name) |
| `mic_setups` | Each setup key → position, brand, mic. Used for friendly labels: "Shure Beta 91 · Kick In" |
| `articulations` | From `_meta.articulations`, labels included |
| `layers_max`, `rr_max`, `sample_count` | From the manifest. The manifest is cheap to read; the WAVs are not opened. |
| `status` | `Ok`, `ManifestError(reason)`, or `MissingFiles(n)`. `MissingFiles` is found by a lazy existence check, not at scan. |

### 3.4 Module and layering

- **`resonance-common/src/drumkit_library.rs`** (plus `sidecar.rs`) contains
  `Library::{open_and_scan, entries, entry, by_id, find(name), rescan,
  import, delete, reload_if_changed}`. It is modelled on `nam_library.rs`.
  - `rescan` walks the root to depth 2, looking for `drum_samples.json`.
  - It needs a row in `PLUGIN_COMMON_ITEMS`
    (`tools/arch-invariants/tests/architecture.rs:463`).
- **`library_marks::kind::DRUMKIT = "drumkit"`.**
- **`resonance-plugin/src/kit_rows.rs`** is the `LibraryRows` adapter, a twin
  of `nam_rows.rs`.
  - Facets: `source`, `mics` (setup count bucket: 1 / 2–4 / 5+), `has
    articulations`.
  - Columns: pieces, mic setups, size, source.
  - It has no GUI gate, because the app and the control handler use it too.
- **Migration from `installed.json`.** On first scan, entries from
  `installed.json` of type `drumkit` become sidecars, carrying `installed_at`
  and `source = plok` when the name matches the index. Drums then stops
  calling `registry` (D3).
- **One library per process.** A `SharedKitLibrary` (an `OnceLock`, as in
  `resonance-amp/src/library.rs`) means N drum instances share one index, one
  `FreshnessPoll` and one download worker.

### 3.5 Deletion

- **Confirm in place**, in the shared `library_ui::confirm_delete_row`:
  `Delete "Drummica" (8.5 GB)? [Delete] [Cancel]`. When open instances are
  using the kit, it adds `Used by 2 open drum instances; they keep playing
  until reloaded`.
- **What is removed:** `remove_dir_all` of the kit directory, done on a
  background job. Marks are kept as orphans.
- **Instances that are playing the deleted kit** keep their decoded samples.
  After a reload they hit the missing-kit path (§5.3).

## 4. Downloads from plok.org

### 4.1 Worker

There is **one** process-wide worker, owned by `SharedKitLibrary` and not by
each instance. It accepts `FetchIndex`, `Download(index_entry)`, `Cancel(id)`
and `Shutdown`.

- **Cancel.** A cancel flag is checked every chunk. A cancelled download
  deletes its `.part` file. `Drop` sets the flag and **does not join while a
  transfer is in flight**: the thread is detached and exits at the next chunk.
  This fixes the hang on project close.
- **Timeouts.** Add a 30 s read timeout to the existing connect timeout.
- **Resume.** `Range: bytes=<part_len>-` resumes an existing `.part` file when
  the server answers 206. On any other answer it restarts.
- **Disk check.** Before starting, `statvfs` must show free space of at least
  `zip_size × 2.1`, because the zip and the extracted files coexist. The
  refusal message names the shortfall.
- **Verify.** When the index entry has `sha256`, the zip is checked against it
  before extraction.
- **Extract.** Unzip into `.staging/<id-or-name>/`. Write the sidecar, then
  `rename` it into place, so the library never shows a half-extracted kit.
  Delete the zip afterwards. Rescan, select the new row in the Installed tab,
  and offer `Load`.
- **Progress** is published as bytes / total, rate and ETA, and later as files
  extracted.

### 4.2 Index format (server side; plok.org is ours)

All new fields are optional. The client tolerates the current file.

```json
{ "drumkits": [ {
    "name": "Drummica", "file": "drummica.zip",
    "bytes": 5690000000, "sha256": "…",
    "pieces": 35, "mic_setups": 14,
    "description": "Acoustic studio kit, multi-mic, 7 velocity layers, 3 RR.",
    "tags": ["acoustic", "rock", "multi-mic"], "added": "2026-04-12",
    "manifest_sha256": "…"            ← = library id; lets the client show "Installed"/"Update"
} ] }
```

- `size` (the display string) is kept as a fallback when `bytes` is absent.
- IT Techno is installed here but is **not in the index**. Adding it is a
  server-side task, listed in K3.

## 5. Kit selection, state, missing kit

### 5.1 `kit_select`: a stable slot param (D4)

The amp's approach (`nam-model-library.md` §5.1) applies here as it is:

- an `IntParam` slot into the library's slot table;
- slots never reused, so a preset or automation lane never recalls a
  different kit;
- `value_to_text` gives the kit name;
- MCP gets pick-by-name for free through the fleet-wide `ParamValue::Label`
  path (NAM slice L6).

The difference from the amp is that the param is flagged **not automatable**,
because a kit swap is a multi-GB decode.

The editor's Library `Load` sets this param, so a kit change is host-undoable.

### 5.2 State v2

- Replace `kit_path` with `kit_ref {id, name, rel_path, abs_path}`.
- Resolution order: `id` in the library, then `rel_path` under the root, then
  `abs_path`.
- `rel_path` makes the "Drummica Kit" track preset portable. Re-save it once
  after K4.
- The `preset_keys` entry changes from `kit_path` to `kit_ref`.
- Loading a v1 state converts `kit_path` to a `kit_ref`. That is one function
  plus one test, nothing more (`project_no_real_users_yet`).

### 5.3 Missing kit

Today a missing kit silently falls back to the built-in kit. Instead, the
editor shows a `WARN` banner over the pad area:

```
⚠ Missing kit "Drummica" — playing the built-in kit.
  [Download from plok.org]   [Locate folder…]   [Choose another kit]
```

- `Download` is shown only when the kit's name or `manifest_sha256` matches
  the index.
- `Locate` relinks when the manifest hash matches, and otherwise asks "Use
  this kit anyway?".

### 5.4 Load progress (both surfaces)

A read-only, non-automatable param `kit_load_progress` (0..1) holds the
decode progress. It is 1.0 when the kit in `kit_select` is fully in place on
the audio thread.

- **Editor:** a progress line in the header.
- **Agents:** poll it through `track_plugin_params`. This replaces the
  skill's correlation hack.

## 6. Editor

### 6.1 Frame

- **Size:** 960×640 default, min 780×520. This matches the amp.
- **Removed:**
  - the traffic-light dots, `? A ⚙`, the `DRUMS` label and the "lit" badge;
  - the tab-bar hint;
  - the pad-list kit card and its `Browse` / `Load kit` buttons;
  - the three-way duplicate kit name.
- **Every region scrolls** where its content can exceed its share of the
  window. No region has a height computed by subtraction.

```
┌ RESONANCE DRUMS │ ★ Drummica ▾  [Library…] │ — preset — ◀ ▶ Save… │ ▓▓▓▓▓░░ loading 71% ┐
├ [ Pads | Mix | Setup ] ─────────────────────────────────────────────────────────────────┤
│   (tab body)                                                                             │
├──────────────────────────────────────────────────────────────────────────────────────────┤
│ 48.0 kHz · 30 pads · 2.1 GB (shared)            last hit: Snare v98 → layer 5/7 take 2/3 │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

**Header**
- The kit name is a dropdown of the library view (favourites first). The ☆/★
  next to it favourites the loaded kit.
- `Library…` (solid accent) opens the Library overlay (§6.5).
- Kit ◀/▶ step through the library view, so they follow search and favourite
  filters, as the amp's `step_in_view` does.
- The preset bar is visually separated and labelled "preset", because the
  kit and the preset are different things.

**Status bar**
- Only real readings: sample rate, pad count, decoded memory (marked "shared"
  when cache hits come from another instance), and the last hit's velocity →
  layer/take.
- The OUT meter stays, because it is real.

### 6.2 Pads tab

```
┌ pad grid (6×5, scroll if narrow) ─────────────┐┌ inspector (scrolls) ──────────────────────┐
│ ┌Kick──┐┌Snare─┐┌Rim───┐┌Side──┐┌Stick─┐┌Tom H┐││ Snare  · D2 (38) · Snare port   [▶ ▾v100] │
│ │ C1 36││ D1 38││ …    ││      ││      ││     │││ [Mute] [Solo]                             │
│ └──────┘└──────┘└──────┘└──────┘└──────┘└─────┘││ ~~~~ waveform of LAST played take ~~~~    │
│ ┌HH cl─┐┌HH op─┐ …                             ││ layer 5/7 · take 2/3 · v98               │
│  …                                             ││ (Vol dB) (Pan) (Tune st) (Decay) (Start)  │
│ dim cell = piece not in this kit (D7)          ││ ARTICULATION  [snap] [body]  ← kit labels │
└────────────────────────────────────────────────┘│ MICS   Top: [Shure SM57 · SN Top ▾] 0.0dB │
                                                  │        Btm: [AKG C451 · SN Btm ▾] −6.0dB │
                                                  │        OH trim  ─────○──── 0.0 dB         │
                                                  │ OUTPUT [Snare ▾]   CHOKE [none ▾]         │
                                                  └───────────────────────────────────────────┘
```

**Pad grid**
- Cells show the kit's display name (from `_meta`) and the note.
- A cell lights on each hit.
- Clicking a cell selects it and auditions it. The vertical click position
  sets the velocity, low to high.
- `Mute` and `Solo` are in the inspector only. Mute is no longer duplicated
  on the row.

**Inspector**
- One control per param. The Balance knob and the OH AMOUNT slider are gone.
  Balance becomes per-mic dB trims, shown only for mics that exist (no dead
  placeholder).
- The waveform shows the **last played** take, not a fixed one.
- Audition plays at a selectable velocity.
- Mic setup combos show `brand · mic · position` instead of raw keys.
- Changing a mic reloads **that pad only** (§7, E4).

### 6.3 Mix tab

The mixer strip view of what is now split across cards:

- one strip per output port, with a meter;
- per-pad Vol/Pan as a compact table;
- **global**: master (dB), polyphony, velocity curve, velocity humanize, round
  robin mode, output mode (Stereo / Multi, D5), release time.

### 6.4 Setup tab

Kit-wide configuration that today lives in the wrong place or nowhere:

- mic banks: overhead setups (up to 3 layered, each with a level, moved out
  of the per-pad card), room setups, bleed on/off and level (E15);
- streaming preload size and the underrun counter (E14);
- the pad → output assignment table;
- choke groups (pad → group 0–8);
- the note map, read-only in K5 and editable later;
- the kit facts shown in the library's detail pane.

### 6.5 Library overlay

The overlay uses the amp's mechanism: the backdrop is painted with the parent
painter and the panel is an `Area` at `Foreground`. It is **modal** (the
backdrop eats input) and closes on Esc.

It has two tabs, `Installed | plok.org`. It opens on Installed, or on
plok.org when nothing is installed.

```
┌ KIT LIBRARY  [ Installed | plok.org ]                       2 kits · 8.5 GB   [Close] ┐
│ [search name, tag, mic…______]  (★ only) (Recent)  Source ▾  Mics ▾  Sort ▾          │
│ ★ Drummica          35 pieces  14 setups  7 layers  3 RR   plok    8.5 GB   ← loaded │
│ ☆ IT Techno         19 pieces   1 setup   1 layer   1 RR   local   5.0 MB            │
│ ───────────────────────────────────────────────────────────────────────────────────── │
│ Drummica · plok.org · added 2026-04-12 · 2,835 samples · …/drumkits/Drummica          │
│ pieces: Kick, Kick (ohne Teppich), Snare, … (35)   articulations: —                   │
│ mics: Kick In (Shure Beta 91), Kick Out (AKG D112), SN Top (SM57), … OH AB (KM184) …   │
│ tags: (rock ×) (metal ×) [+ tag]                     used in 1 open drum instance      │
│ [Load]  [Preview ▶]  [Reveal]  [Re-download]  [Delete…]                                 │
│ [Import kit folder / .zip…]  [Rescan]                          status / last error    │
└───────────────────────────────────────────────────────────────────────────────────────┘
```

- **Shared pieces.** It is built from `BrowserModel` +
  `library_ui::{search_field, library_list, facet_menu, tag_row,
  confirm_delete_row}`, the same as the amp. Search syntax (`is:fav`, `tag:`,
  `by:`) comes for free.
- **`Preview ▶`** decodes just the kit's kick, snare and closed-hat at the
  top velocity, first take, and plays them as a short pattern through the
  `Audition` hook. It never loads a multi-GB kit to answer "what does this
  sound like".
- **`Import…`** accepts a folder or a `.zip`. A folder is **registered in
  root** (D2) on a background job, with progress, cancel and the same disk
  check as a download (§4.1). It copies into `.staging/` first and renames,
  so a half-copied kit is never listed. A zip goes through the
  same staging and extraction as a download.
- **The `plok.org` tab** shows one row per index entry: name, size,
  description, tags, added. On the right of the row:
  - `Download`, then a progress bar with `Cancel`;
  - `Installed`, when the `manifest_sha256` or name is present locally;
  - `Update`, when the name matches but the hash differs.
  - There is also a `Refresh` button. The index is re-fetched each time the
    tab opens and is cached for 10 minutes.

## 7. Sampler fixes

Each item names the test that proves it.

| # | Fix | Test |
|---|---|---|
| E1 | **Click-free steal.** The victim goes to a 3 ms fade in one of 32 extra "tail" slots, and the new hit takes a clean slot. | Max inter-sample delta at the steal point stays below a bound. 64-voice saturation pattern. |
| E2 | **Release and choke in ms** (default 25 ms hat choke, 5 ms reset/swap) with an equal-power curve, sample-rate independent. The second quick kit swap no longer hard-cuts. CLAP `reset()` stays an instant cut on purpose: bounce calls `reset_plugins` before rendering and a fade would leak into the export (`release_ms.rs::reset_is_immediate`). | The same fade length in ms at 44.1, 48 and 96 kHz. |
| E3 | **Latest-wins kit hand-off.** A one-slot mailbox. When the slot is occupied, the loader takes the stale kit back and sends it to the janitor. `kit_load_progress` reaches 1.0 only once the audio thread has taken the kit. | Two back-to-back loads: the second kit is the one that plays. |
| E4 | **Incremental reload.** A mic, overhead or articulation change decodes only the affected pads. `initialize` at an unchanged rate reuses the loaded kit. | A decode counter (test hook): changing one pad's mic decodes that pad's files only. |
| E5 | **Shared sample cache.** It is process-wide and holds `Arc<[f32]>` keyed by `(path, mtime, rate)`, with a weak-ref sweep. Mono files stay mono, and the voice reads mono into both channels. Decoding runs on a small pool (about cores/2). | Two instances that load Drummica share memory (the second instance's decode count is 0). Memory for the default setup drops by roughly the mono share. |
| E6 | **Partial-kit tolerance.** An unreadable WAV drops that take or layer and is counted, and the kit loads. The status shows "3 samples unreadable". | Fixture kit with one corrupt WAV. |
| E7 | **Velocity that follows the recording.** RMS is measured per layer at load. Layer choice uses the measured loudness. Gain within a layer is interpolated against the next layer, so there is no step at a boundary. Banks with fewer layers map by relative position instead of being dropped. Velocity humanize (global, ±0–20). | A velocity sweep is monotonic in output RMS, with no step larger than X dB at layer boundaries. Mismatched-bank fixture still sounds. |
| E8 | **Pitch, decay, start** per pad. `pad_N_tune` (±24 st, fine in cents) uses fractional playback with 4-point Hermite interpolation. Tune 0 stays bit-exact on the integer path, so goldens hold. `pad_N_decay` is an AHD with hold and decay in ms, and "off" = full sample. `pad_N_start` (0–100 ms). | Tune +12 gives an octave (FFT peak). Decay shortens the RMS tail. Golden unchanged at defaults. |
| E9 | **dB levels.** `pad_N_level` −inf..+6 dB, default 0 dB. Master (`master_level`) the same. Per-mic trims in dB (replacing `balance`). v1 states convert linear values to dB once. The dB params take new ids (v1 had `master_volume`, `pad_N_volume`), so a value the host re-sends by a v1 id after the state — a project's param overrides, an automation lane — names no param and is dropped instead of landing a linear value on a dB param. | State migration test, and a param display test. |
| E10 | **Kit-driven pads.** Piece names and articulation labels come from `_meta`. An optional `_meta.pads: {piece: {note, port, choke}}` overrides the Drummica table, which remains the fallback. Pads the kit lacks are **dimmed and silent**, not filled from the built-in kit (D7). The built-in kit plays only when no kit is selected. | IT Techno shows "Perc Conga" / "punch/deep". A kit without toms has silent, dimmed tom pads. |
| E11 | **Output.** `output_mode` is Stereo or Multi (D5). In Stereo everything sums to Main. In Multi, `pad_N_output` (a choice of the 7 ports) applies, and the overhead takes of close-miked pads go to the Overhead port. Pads with no close mic (the cymbals, whose overhead take *is* the sound) keep their overhead on their own port, so the Cymbals sub-track is not silent (ba #1232). | Stereo: port 0 RMS ≈ the full kit. Multi: the cymbals' OH lands on Overhead. |
| E12 | **Choke groups as params.** `pad_N_choke` (0 = none, 1–8), defaulting to the hats in group 1. | An open hat choked by a pedal hat, with a configured group on toms. |
| E14 | **Disk streaming (D8).** At load, only a **head** of each sample stays in RAM: the first 32 k frames by default, which covers about 0.7 s at 48 kHz (64 k measured 1,074 MiB for the default Drummica setup, over the 1 GB goal). These heads are what the E5 cache shares. A voice starts on its head, then reads the **tail** from a per-voice ring buffer that a disk-reader pool fills ahead of it. Files are memory-mapped or `pread`. Decode is WAV-only, so tails need no decoder state. Samples with no head/tail split (mono, short) are fully cached. Rules: the audio thread never blocks or allocates. An underrun outputs silence for that voice and bumps a counter shown in the status bar and readable as a param. The reader runs ahead by at least 4 host blocks plus the ring size. Choke, steal and release all work on whatever the ring holds. Preload size is a global setting (32 k / 64 k / 128 k frames, default 32 k). | Memory for the default Drummica setup drops below 1 GB, measured. A 64-voice saturation test over a cold page cache plays with zero underruns at 128-frame buffers. A streamed render is **bit-identical** to a fully-cached render of the same MIDI (golden). The reader thread is killed mid-render and the audio thread does not block. |
| E15 | **More mic banks (D9).** Each pad gains optional **bleed** banks: kit setups whose position belongs to another piece, such as SN Btm on toms and kick. It also gains **room** banks (positions `Room*`) and up to **3 overhead setups at once** (for example OH AB + OH XY + Room). The mic list comes from the kit. Each bank kind has a global level and on/off param (`bleed_level`, `room_level`, `oh_N_level`), and each pad has a per-mic trim. Bleed and room are routed to the Overhead/room port in Multi mode and to Main in Stereo. Banks share the hit's layer and take. Voices per hit rise from 3 to as many as 8, so the voice cap becomes 128 + 32 tail slots. Banks are off by default, and turning one on loads only that bank, incrementally (E4), streamed (E14). Every bank adds its heads to memory: on Drummica, three overhead setups plus bleed take an instance past 1 GB, against the default setup's < 1 GB (E14). | A fixture kit with bleed and room setups: turning bleed off removes exactly that bank's energy. Each OH setup's level scales its bank only. 128-voice saturation stays click-free (E1). |
| E13 | **Cleanup.** Remove `samples/clap.wav` and `cowbell.wav`, the stale Clap/Cowbell docs, the `#[allow(dead_code)]` fields that are now used, and the per-param `Box::leak`. Move the rfd dialog off the UI thread. | — |

**Goldens.** E1, E2, E7, E11 and E15 change sound on purpose. Each re-blesses
`tests/golden/dsp_golden.u32` in its own commit, and the commit message says
why. Per `feedback_silent_goldens_are_vacuous`, every scenario guards against
silence on its own.

## 8. Control API / MCP

The tools follow the one-method-per-tool rule, and each slice lands the wire
format, the handler and the tool together.

| Method / tool | Shape | Slice |
|---|---|---|
| `drum_kits.list` / `drum_kits_list` | `query, favorites_only, source, limit` → `[{id, name, pieces, mic_setups, layers, size, source, favorite, tags, loaded_in:[track]}]` | K9 |
| `drum_kits.set_marks` / `drum_kits_set_marks` | `id` (prefix ≥ 8 chars), `favorite`, `tags`. The amp's twin. | K9 |
| `track_set_plugin_param kit_select "Drummica"` | Free through the param and `ParamValue::Label` | K4 |
| `track_plugin_params` → `kit_load_progress` | Free through the param | K4 |

- **Not exposed:** delete, import, download. This matches the NAM spec's D6.
- **The app's kit picker** (`kit_picker.rs:20`, `groups.rs:395`) reads the
  real kit name and pad names from the instance, instead of the hardcoded
  "Drummica" and `default_kit_pads()`.
- **The `resonance-studio` skill** replaces "apply the Drummica Kit preset,
  check correlation < 1" with `drum_kits_list` → `kit_select` → poll
  `kit_load_progress`.
  - `agent_plugin_lockstep.rs` must still pass.
  - That skill currently lives in `~/.claude/skills`, not in
    `resonance-agent-plugin/`. Decide in K9 whether it moves.
  - **Decided (K9):** it stays user-level in `~/.claude/skills` for now. It
    is a per-session production workflow tied to this user's setup (GUIDE
    tracks, their instruments, their genre skills), not a per-craft skill,
    so it does not fit the agent plugin's rule; the craft parts the plugin
    needs (kit loading, `drum_kits_list`) live in the `drumming` skill and
    the tool descriptions, which the lockstep test does cover. Revisit if
    the workflow is generalised for other users.

## 9. Tests

The plugin crate keeps one binary per concern, as today. The XDG/env-isolated
library tests keep **one** `#[test]` per binary (the amp's `model_selector.rs`
precedent).

- **`resonance-common/tests/drumkit_library.rs`** covers:
  - scan at depth 0 and 1;
  - id stability across a rename;
  - sidecar round-trip;
  - `installed.json` migration;
  - import copies through `.staging/` (cancel leaves nothing behind);
  - delete;
  - `MissingFiles` detection.
- **`plugins/resonance-drums/tests/download.rs`** runs against an in-test
  `TcpListener` HTTP server serving a small zip. It covers:
  - progress;
  - cancel removing `.part`;
  - resume over 206;
  - sha mismatch refusal;
  - the disk-space refusal, via an injected free-space fn;
  - staging → rename;
  - **drop during a transfer returns in under 1 s**.
- **`kit_library.rs`** covers:
  - `kit_select` slots stable across add/delete;
  - `value_to_text` = kit name;
  - `kit_ref` resolution order;
  - v1 → v2 state;
  - the missing-kit path.
- **Editor**, with headless egui frames as in `resonance-plugin/tests/library_ui_render.rs`:
  - at **960×640 and at 780×520**, every global control's rect lies inside
    the window (this would have caught §1.3);
  - the overlay panel's layer is above its backdrop (this would have caught
    §1.2);
  - Esc closes the overlay;
  - every tab is drawn.
  - `editor_honesty.rs` is extended to all editor files and drops the
    `chrome.rs` exemption.
- **Engine:** the E-table tests above. Run the suite once with
  `RESONANCE_RENDER_THREADS=8` after E1–E5.

## 10. Build plan (vertical slices)

Each slice is mergeable on its own. K0 is first because it gives the user
downloads back the same day.

| Slice | Content | Size |
|---|---|---|
| **K0 — stop the bleeding** | Overlay backdrop on the parent painter, modal input, Esc. Rename `Browse` → `Download kits…` and move it to the header. Window 960×640 with scrolling body and inspector. Fix the kit name/◀▶ mismatch by naming from the top directory. E3 latest-wins hand-off. E1 click-free steal. Download drop does not join mid-transfer. | S |
| **K1 — library core** | `drumkit_library` + sidecar + id + `library.json` + `installed.json` migration; `kind::DRUMKIT`; `kit_rows`; `SharedKitLibrary`; arch-invariants row | M |
| **K2 — Installed tab** | Library overlay: list, search, facets, ★, tags, recents, detail pane, reveal, delete-confirm, import folder/zip, rescan | M |
| **K3 — plok.org tab** | Shared worker, cancel/resume/timeout/disk check/sha/staging, Installed/Update badges, auto-select after install. **Server:** add `bytes`, `sha256`, `manifest_sha256`, tags and descriptions to `index.json`, and publish IT Techno if wanted. | M |
| **K4 — selection & state** | `kit_select` slot param (non-automatable), `kit_ref` state v2, missing-kit banner, `kit_load_progress`. Re-save the "Drummica Kit" track preset. | M |
| **K5 — editor rebuild** | Header / Pads / Mix / Setup per §6. Remove fake chrome and duplicates. Friendly mic labels. Live last-hit display. Fit tests. | L |
| **K6 — engine: load path** | E4 incremental reload, E5 shared cache + mono + parallel decode, E6 partial kits, E2 ms release | M |
| **K6b — disk streaming** | E14: head/tail split, disk-reader pool, per-voice rings, underrun counter, preload setting, streamed-vs-cached bit-identity golden. Runs the suite with `RESONANCE_RENDER_THREADS=8`. Needs a live 10-minute session before merging, as the RT-multithreading work did. | L |
| **K7 — engine: playing** | E7 velocity, E8 tune/decay/start, E9 dB, E11 output mode, E12 choke params. Goldens re-blessed per item. | L |
| **K7b — mic banks** | E15 bleed, room and layered overheads, bank params, Setup-tab UI, 128-voice cap. Depends on K6b. | L |
| **K8 — kit-driven pads** | E10 `_meta` names, labels, pad map, dimmed missing pads | S |
| **K9 — agents & app** | `drum_kits.list` / `set_marks` tools, app kit picker reads the real kit, skill update | S |
| **K10 — later** | Kit `Preview ▶` (§6.5), folder-of-WAVs import with manifest generation, editable note map | — |

## 11. Decisions (answered 2026-10-01)

| # | Question | Decision |
|---|---|---|
| D1 | Kit identity | **Manifest hash** (sha256 of `drum_samples.json`). |
| D2 | Import | **Copy into the library root**, the same as the NAM library. The copy runs on a background job with progress, cancel and a disk check, through `.staging/` (§6.5). This differs from the first draft, which registered folders in place. |
| D3 | `installed.json` | **Retire it.** A one-time migration into `library.json` and the sidecars, then delete `registry`. |
| D4 | How a kit is chosen | **A slot param that is not automatable** (`kit_select`), as on the amp. |
| D5 | Default output | **Stereo** for a fresh instance. The Drummica track preset sets Multi — **it must be re-saved from an instance set to Multi** (see the note below the table). |
| D6 | Volume scale | **dB**, with a one-shot conversion of v1 linear values. |
| D7 | A piece the kit lacks | **Dim and silence.** The built-in kit plays only when no kit is selected. |
| D8 | Disk streaming | **Now, in this rework** (E14, slice K6b). This differs from the first draft, which deferred it. |
| D9 | Bleed / room / layered OH | **In this rework** (E15, slice K7b, after streaming). This differs from the first draft, which deferred it. |
| D10 | Index hosting | **Keep the single `index.json`** and add the optional fields of §4.2. |

**D5 and the track preset (verified 2026-10-02).** A track preset *does*
carry `output_mode`. "Save track as preset" sends `SaveAllPluginStates`,
which calls the plugin's plain `clap.state.save` (`ClapInstance::save_state`,
not `clap.state-context` `FOR_PRESET`), and applying the preset sends
`LoadPluginState` → plain `clap.state.load`
(`resonance-app/src/engine_events/{presets,plugins}.rs`). The full state
writes every param that is not `state_excluded`; `output_mode` is only
`excluded_from_presets()`, which applies to **plugin** presets
(`*.save_plugin_preset`, the preset bar), not to the full state. So a
track preset saved from a Multi instance restores Multi.

The user's existing "Drummica Kit" track preset predates the `params` map:
its drums state holds only `articulations`, `kit_path` (absolute),
`overhead_setup_key` and `pad_mic_choices`. `upgrade_output_mode` gives
Multi only to a state that *has* params but no `output_mode`, so this
preset loads as **Stereo**, and its `kit_path` becomes a `kit_ref` with no
`rel_path` until re-saved. Re-save it once from an instance with the
Drummica kit selected and Output Mode = Multi.

## 12. As built

Where `feat/drums-rework` differs from the spec above:

- **Tail slots: 32**, not 16 (E1/E15). Saturation at 128 voices needs the
  extra fades.
- **Polyphony: 128** (`polyphony` max and default). A pre-E15 state at 64
  (that build's maximum) is upgraded to 128 (`upgrade_polyphony`).
- **Streaming preload default: 32k frames** (E14). 64k measured 1,074 MiB
  for the default Drummica setup.
- **No Solo and no release-time params.** The inspector has Mute only; the
  Mix tab has no release-time control (E2's release and choke fades are
  fixed in ms).
- **E2: `reset()` stays an instant cut**, as the spec says: bounce resets
  before rendering, and a fade would leak into the export.
- **E11: cymbals keep their overhead on their own port** in Multi (they
  have no close mic), so the Cymbals sub-track is not silent.
- **Library overlay is an `egui::Modal`** (backdrop, modal input and Esc
  from egui), not a hand-painted overlay on the parent painter.
- **Header load progress is text only** (a percentage), no progress bar.
- **Mic labels read `position · brand mic`** (e.g. `OHsAB · Sennheiser
  e914`), not `brand · mic · position`; an unknown key shows raw.
- **`kit_select` -2 is "parked"**: a kit with no slot (outside the
  library, not yet slotted, or missing on this machine). Writing -2 returns
  to it; its text is `"<name> (external)"` / `"<name> (missing)"`.
- **D2: import copies** into the library root through `.staging/`, as
  decided; nothing is registered in place.
- **D3: `installed.json` is read once** by the migration's own read-only
  deserialiser (`drumkit_library`); `resonance_common::registry` is
  deleted, and `drumkits_root()` is the library's root (honouring
  `RESONANCE_DRUMKIT_DIR`).
- **K10 deferred**: no kit `Preview ▶`, no folder-of-WAVs import, the note
  map is read-only.
