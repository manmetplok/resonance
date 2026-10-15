# NAM model library and Tone3000 downloads

> Verified against master @ `6137b6fb` on 2026-10-15.
> Spec history: `docs/history/specs/nam-model-library.md` (merged as
> `e28ca519`; its status line still says "unmerged").

Amp models (`.nam` files) live in one library shared by every amp instance
and the app. In the amp editor's Library overlay a user browses installed
models, favourites and tags them, imports files, deletes them, and searches
and downloads captures from tone3000.com after signing in. Projects point at
a model by content hash, so a moved file is found again. Agents list models
and set marks over the control API.

## Behaviour

- **Location.** `$RESONANCE_AMP_MODEL_DIR`, else
  `<data dir>/resonance/amp-models`, with `tone3000/` (downloads plus
  `*.nam.meta.json` sidecars), `imported/`, `library.json` and
  `library.lock`.
- **Identity and slots.** The id is the sha256 of the file. Slots
  (`0..=999`, what the amp's "Model Select" holds) are append-only below the
  high-water mark; the slot core is shared with the drum-kit library
  (`resonance-common/src/content_index.rs`). Duplicate files are marked
  `DuplicateOf`, unreadable ones `Unreadable`.
- **Entries** carry name, author, gear, gear type, tone type,
  architecture, sample rate, size, mtime, added time, source (Tone3000,
  Imported, External), ESR and loudness, read from the file header without
  loading the weights.
- **Import** copies a `.nam` into `imported/` (D3). **Delete** removes the
  file permanently but keeps its marks (D4). There is no watched folder.
- **Marks.** Favourite, personal tags, last used and use count in the
  shared marks file (`$XDG_DATA_HOME/resonance/library/marks.json`,
  override `RESONANCE_LIBRARY_DIR`) under `amp-model:<sha>`. Writes lock
  `marks.lock` and apply only their own change. Orphaned marks are pruned
  after 90 days. A project restore does not count as a recent pick (D10).
- **Many instances.** A process-wide `SharedLibrary` serves every amp; the
  editor shows "used in N open amps" before a delete. Each instance reloads
  when `library.json` changes.
- **Tone3000.** OAuth 2.0 authorization code + PKCE with a loopback
  listener on port 47834; tokens are stored in
  `<config dir>/resonance/tone3000.json` and refreshed on 401. Search uses
  `/api/v1/tones/search` filtered to amp and amp-cab gear, models come from
  `/api/v1/models`, downloads are capped at 200 MB and land in
  `<root>/tone3000/<name>_<model_id>.nam` with a sidecar, written
  atomically, followed by a rescan. One worker per process, created when an
  editor first opens. Re-downloading finds an existing file by Tone3000 ids.
- **Search syntax** is the shared one (`is:fav`, `is:recent`, `tag:`, `by:`,
  `is:tone3000`, `is:imported`, `gear_type:`, `tone_type:` …), the same in
  the overlay and in `amp_models.list`.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Library | `resonance-common/src/nam_library.rs`, `nam_library/{header,sidecar}.rs` | `Library::{open, rescan, reload_if_changed, import, delete, find, by_slot}`, `Entry`, `read_header` |
| Marks + vocabulary | `resonance-common/src/library_marks.rs`, `library_marks/vocab.rs` | `Marks`, `SharedMarks`, `kind::AMP_MODEL` |
| Browser rows | `resonance-plugin/src/nam_rows.rs`, `library_view.rs` | `ModelRows`, `BrowserModel` |
| Plugin glue | `plugins/resonance-amp/src/library.rs`, `models.rs` | `SharedLibrary`, usage registry |
| Tone3000 | `plugins/resonance-amp/src/tone3000/{auth,client,types,worker}.rs` | `TONE3000_CLIENT_ID`, `REDIRECT_PORT`, `finalize_download` |
| Overlay UI | `plugins/resonance-amp/src/editor/{library_panel,tone3000_panel,jobs,actions}.rs` | |
| App cache + handlers | `resonance-app/src/state/library_cache.rs` (`AmpLibraryCache`), `resonance-app/src/update/control/amp_models.rs` | `list`, `set_marks`, `slot_for_label` |

Invariant: the library file is the only slot authority; the app never
imports, downloads or deletes models.

## Surfaces

- **GUI:** amp editor → `Library…` (Installed | Tone3000), the ◀/▶ and ★ in
  the amp header, the missing-model banner.
- **Control API / MCP:** `amp_models.list` →
  `mcp__resonance__amp_models_list` (`query`, `favorites_only`,
  `gear_type`, `tone_type`, `limit`; entries give slot, id, name, author,
  gear, types, architecture, sample rate, size, source, favourite, tags,
  last used, status, error; results are favourites first, then slot order);
  `amp_models.set_marks` → `mcp__resonance__amp_models_set_marks` (`id` or
  an 8+ char prefix, `favorite`, `tags` replaces). Neither makes an undo
  entry or bumps the revision. Loading a model is `track_set_plugin_param`
  on "Model Select".
- **Agent skills:** `resonance-agent-plugin/skills/mixing` names
  `amp_models_list` and `amp_models_set_marks`.

## How it's tested

| What | Where | Run |
|---|---|---|
| Library + marks | `resonance-common/tests/{nam_library,library_marks,content_index_conformance}.rs` | `./scripts/run-tests.py -p resonance-common` |
| Plugin side (resolve, v1 → v2 state, slot text, delete keeps marks, headless editor frames) | `plugins/resonance-amp/tests/model_library.rs`, `model_selector.rs` | `./scripts/run-tests.py -p resonance-amp` |
| Tone3000 client logic (query, paging, authorize URL, sidecar) | `plugins/resonance-amp/tests/tone3000_browser.rs` | same |
| Browser rows | `resonance-plugin/tests/library_view.rs` | `./scripts/run-tests.py -p resonance-plugin` |
| Control methods | `resonance-app/tests/control/control_amp_models.rs` | `./scripts/run-tests.py -p resonance-app` |

Not covered: the live tone3000.com API and the browser sign-in.

## Decisions and deviations

- D1–D10 were all built as their recommended option; none was put to the
  user. D6 keeps the control API to list and marks; import, delete and
  download are editor-only.
- The spec's §1 "old registry" is gone (`resonance-common/src/registry.rs`
  was deleted in the drums rework).

## Known gaps

- Doc comments disagree with the code: `tone3000/mod.rs` says the client id
  is "left empty on purpose" (it is set) and the redirect is `127.0.0.1`
  (the code sends `localhost`, binding 127.0.0.1).
- No watched folder, no loudness normalisation, no embedding models in
  projects (spec non-goals).
