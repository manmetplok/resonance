# Drum kit library and plok.org downloads

> Verified against master @ `2b28debd` + vocal phase 4 on 2026-10-07.
> Spec history: `docs/history/specs/drums-plugin-rework.md` §3–§4, slices
> K1–K3 and K9 (merged in `0984dfc6`).

Drum kits live in one shared library on disk. Users browse, favourite, tag,
import, delete and download kits from the drums editor's Library overlay;
the app and agents read the same library to list kits and set marks. Kits
come from `https://resonance.plok.org/index.json` or from a folder or zip
the user imports.

## Behaviour

- **Location.** `$XDG_DATA_HOME/resonance/drumkits` (override
  `RESONANCE_DRUMKIT_DIR`). A kit is a directory directly under the root
  with `drum_samples.json` at depth 0 or 1. `library.json` caches entries and
  the slot table; writes take `library.lock`. A one-time migration reads the
  old `installed.json`.
- **Identity and slots.** A kit's id is the sha256 of its
  `drum_samples.json`. Slots (`0..=999`, what the `kit_select` param holds)
  are append-only below the high-water mark, so a project's slot keeps
  pointing at the same kit. The slot/identity core is shared with the NAM
  library (`resonance-common/src/content_index.rs`).
- **Manifest.** `{piece: {setup_key: {brand, channel, mic, position,
  rounds: {RRnn: {VelNN: file}}}}}` plus an optional `_meta` (`name`,
  `pieces.<p>.name`, `articulations`, `pads.<p>.{note, port, choke}`,
  `mic_kinds`). Each `_meta` part is read on its own; a bad part is ignored,
  not fatal.
- **Import** copies a folder or `.zip` into the root through `.staging/`
  with a free-space check, progress and cancel; stale staging is swept after
  24 h. The source must already contain a `drum_samples.json`.
- **plok.org downloads** (`plugins/resonance-drums/src/download.rs`): one
  process-wide worker. Its transport — HTTP agent and timeouts, index
  fetch and cache, the resumable `.part` protocol, the free-space check
  and the sha256 verify — is `resonance_common::plok_download` (feature
  `plok-download`), shared with the app's voicebank library; the worker
  keeps its queue, threads, status, lifetime rules and the kit-library
  install. The index is cached 10 minutes in
  `<root>/.plok-index.json` and fetched on its own thread so it never waits
  behind a download. Entries carry `name` and `file`, and optionally `size`,
  `bytes`, `sha256`, `pieces`, `mic_setups`, `description`, `tags`, `added`,
  `manifest_sha256`. Downloads resume (`Range` + `If-Range` on `.part`
  files), check free space (2.1× the zip), time out (connect 10 s, read
  30 s), verify sha256 when the index gives one, and install through
  `Library::install_zip[_replacing]` (handles a wrapper directory, skips
  `__MACOSX` and dotfiles). An update whose id changes carries the marks
  over. The Update badge compares `manifest_sha256`.
- **Marks.** Favourite, personal tags, last used and use count live in the
  shared marks file under `drumkit:<id>`
  (`resonance-common/src/library_marks.rs`), next to amp models and plugin
  presets.
- **Library overlay** in the drums editor (`egui::Modal`, tabs Installed |
  plok.org): search with the shared syntax (`is:fav`, `is:recent`, `tag:`,
  `is:plok`, `mics:`, `articulations:` …), ★, Recent, Source/Mics/Sort
  facets, detail pane, Load, Reveal, Re-download, Delete with confirmation,
  Import, Rescan; the plok.org tab has Download with progress and Cancel,
  Installed/Load and Update.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Library | `resonance-common/src/drumkit_library.rs`, `drumkit_library/{manifest,sidecar,install}.rs` | `Library`, `Entry`, `EntryStatus`, `Source`, `hash_manifest`, `install_zip` |
| Shared slot core | `resonance-common/src/content_index.rs` | `MAX_SLOT` |
| Marks | `resonance-common/src/library_marks.rs` | `Marks`, `SharedMarks`, `kind::DRUMKIT` |
| Browser rows | `resonance-plugin/src/kit_rows.rs`, `library_view.rs` | `KitRows`, `BrowserModel`, `parse_search` |
| Plugin-side handle | `plugins/resonance-drums/src/library.rs`, `download.rs` | `SharedKitLibrary`, `WorkerHandle`, `ServerKit::target` |
| Shared transport | `resonance-common/src/plok_download.rs`, `plok_download/{http,index,part,transfer,package,read_timeout}.rs` | `INDEX_URL`, `HttpConfig`, `fetch_index`, `download` + `Control` + `Target`, `PartFile`, `sweep_stale_downloads`, `DownloadError` (`ChecksumMismatch`), `package::install_package` (used by voicebanks, not kits) |
| Overlay UI | `plugins/resonance-drums/src/editor/{library_panel,plok_panel,jobs,missing_kit}.rs` | Installed / plok.org tabs |
| App cache + handlers | `resonance-app/src/state/library_cache.rs` (`DrumKitLibraryCache`), `resonance-app/src/update/control/drum_kits.rs` | list, set_marks |

Invariant: the library file is the only slot authority and every writer
takes its lock; the app never imports, downloads or deletes kits.

## Surfaces

- **GUI:** drums editor → `Library…`; missing-kit banner (Download, Locate,
  Choose); app drum-group kit picker.
- **Control API / MCP:** `drum_kits.list` → `mcp__resonance__drum_kits_list`
  (filters `query`, `favorites_only`, `source`, `limit`; each entry has
  `slot`, `id`, `name`, `description`, `pieces`, `piece_names`,
  `mic_setups`, `layers`, `rr`, `size_bytes`, `source`, `favorite`, `tags`,
  `last_used`, `status`, `error`, `loaded_in`); `drum_kits.set_marks` →
  `mcp__resonance__drum_kits_set_marks` (`id` or an 8+ char prefix,
  `favorite`, `tags` replaces). Both run above the mutation gate: no undo
  entry, no revision bump. Loading a kit is `track_set_plugin_param` on
  `kit_select`.
- **Agent skills:** `resonance-agent-plugin/skills/drumming` names
  `drum_kits_list`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Library core | `resonance-common/tests/{drumkit_library,library_marks,content_index_conformance}.rs` | `./scripts/run-tests.py -p resonance-common` |
| Plugin library + downloads (local HTTP server) | `plugins/resonance-drums/tests/{kit_library,shared_library,download,library_overlay}.rs` | `./scripts/run-tests.py -p resonance-drums` |
| The shared transport with voicebanks (resume, sha mismatch, wrapper strip) | `resonance-app/tests/control/control_voicebank_install.rs` | `./scripts/run-tests.py -p resonance-app` |
| Browser rows | `resonance-plugin/tests/kit_rows.rs` | `./scripts/run-tests.py -p resonance-plugin` |
| Control methods | `resonance-app/tests/control/control_drum_kits.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: the real `resonance.plok.org` server; tests use a local server.

## Decisions and deviations

- D1 identity is the manifest hash; D2 import copies (the first draft
  registered folders in place); D3 `installed.json` and the old `registry`
  module are retired; D10 one `index.json` with optional fields.
- The control API is read and mark only. Download, import and delete stay
  in the plugin editor.

## Known gaps

- The hosted index still lacks `bytes`, `sha256` and `manifest_sha256` for
  the Drummica entry, so it is never checksum-verified and its Update badge
  falls back to the name; IT Techno is not published. That is server work
  outside the repo:
  [drums-live-verification](../../intents/plugins-instruments/drums-live-verification.md).
- Folder-of-WAVs import with a generated manifest, and kit preview:
  [drums-kit-authoring](../../intents/plugins-instruments/drums-kit-authoring.md).
- `drum_kits.list` gives piece names but not the pad/note map:
  [feature-request-2026-10-07-drum-pad-map-mcp](../../intents/control-api/drum-pad-map-mcp.md).
