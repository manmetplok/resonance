# Voicebank library (registry, plok.org install, library overlay)

> Verified against master @ `a0c05be8` + the LUNAI English singers on 2026-10-13.
> Spec history: [`docs/history/specs/vocal-mcp-control-and-quality.md`](../../history/specs/vocal-mcp-control-and-quality.md) §4–§5 (slices 4 and 7); research [`singing-voicebank-options-2026`](../../history/research/singing-voicebank-options-2026.md).

Voicebanks are content a user installs, not three banks fixed in the code.
Resonance sings with any DiffSinger bank in OpenUtau layout that can sing
English: one dropped into the user voicebank folder, one of the three
development banks under the source tree, or one installed from the
plok.org index — whose entries point at the creator's own download, never
a copy. An agent lists what is installed and what can be downloaded
(`vocal.voicebanks`) and installs a bank with its vocoder as a job
(`vocal.voicebank_install`); a user does the same in the library overlay
opened from a vocal lane's voicebank picker. How a bank is used to sing is
in [vocal rendering](vocal-rendering.md); how lyrics reach its phonemes is
in [pronunciation](pronunciation.md).

## Behaviour

### The registry

- **Ids.** A lane names its bank by string id (`VocalParams::voicebank:
  VoicebankId`, Lilia by default) and its voice mode by `speaker:
  Option<String>`. `VoicebankRegistry` (`compose/vocal_svs/registry/`)
  scans at startup, on every `vocal.voicebanks`, when the overlay opens or
  refreshes, and after an install or a Delete (`rescan`):
  - **user banks**: each folder of `$XDG_DATA_HOME/resonance/voicebanks/`
    (`$RESONANCE_VOICEBANK_DIR`) holding an acoustic `dsconfig.yaml` (the
    one with an `acoustic:` key) at most two levels down, which covers
    OpenUtau layout, Meiji v160's `configs/configs/` and the LUNAI v170
    banks' `configs/`. The id is the folder name when it already is one
    (lowercase letters, digits, `_`, `-`: `keiro_revenant`, so a plok.org
    install of an index id comes back under that id;
    `layout::folder_id`), else the name as a slug (`My Bank_v2` →
    `my-bank-v2`), or — for a folder whose name slugs to nothing — the
    character file's name as one (a warning names it; with neither the
    folder is skipped with a warning). Hidden folders (an install's
    staging, a replace or delete in progress) are never scanned;
  - **development banks**: the three shipped layouts under
    `$RESONANCE_SVS_MODELS_DIR` and `resonance-svs/models/` (resolved from
    the app crate's compile-time `CARGO_MANIFEST_DIR`) with fixed ids —
    `singer/extracted` = `tiger`, `voicebanks/lilia` = `lilia`,
    `voicebanks/meiji` = `meiji` (`BuiltinBank::dev_dir`; the LUNAI
    English singers have none — they are only ever user banks).
    `resonance-svs/scripts/download_models.sh` fetches them; models are
    git-ignored;
  - **precedence for one id**: the user folder beats every development
    root; within the user folder the first folder in sorted order wins and
    the others are skipped with a warning naming both;
    `$RESONANCE_SVS_MODELS_DIR` beats the source tree;
  - **vocoders**: the folder the acoustic config's `vocoder:` names next to
    it (TIGER's and Meiji's `dsvocoder`), else a bundled `dsvocoder/`
    (Lilia), else a shared vocoder by id in `…/resonance/vocoders/<id>/`
    (`$RESONANCE_VOCODER_DIR`; with only `$RESONANCE_VOICEBANK_DIR` set, the
    `vocoders/` folder beside it) or a dev root's `vocoder/`, matched by
    folder or by the config's `name`. A vocoder counts as installed when
    its model file exists;
  - **rescan reuse**: a bank found in the same folder as last time keeps
    its `Arc<InstalledBank>` (parsed configs, inventory, the lazily-read
    dictionary) while a `BankStamp` of what the scan read — times and
    lengths of the acoustic config, phoneme list, character file,
    `languages.json`, dictionary and vocoder config, and the bank folder's
    subfolders — still matches; a bank whose vocoder was missing is always
    re-read. Every rescan and index change bumps
    `VoicebankRegistry::revision`, which the overlay's row cache keys on;
  - **the bank's own models**: `dsdur/`, `dspitch/` and `dsvariance/`
    beside the acoustic config, each with a `dsconfig.yaml` naming a
    `linguistic` encoder and the `dur` / `pitch` / `variance` model
    (OpenUtau's layout), are found by `resonance_svs::predict::discover`
    and kept as `InstalledBank::models` (`BankModelPaths`). One counts
    when its config parses and both model files exist. All three shipped
    banks have duration and pitch models; Lilia and Meiji also a variance
    model (TIGER's acoustic model takes no variance curve). The LUNAI
    English singers ship all three (the duration model with only the
    `standard` speaker, which every mode's timing then uses).
  - **speaker embeddings** are `hidden_size` floats wide, read from the
    acoustic config: 256 on TIGER, Lilia and Meiji v160, 384 on the LUNAI
    v170 banks (Meiji v170 included). Until 2026-10-13 the width was fixed
    at 256 and every v170 bank failed its render
    (`resonance_svs::pipeline::speaker_embeddings`). A lane uses
    them through `bank_timing` / `bank_pitch` ([vocal
    rendering](vocal-rendering.md)).

  A hermetic test app scans only a private folder under its hermetic root —
  never the source tree and never `$RESONANCE_SVS_MODELS_DIR`, so the
  variable cannot change a test's result. A test that wants the real banks
  opts in with `Resonance::test_install_dev_voicebanks_from_env`.
- **Facts per bank** (`BankProfile`): derived from the files when the bank
  is installed, from the built-in table (`registry/builtin.rs`) when it is
  a known bank that is not, and conservative defaults otherwise.
  - *Speakers*: the acoustic config's `speakers`. The table holds TIGER's
    allow-list (its acoustic model lists about 40 speakers, most from other
    creators' banks; only `glam fresh disco royal electric mystic vinyl`
    are offered, matched to `…/tiger_*`), Meiji's four modes (`standard
    hunter lilith phantom` → `embeds/*`) and the LUNAI English singers'
    modes in their character files' order with display names (`vf` →
    "Vocal fry"): Katyusha `standard strong delicate lullaby`, Keiro
    Revenant `standard native cold fragile mellow aggressive solid vf
    whisper growl`, Liam Thorne `standard gentle tired deep powerful
    aggressive rock crispy vf growl`, Allen Crow `standard heavy open
    gentle whisper`. Other banks offer every model speaker, id = the slug
    of its last path segment. A bank switch resets the speaker to the
    bank's default (TIGER: by voice type; Meiji and the LUNAI singers:
    `standard`; others: the first; none for ≤1 speaker).
  - *Phoneme inventory*: the acoustic phoneme dictionary; an ARPAbet phone
    counts as present bare or `en/`-prefixed. `missing_phonemes` is ARPAbet
    minus that. A bank missing more than 8 phones is `english: false`
    (listed, refused by `vocal.set_params`).
  - *Spelling and language ids*: a phone is sent under the key the bank
    stores it as (`en/ah` on Meiji and the other LUNAI banks, whose
    English `dsdict-en.yaml` replacements only add the prefix, plus `q` →
    `en/cl`). The `replacements:` of the bank's
    `dsdict-en.yaml` come first (`dsdict::load_replacements`, read at scan
    time into `PhonemeInventory::with_replacements`): LIEE Lilia's
    multi-language inventory spells English `w` as `L`, `t` `t0`, `d` `d0`,
    `r` `r0`, `l` `ll`, `y` `j` and `v` as her own `w`. A replacement into a
    symbol the inventory lacks is ignored. The built-in table carries
    Lilia's (`shipped_replacements`) for when she is not installed. When the model takes a language input,
    a namespaced key reports its language's id from the bank's
    `languages.json` (read from the path the config names, else the bank
    root), a bare symbol 0 in a bank that namespaces English and the bank's
    id for `en` otherwise.
  - *Curves*: `use_energy_embed` gates Dynamics, `use_breathiness_embed`
    Breathiness, `use_tension_embed` Tension; pitch bend always applies.
    Read at scan time.
  - *Comfortable range* and *substitution hints* come from the table only.
    Ranges are where a plain pop line sits, not the voice's extremes, from
    the research notes and a Whisper transposition sweep
    ([vocal rendering § Measurements](vocal-rendering.md#measurements)):
    TIGER (male, A2–C♯5 at the extremes) C3–A4; Lilia (alto, optimal
    C3–G5, clearer low) D3–A♯4; Meiji (A2–D5; the research notes say
    male, LUNAI's site calls it a girl's voice) B2–F♯4; Katyusha (soprano
    / alto) G3–D5; Keiro Revenant (tenor) C3–A4; Liam Thorne (baritone /
    bass) A2–F4; Allen Crow (male, no published range) C3–A4. The LUNAI
    ranges come from a transposition sweep with the banks' own pitch
    models: Whisper word accuracy on both phrases and pitch accuracy
    (pyin against the written notes), every mode, the melody centred from
    G2 to E5. An unknown bank gets 48–76 (`range_source: "default"`).
    `BuiltinBank::speaker_ranges` gives a speaker its own range: Allen
    Crow's `open` and `gentle` (LUNAI: "intended for notes above C4"; they
    lose their low notes in the sweep) sit G3–D5; every other speaker
    reports its bank's. No shipped bank needs a substitution hint any more
    (Lilia's `v` is spelt, see above).
  - *Formant swing* (`BuiltinBank::gender_scale`): how far the timbre chip
    moves the model's `gender` input. TIGER 0.25, the others 1
    ([vocal rendering](vocal-rendering.md)).
  - *Terms* (credit, terms URL, commercial use: `free_with_credit` |
    `paid` | `permission`): `VoicebankRegistry::terms`. The built-in table
    has TIGER "TIGER by tigermeat", `paid`; Lilia "LIEE",
    `free_with_credit`; Meiji "Gahata Meiji from LUNAI Project",
    `permission`; Katyusha, Keiro Revenant, Liam Thorne and Allen Crow
    "<Name> from LUNAI Project", `permission`, all LUNAI banks with
    `https://lunaiproject.github.io/termsofuse/` (the URL §6.1 of the
    terms each bank bundles names as current; their bundled copies are
    dated 2025-11-21 and 2026-01-16). An install record (below) carries the terms the user
    accepted and wins — except on a built-in id, where the built-in terms
    win unless the record's `sha256` is the one the index lists for that
    id now. An unknown bank without a record has no terms.

### The plok.org index

`https://resonance.plok.org/index.json` carries optional `voicebanks[]` and
`vocoders[]` next to the drums' `drumkits[]`: `{id, name, version,
languages, speakers, size, bytes?, sha256?, url, terms_url, credit,
commercial, vocoder?, added}` (a vocoder entry has no languages/speakers;
its `id` is the name a bank's `dsconfig.yaml` gives it, and the folder it
installs into). `url` is the creator's own download — plok.org lists,
never hosts. Fields past `id`/`name`/`url` are lenient: one odd field does
not cost the whole list. An entry whose `id` is not a plain folder name —
one component of `[A-Za-z0-9._-]`, not starting with `.` (so no `../x`,
`/abs`, `a/b`, `.hidden`; `plok_download::is_safe_id`) — is dropped when
the index is read, with a warning. The proposed entries for TIGER, Lilia,
Meiji v170, the LUNAI English singers (Katyusha, Keiro Revenant, Liam
Thorne, Allen Crow v170: URL, bytes and sha256 from the GitHub releases
API, the zip's top level — `configs/`, `files/`, `tgm_hifigan/`, no
wrapper — read from its central directory) and the two openvpi vocoders
are committed at
[`plok-voicebank-index.json`](plok-voicebank-index.json) for the server to
publish; keys starting with `_` are notes the app ignores.

The index is fetched on its own thread (never the update loop), kept 10
minutes, and cached in `<voicebanks>/.plok-index.json` for an offline
plok.org tab (the cache is read with the library's lock released). A
hermetic test app has no index URL until a test sets one.

### Installing

- **One thread per install** (`VoicebankLibrary::start_install`,
  `registry/install.rs`), through the shared
  `resonance_common::plok_download` (the drums' transport: resumable
  `.part` with `Range` + `If-Range`, free-space check at 2.1× the file,
  10 s connect / 30 s per-read timeouts, sha256 while streaming):
  1. the index (fetched when stale; a failed fetch falls back to the
     cached index, and fails the install as `Failed` when there is none);
  2. a bank already installed as the index lists it, with its vocoder, is
     left alone (`already_installed`);
  3. the bank's upstream file is downloaded and checked: a sha256 that is
     not the index's fails with "upstream file changed; the index needs
     updating" and installs nothing;
  4. the zip unpacks through a hidden staging folder into
     `<voicebanks>/<id>/` (one wrapper directory stripped, files outside
     the `dsconfig.yaml` branch dropped, `__MACOSX` and dotfiles skipped,
     an older install moved aside and replaced only once the new one is
     whole) and must then hold an acoustic `dsconfig.yaml`;
  5. the install writes its record `.resonance-plok.json` (id, version,
     URL, sha256, terms, credit, commercial, time) into the bank folder;
  6. a bank that ships its vocoder uses it. Otherwise the vocoder its
     `dsconfig.yaml` names (or the entry's `vocoder`) is looked up in the
     vocoder library and, when missing, fetched from `vocoders[]` into
     `<vocoders>/<vocoder id>/` (`vocoder.yaml` at the top or one folder
     down), with its own record.
- **Ids are folder names.** `Install::run`, `start_install`, the control
  handler, the overlay's Delete, `delete_user_bank` and
  `VoicebankRegistry::user_bank_dir` all refuse an id (bank or vocoder)
  that is not a safe folder name; over the wire that is `not_found`.
- **Concurrent installs.** Each install names its `.part` files with a
  nonce of its own (`plok_download::new_nonce`), so two never write one
  file. A part is opened without truncating, locked, and only then emptied
  and given its `.meta` record — a part another download holds is never
  cut. Two banks that need the same missing vocoder do not both fetch it:
  the first claims the vocoder id (`VoicebankLibrary::claim_vocoder`), the
  second shows "Waiting for <vocoder>…" (`InstallPhase::WaitingFor`) and
  then finds it installed (`vocoder_fetched: false`).
- **Install records are trusted for their own folder only.** The registry
  reads `.resonance-plok.json` only when its `id` is the folder's name; a
  record copied in with another bank is ignored (warning). UPDATE is
  offered when the record no longer matches the index entry (same pinned
  sha256, else same version and URL) — read from the record loaded with
  the bank, never from disk while the overlay builds its rows.
- **Leftovers.** `VoicebankLibrary::sweep_leftovers` runs on a thread of
  its own at startup and whenever the overlay opens, over the voicebank
  and vocoder folders:
  - `.part` files nobody holds the lock of, when they have had no record
    for a minute or no write for an hour (`sweep_stale_downloads`, shared
    with the drums);
  - the hidden folders installs and Deletes set aside —
    `.<name>.staging-<pid>-<nonce>`, `.<name>.old-<pid>-<nonce>`,
    `.<name>.deleting-<pid>-<nonce>` (`package::aside_name`) — of a
    process that is gone, or of this process while no install runs
    (`package::sweep_install_leftovers`); another live process's are left
    alone. An `.old-…` whose bank folder is missing (the process died
    between the two renames of a replace) is renamed back instead.
  An install thread passes through the sweep's gate before it writes
  anything, so a sweep that found the library idle never removes a new
  install's staging folder.
- **Collecting.** Progress and the outcome land in the shared
  `LibraryState`. The tick (`update::voicebank_library::poll`) collects
  finished installs, rescans the registry and only then resolves a
  control job, so a client that waited finds the bank listed. A finished
  install that a control job waits on stays until the tick collects it,
  even when the same bank is installed again before that tick.
- Nothing here touches the open project: no undo entry, no revision.

### The overlay

Opened by **Library…** beside a vocal lane's Voicebank picker; tabs
`Installed | plok.org` over the shared `BrowserModel` and marks
(`voicebank:<id>`). The rows are built in the update loop, never in
`view`:
- rebuilt when the library's structure (installs started or finished,
  the index) changes, the registry is rescanned or the marks move
  (`LibraryOverlay::built_from`);
- a running install's progress only patches its own row's status in place
  (`LibraryState::progress_revision` → `patch_statuses`, redraw key
  `status_generation`); the browsers do not re-filter;
- typing in the search field, selecting a row or switching tabs re-filters
  the browser over the same rows.

## Where the logic lives

| Concern | Location | Key items |
|---|---|---|
| Registry | `resonance-app/src/compose/vocal_svs/registry/{mod,roots,scan,stamp,installed,profile,builtin,dict}.rs` | `VoicebankRegistry` (`scan`, `rescan`, `revision`, `profile`, `get`, `is_known`, `user_bank_dir`), `VoicebankRoots` (`for_machine`, `from_dirs`, `hermetic`, `dev_models_from_env`), `BankStamp`, `InstalledBank` (`plok`: the trusted install record), `install_record`, `BankProfile`, `VoicebankMissing`, `BUILTIN_BANKS` |
| Index and terms | `registry/{plok_index,terms}.rs` | `PlokIndex` (ids checked at parse), `PlokVoicebank`, `PlokVocoder`, `PlokSidecar` (`.resonance-plok.json`, `matches`), `VoicebankRegistry::{set_index, in_index, terms}`, `BankTerms` |
| Installs | `registry/{library,install}.rs` | `VoicebankLibrary` (`index`, `refresh_index`, `start_install`, `cancel`, `take_finished`, `claim_vocoder`, `sweep_leftovers[_now]`), `LibraryState` (`revision`, `progress_revision`, `job`, `live_job`), `InstallPhase`, `InstallOutcome`, `InstallError`, `Install::run`, `delete_user_bank` |
| Shared transport | `resonance-common/src/plok_download.rs`, `plok_download/{part,transfer,package,index,http}.rs` | `is_safe_id`, `new_nonce`, `PartFile::acquire`, `download`, `sweep_stale_downloads`, `package::{install_package, aside_name, Aside, sweep_install_leftovers}` |
| App state | `resonance-app/src/lib.rs`, `update/compose/vocal_control.rs` | `Resonance::{voicebanks, voicebank_library}`, `rescan_voicebanks`, the startup sweep |
| Overlay | `state/voicebank_library.rs`, `update/voicebank_library.rs`, `view/compose/voicebank_library.rs`, `state/overlay.rs` | `VoicebankLibraryState`, `LibraryOverlay` (`built_from`, `built_progress`, `rows_generation`, `status_generation`), `VoicebankRows` (`LibraryRows`), `VoicebankLibraryMessage`, `poll`, `refresh_rows`, `patch_statuses`, `Overlay::VoicebankLibrary` |
| GUI pickers | `view/compose/lane_inspector/vocal/voice_group.rs`, `state/picks.rs`, `state/ui_caches.rs` | `voicebank_pickers`, `VoicebankChoice`, `SpeakerChoice`, `UiViewCaches::{voicebank_choices, missing_voicebank_choices}` |
| Control | `update/control/vocal.rs`, `view_model/voicebanks.rs`, `gate.rs`, `control_jobs.rs` | `voicebanks`, `voicebank_install`, `voicebank_install_settled`, `voicebank_list`, `JobToken::VoicebankInstall` |
| SVS crate | `resonance-svs/src/voicebank/{mod,layout,dsdict,phonetics}.rs`, `predict/config.rs` | `voicebank::scan`, `looks_like_bank`, `find_library_vocoder`, `installed_vocoder_name`, `read_curve_support`, `predict::discover`, `BankModelPaths`, `ModelConfig` |

## Surfaces

- **GUI:** the Voice & delivery card's **Voicebank** pick list offers the
  installed English banks (options cached in
  `UiViewCaches::voicebank_choices`, rebuilt on every rescan; a lane on a
  built-in bank that is not installed shows `<id> (not installed)`) and a
  **Speaker** list (hidden for a bank with at most one). **Library…** opens
  the overlay: search with the shared syntax (`is:fav`, `lang:en`,
  `is:user` / `is:dev` / `is:plok`) and ★, a list with INSTALLED / UPDATE /
  % chips, and a detail pane with credit, terms URL, commercial use and
  vocoder. Download (and Update) first shows the terms and credit line and
  starts only on **Accept & download**; a running install shows its
  progress and **Cancel**; **Delete…** (user installs only) asks first;
  **Use on this lane** sets the lane's bank (one undoable lane edit) and
  closes the overlay. Esc or the backdrop closes it; installs keep going.
- **Control API / MCP:**
  - `vocal.voicebanks` → `mcp__resonance__vocal_voicebanks` (read-only,
    above the mutation gate, no revision, no project needed):
    `{installed: [{id, name, version, english, speakers [{id, name,
    comfortable_range}], default_speaker, comfortable_range {low, high,
    range_source}, missing_phonemes, substitutions [{phoneme, sung_as}],
    curves {dynamics, tension, breathiness, pitch_bend}, models
    {duration, pitch, variance}, credit,
    terms_url, commercial, vocoder, vocoder_installed}], available: [{id,
    name, version, terms_url, credit, commercial, languages, speakers,
    size, vocoder, url}], index_status: fresh | cached | fetching |
    unavailable, index_error?}`. It rescans first, reads the index from
    memory or its disk cache and starts a background fetch when it is
    stale.
  - `vocal.voicebank_install {id}` → `vocal_voicebank_install`, a job
    (`job_status` / `job_wait`, MCP wait 120 s; above the mutation gate, no
    undo entry, no revision, no project needed): installs the bank and the
    vocoder it needs; the result is `{id, name, version, terms_url, credit,
    commercial, vocoder, vocoder_installed, vocoder_fetched,
    already_installed}`. Running it accepts the terms, so the agent passes
    them on. An id that is not a folder name, or that the known index does
    not list, is refused `not_found` (a job that finds out itself fails
    with `not_found`); a bank already installing is `busy`; an index that
    cannot be fetched fails the job without a kind; a hash mismatch fails
    it with the "upstream file changed" message.
  - `vocal.set_params` accepts any installed bank, the seven built-in ids
    and an id the index lists (English when its `languages` name `en` or
    nothing) — see [lyrics and melody](vocal-lyrics-and-melody.md).
- **Agent skills:** the `arranging` skill's generator reference names the
  bank ids; external `singer-songwriter` / `resonance-studio` skills pick
  banks.

## How it's tested

| What | Where | Run |
|---|---|---|
| Registry scans (temp-dir fixture banks: slugs, two-level layout, languages, shared vocoder, user-over-dev, rescan, built-in fallbacks); the four LUNAI v170 banks in their zip layout register under their folder ids with the table's names, mode names, ranges (Allen Crow's per-speaker ones) and terms, `en/` + language id 3, their own three models and bundled vocoder | `resonance-app/tests/vocal/voicebank_registry.rs` (`lunai_v170_banks_register_under_their_folder_ids_with_the_builtin_facts`); fixtures from `resonance_app::{write_voicebank_fixture, VoicebankFixture}` | `./scripts/run-tests.py -p resonance-app` |
| Folder ids (`keiro_revenant` kept, `My Bank_v2` slugged); the LUNAI v170 layout scanned; 384-wide embeddings loaded, a width other than `hidden_size` refused, 256 by default | `resonance-svs/tests/voicebank_manifest.rs` (`a_folder_already_named_as_an_id_keeps_it`, `a_lunai_v170_bank_scans_and_loads_its_384_wide_embeddings`, `embeddings_default_to_256_wide`) | `./scripts/run-tests.py -p resonance-svs` |
| Real banks register with the right ids, speakers, curve flags; the real LUNAI singers register and load their embeddings | `tests/vocal/voicebank_curve_support.rs` (no-ops unless set; `real_lunai_banks_register` reads `$RESONANCE_VOICEBANK_DIR`) | `RESONANCE_SVS_MODELS_DIR=…/resonance-svs/models RESONANCE_VOICEBANK_DIR=~/.local/share/resonance/voicebanks cargo test -p resonance-app --test vocal` |
| `vocal.voicebanks`, `voicebank_missing`, set_params bank checks; hermetic roots and the dev-bank opt-in; vocoders beside `$RESONANCE_VOICEBANK_DIR`; id fallback and duplicate precedence; rescan reuse | `tests/control/control_voicebanks.rs`, `control_jobs.rs` | same |
| Install over a loopback server: available → installed, vocoder auto-fetch, terms in the result, no revision / undo, `already_installed`, sha mismatch installs nothing, a cut download resumes with `Range`, unknown id `not_found`, no project needed; the committed index proposal parses (the LUNAI four pinned, English, and with the built-in table's credit and terms) | `tests/control/control_voicebank_install.rs` (server: `tests/common/plok_server.rs`, `Act::{CutAfter, DelayMs}`) | same |
| Guards: `../x`, `/abs`, `a/b`, `.hidden`, `..` dropped from the index and refused (nothing written outside the library), Delete refuses a non-id folder; two installs needing one vocoder fetch it once; a finished install's job settles when the bank is installed again before the tick; a failed index fetch is not `not_found`; the sweep (dead / own-idle / live-other staging, deleting, a cut replace restored, a landed replace's old copy removed); install records trusted for their own folder and, on a built-in bank, only for the index's pinned file | `tests/control/control_voicebank_install_safety.rs` | same |
| Part-file protocol and the record-less-part age rule (shared with the drums) | `plugins/resonance-drums/tests/download.rs` (`a_starting_worker_sweeps_stale_download_leftovers`, resume and lock tests) | `./scripts/run-tests.py -p resonance-drums` |
| Overlay: Library… opens it on the lane's bank, a download waits for Accept (Decline fetches nothing), Use sets the lane, Delete asks first, Cancel; progress patches one row and typing rebuilds none; goldens of the Installed tab, plok.org tab, accept-terms, downloading and confirm-delete states | `tests/compose/compose_voicebank_library.rs` → `compose_voicebank_library_{installed,plok,accept_terms,downloading,confirm_delete}-wgpu.png` | `RESONANCE_BLESS=1` to re-bless |
| The bank's own models found beside the acoustic config (a missing model file does not count; speakers map by last path segment, else the model's first); `vocal.voicebanks` `models`; a lane's flags pick them | `resonance-svs/tests/voicebank_manifest.rs` (`the_banks_own_models_are_found_next_to_the_acoustic_config`), `tests/control/control_voicebanks.rs` (`voicebanks_reports_the_banks_own_models`), `tests/vocal/vocal_bank_models.rs` (`a_lane_uses_the_bank_models_its_flags_ask_for`) | `./scripts/run-tests.py` |
| Inspector pickers (installed bank + speaker; `lilia (not installed)`) | `tests/compose/compose_rail_collapse.rs` → `compose_vocal_rail_installed_bank-wgpu.png` | same |

Not covered: a real install from GitHub (hundreds of MB) has not been run;
the tests use a loopback server and fixture banks. The LUNAI four were
unpacked by hand into the user voicebank folder, which the installer's
plan would produce from the same zips (no wrapper, `configs/dsconfig.yaml`
one level down).

## Decisions and deviations

- **Distribution (D14).** The index points at the creator's URL and pins
  the file by sha256; plok.org holds no copies, because TIGER's and LUNAI's
  terms forbid rehosting and Lilia's ask for permission.
- **Facts (D10).** Derived from the bank's files first, the built-in table
  second (speaker allow-lists, ranges, substitution hints, terms).
- `vocal.voicebanks` never blocks on the network (against §5): a first
  call answers `index_status: fetching` with an empty `available`.
- The install record (`.resonance-plok.json`) is an addition: it carries
  the accepted terms and drives UPDATE. A bank with no record never reads
  UPDATE (only plok.org installs are compared). Because anyone can write
  the file, it is read only when its id is its folder's name, and on a
  built-in id it cannot override the built-in terms unless it pins the
  index's current file.
- Index entries name vocoders by their `dsconfig.yaml` name, which is also
  the folder they install into.
- Delete is GUI-only and removes user-folder banks only, never development
  banks; there is no MCP delete.
- `vocal.generate` reports a missing bank as a `render_error` field of its
  result rather than failing (the melody is written either way).
- Aside folders carry the owning process id, so a sweep can tell a dead
  install's leftovers from another running Resonance's.
- The model root falls back to a compile-time source path, so an installed
  binary away from the source tree needs `RESONANCE_SVS_MODELS_DIR` (or the
  bank in the user voicebank folder).

## Known gaps

- The live `resonance.plok.org/index.json` does not list voicebanks yet:
  publishing [`plok-voicebank-index.json`](plok-voicebank-index.json) is
  server work, and TIGER's and the NSF-HiFiGAN 2024.02 entry's sha256 are
  not pinned (GitHub publishes no digest; they must be hashed before
  publishing). TIGER's pack layout was not inspected. Which audition
  candidates join the index is still open —
  [vocal follow-ups](../../intents/compose/vocal-followups.md).
- Lilia's ToU forbids "AI-generated music and other generative AI
  contexts" and its vocoder is CC BY-NC-SA; D9 accepts both while
  Resonance is not commercial — see
  [vocal follow-ups](../../intents/compose/vocal-followups.md).
- Per-speaker ranges exist for Allen Crow's `open` and `gentle` only.
  Vocal fry modes (Keiro's, Liam's `vf`) hit their notes loosely
  anywhere (pitch within 50 cents on 18–70 % of the notes for Keiro), and
  growl modes lose their lowest notes; both keep their bank's range.
- Meiji's range and gender are uncertain: the research notes call the
  voice male (A2–D5), LUNAI's site describes a girl's voice. The range
  stays B2–F♯4, which measures well and which the 12-voice listen
  approved.
- Non-English banks are listed (`english: false`) but cannot be used,
  because lyrics are English.
