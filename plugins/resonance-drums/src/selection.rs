//! Which kit an instance plays, and how that choice travels
//! (drums-plugin-rework.md §5).
//!
//! # `kit_select`
//!
//! A stepped parameter whose value is a **slot** in the shared kit
//! library's slot table (`drumkit_library::Library::by_slot`), or
//! [`NO_KIT`] for the built-in kit. Slots are never reused while others
//! are free, so a value recalls the same kit after kits are added or
//! deleted. Its text is the kit's name, and a name parses back to the slot
//! ([`Library::find`]) — which is what makes the control API's
//! `ParamValue::Label` pick a kit by name.
//!
//! The parameter is **not automatable** (a kit swap is a multi-gigabyte
//! decode) and **not in the state**: a slot is this machine's library
//! layout. The kit travels as a [`KitRef`] instead, and the slot is derived
//! from it on load.
//!
//! Setting the parameter — from the host, the control API or the editor —
//! loads the kit through the one path: [`apply_pending`], run by the
//! instance's watcher thread (off the audio thread), and by the editor at
//! once after its own write.
//!
//! # State v2: `kit_ref`
//!
//! `{id, name, rel_path, abs_path}`, resolved in that order of trust: the
//! manifest hash in the library (survives renames and moves), the path
//! under the library root (portable between machines), the absolute path.
//! [`upgrade_v1_state`] turns a v1 `kit_path` into one.
//!
//! # Missing
//!
//! A reference that resolves to nothing plays the built-in kit, is kept
//! verbatim (so a save does not lose it), reads as `"<name> (missing)"` in
//! `kit_select`'s text, and the editor shows a banner (§5.3).

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicI32, Ordering};
#[cfg(feature = "editor")]
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use resonance_common::drumkit_library::{self, Entry, EntryStatus, Library};
use serde_json::{Map, Value};

use crate::kit_loader::{self, KitStatus};
use crate::KitBridge;

/// `kit_select`'s value for "no kit chosen": the built-in kit plays.
pub const NO_KIT: i32 = -1;
/// The highest `kit_select` value: the library's last slot.
pub const MAX_KIT_SLOT: i32 = drumkit_library::MAX_SLOT as i32;

/// State keys (v2).
pub const KIT_REF_KEY: &str = "kit_ref";
pub const KIT_REF_FALLBACK_KEY: &str = "kit_ref_fallback";
/// State keys (v1), converted on load by [`upgrade_v1_state`].
pub const V1_KIT_PATH_KEY: &str = "kit_path";
pub const V1_KIT_PATH_FALLBACK_KEY: &str = "kit_path_fallback";

// ---------------------------------------------------------------------------
// KitRef
// ---------------------------------------------------------------------------

/// A saved reference to a kit (state v2). Every field is optional: a v1
/// state converts to a path-only reference, and a kit from outside the
/// library has no id until the library sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KitRef {
    /// sha256 of the kit's `drum_samples.json` (the library's id, D1).
    pub id: Option<String>,
    /// The name the kit had when saved — what a missing kit is called.
    pub name: Option<String>,
    /// The manifest relative to the library root, `/`-separated.
    pub rel_path: Option<PathBuf>,
    /// The manifest, absolute, on the machine that saved it.
    pub abs_path: Option<PathBuf>,
}

/// Which field of a [`KitRef`] found the kit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedBy {
    Id,
    RelPath,
    AbsPath,
}

impl KitRef {
    /// The reference to library kit `entry`.
    pub fn for_entry(entry: &Entry) -> Self {
        Self {
            id: Some(entry.id.clone()),
            name: Some(entry.name.clone()),
            rel_path: Some(entry.rel_path.clone()).filter(|p| !p.is_absolute()),
            abs_path: Some(entry.manifest_path.clone()),
        }
    }

    /// A reference to the manifest at `path`, with no id: what a v1
    /// `kit_path` becomes. `rel_path` is filled in when the manifest lies
    /// under `root`; the name is the loader's (the directory under the
    /// root, else the manifest's own directory).
    pub fn from_manifest_path(path: &Path, root: Option<&Path>) -> Self {
        let rel_path = root
            .and_then(|r| path.strip_prefix(r).ok())
            .filter(|rel| is_plain_relative(rel))
            .map(Path::to_path_buf);
        Self {
            id: None,
            name: Some(kit_loader::kit_display_name(path, root)),
            rel_path,
            abs_path: Some(path.to_path_buf()),
        }
    }

    /// The name to show for this reference.
    pub fn display_name(&self) -> String {
        if let Some(name) = self.name.as_deref().filter(|n| !n.trim().is_empty()) {
            return name.to_string();
        }
        let from_path = |p: &PathBuf| kit_loader::kit_display_name(p, None);
        if let Some(name) = self
            .rel_path
            .as_ref()
            .or(self.abs_path.as_ref())
            .map(from_path)
        {
            return name;
        }
        match &self.id {
            Some(id) => format!("kit {}", &id[..id.len().min(8)]),
            None => "kit".to_string(),
        }
    }

    /// The JSON form saved under [`KIT_REF_KEY`]. Paths are written with
    /// `/`, so a `rel_path` reads the same on every platform.
    pub fn to_json(&self) -> Value {
        let mut map = Map::new();
        if let Some(id) = &self.id {
            map.insert("id".into(), Value::String(id.clone()));
        }
        if let Some(name) = &self.name {
            map.insert("name".into(), Value::String(name.clone()));
        }
        if let Some(rel) = &self.rel_path {
            map.insert("rel_path".into(), Value::String(slash_path(rel)));
        }
        if let Some(abs) = &self.abs_path {
            map.insert(
                "abs_path".into(),
                Value::String(abs.to_string_lossy().into_owned()),
            );
        }
        Value::Object(map)
    }

    /// Read a saved reference: `null` (or anything that names nothing) is
    /// `None`. A bare string is taken as an absolute path.
    pub fn from_json(value: &Value) -> Option<Self> {
        let str_of = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let r = match value {
            Value::String(s) if !s.trim().is_empty() => Self {
                abs_path: Some(PathBuf::from(s)),
                ..Self::default()
            },
            Value::Object(_) => Self {
                id: str_of("id").map(|s| s.to_ascii_lowercase()),
                name: str_of("name"),
                rel_path: str_of("rel_path").map(PathBuf::from),
                abs_path: str_of("abs_path").map(PathBuf::from),
            },
            _ => return None,
        };
        (r.id.is_some() || r.rel_path.is_some() || r.abs_path.is_some()).then_some(r)
    }

    /// Find the kit: its id in `library`, then `rel_path` under `root`,
    /// then `abs_path`. Each candidate must be a manifest file that exists.
    pub fn resolve(
        &self,
        library: Option<&Library>,
        root: Option<&Path>,
    ) -> Option<(PathBuf, ResolvedBy)> {
        if let (Some(id), Some(lib)) = (&self.id, library) {
            if let Some(e) = lib.by_id(id) {
                if e.manifest_path.is_file() {
                    return Some((e.manifest_path.clone(), ResolvedBy::Id));
                }
            }
        }
        if let (Some(rel), Some(root)) = (&self.rel_path, root) {
            if is_plain_relative(rel) {
                let path = root.join(rel);
                if path.is_file() {
                    return Some((path, ResolvedBy::RelPath));
                }
            }
        }
        if let Some(abs) = &self.abs_path {
            if abs.is_absolute() && abs.is_file() {
                return Some((abs.clone(), ResolvedBy::AbsPath));
            }
        }
        None
    }
}

/// A relative path with no `..`, root or prefix: safe to join under the
/// library root.
fn is_plain_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty() && path.components().all(|c| matches!(c, Component::Normal(_)))
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Convert a v1 state's kit keys in place: `kit_path` → `kit_ref`,
/// `kit_path_fallback` → `kit_ref_fallback` (a v2 key already present
/// wins, and the v1 key is dropped either way). `root` is the library
/// root, so a v1 path under it gains the portable `rel_path`.
///
/// This is the whole v1 → v2 migration (`project_no_real_users_yet`).
pub fn upgrade_v1_state(state: &mut Value, root: Option<&Path>) {
    let Some(obj) = state.as_object_mut() else {
        return;
    };
    for (v1, v2) in [
        (V1_KIT_PATH_KEY, KIT_REF_KEY),
        (V1_KIT_PATH_FALLBACK_KEY, KIT_REF_FALLBACK_KEY),
    ] {
        let Some(old) = obj.remove(v1) else {
            continue;
        };
        if obj.contains_key(v2) {
            continue;
        }
        let converted = match old.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            Some(path) => KitRef::from_manifest_path(Path::new(path), root).to_json(),
            // `kit_path: null` is "no kit", which a v2 `kit_ref: null` says.
            None => Value::Null,
        };
        obj.insert(v2.to_string(), converted);
    }
}

// ---------------------------------------------------------------------------
// The shared library, as selection sees it
// ---------------------------------------------------------------------------

/// The process-wide kit library, opened on first use — a `kit_select`
/// text, a state's reference, a slot pick. Headless builds have no
/// library (it lives with the download worker behind `editor`): there
/// a reference resolves by path only and a slot names nothing.
#[derive(Default)]
pub struct LibraryHandle {
    #[cfg(feature = "editor")]
    cell: OnceLock<Arc<crate::library::SharedKitLibrary>>,
}

impl LibraryHandle {
    /// The shared library, opening it on first use.
    #[cfg(feature = "editor")]
    pub fn shared(&self) -> Arc<crate::library::SharedKitLibrary> {
        self.cell.get_or_init(crate::library::shared).clone()
    }

    /// Use `library` rather than the process default (an editor test's
    /// own root). `false` once one is in use already.
    #[cfg(feature = "editor")]
    pub fn set(&self, library: Arc<crate::library::SharedKitLibrary>) -> bool {
        self.cell.set(library).is_ok()
    }

    /// The library root a `rel_path` resolves under.
    pub fn root(&self) -> Option<PathBuf> {
        #[cfg(feature = "editor")]
        {
            self.shared().root().map(Path::to_path_buf)
        }
        #[cfg(not(feature = "editor"))]
        {
            drumkit_library::default_root()
        }
    }

    /// `f` over the library, waiting for a writer's swap if one is under
    /// way. `None` in a headless build. Never from the audio thread.
    pub fn with<R>(&self, f: impl FnOnce(&Library) -> R) -> Option<R> {
        #[cfg(feature = "editor")]
        {
            let lib = self.shared();
            let guard = lib.read();
            Some(f(&guard))
        }
        #[cfg(not(feature = "editor"))]
        {
            let _ = f;
            None
        }
    }

    /// [`with`](Self::with), or `None` at once while a writer swaps the
    /// index in.
    pub fn try_with<R>(&self, f: impl FnOnce(&Library) -> R) -> Option<R> {
        #[cfg(feature = "editor")]
        {
            let lib = self.shared();
            let guard = lib.try_read()?;
            Some(f(&guard))
        }
        #[cfg(not(feature = "editor"))]
        {
            let _ = f;
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Per-instance selection state
// ---------------------------------------------------------------------------

/// What this instance's `kit_select` means beyond a library slot, shared
/// by the parameter's text closures, the state saver, the watcher and the
/// editor.
#[derive(Default)]
pub struct KitSelection {
    pub library: LibraryHandle,
    /// Held across every "read `kit_select`, act on it, record `acted`"
    /// — the watcher's [`apply_pending`], the editor's [`select_now`],
    /// and a state load's [`adopt_state_kit`] with the load it starts —
    /// so no two of them interleave: a watcher that read a host write
    /// cannot act on it after a state load moved past it (and clear the
    /// state's fallback kit, or start a second load).
    act: Mutex<()>,
    /// The `kit_select` value this instance last acted on, so the watcher
    /// sees a write as a change exactly once. Written under `act`; read
    /// lock-free (text, progress).
    acted: AtomicI32,
    /// The reference a state asked for that resolved to nothing: kept
    /// verbatim (a save writes it back), shown as missing.
    missing: Mutex<Option<KitRef>>,
    /// A kit playing that holds no slot (loaded by path, from outside the
    /// library or before the library indexed it): `kit_select` parks at
    /// [`NO_KIT`] and reads as this name.
    unslotted: Mutex<Option<String>>,
    /// The reference the wanted kit was resolved from, so a save keeps
    /// its id and name even when the library does not know the kit.
    resolved_from: Mutex<Option<(PathBuf, KitRef)>>,
}

impl KitSelection {
    pub fn new() -> Self {
        Self {
            acted: AtomicI32::new(NO_KIT),
            ..Self::default()
        }
    }

    /// The missing kit's reference, if the state asked for one that
    /// resolved to nothing.
    pub fn missing(&self) -> Option<KitRef> {
        self.missing.lock().clone()
    }

    /// The value `kit_select` was last acted on at.
    pub fn acted(&self) -> i32 {
        self.acted.load(Ordering::Acquire)
    }

    /// Hold off every other act on `kit_select` (see the `act` field):
    /// a state load takes it around resolving, adopting and loading its
    /// kit. Never from the audio thread.
    pub fn acting(&self) -> parking_lot::MutexGuard<'_, ()> {
        self.act.lock()
    }

    /// `kit_select`'s text for `value`: the slot's kit name, `"<name>
    /// (missing)"` for a missing kit, the playing kit's name where it holds
    /// no slot, `"None (built-in kit)"`, `"(empty slot N)"`. Never blocks:
    /// a library mid-swap reads as `"slot N"`.
    pub fn text(&self, value: i32) -> String {
        if value == self.acted() {
            if let Some(m) = self.missing.try_lock().and_then(|m| m.clone()) {
                return format!("{} (missing)", m.display_name());
            }
            if let Some(name) = self.unslotted.try_lock().and_then(|n| n.clone()) {
                return name;
            }
        }
        if value <= NO_KIT {
            return "None (built-in kit)".to_string();
        }
        match self
            .library
            .try_with(|lib| lib.by_slot(value as u32).map(|e| e.name.clone()))
        {
            Some(Some(name)) => name,
            Some(None) => format!("(empty slot {value})"),
            None => format!("slot {value}"),
        }
    }

    /// The `kit_select` value `text` names: `"none"` / `"built-in"` is
    /// [`NO_KIT`]; a number (`"12"`, `"slot 12"`) is that slot; else a kit
    /// name or id prefix (`Library::find`). `None` when nothing (or more
    /// than one kit) matches.
    pub fn parse(&self, text: &str) -> Option<i32> {
        let t = text.trim();
        let lower = t.to_ascii_lowercase();
        if matches!(
            lower.as_str(),
            "none" | "built-in" | "builtin" | "built-in kit" | "none (built-in kit)"
        ) {
            return Some(NO_KIT);
        }
        let digits = lower.strip_prefix("slot ").unwrap_or(&lower);
        if let Ok(n) = digits.trim().parse::<i32>() {
            return (NO_KIT..=MAX_KIT_SLOT).contains(&n).then_some(n);
        }
        // `"<name> (missing)"`, as the text reads, still names the kit.
        let name = t.strip_suffix(" (missing)").unwrap_or(t);
        self.library
            .with(|lib| lib.find(name).and_then(|e| e.slot))
            .flatten()
            .map(|s| s as i32)
    }

    /// The reference a save writes for the kit at `manifest`: the library
    /// entry's (id, name, paths) when the library has it; else the one the
    /// kit was resolved from, else a path-only one.
    pub fn ref_for_manifest(&self, manifest: &Path) -> KitRef {
        // Waits for a writer's swap rather than save a reference without
        // its id (a swap is a pointer's worth of work).
        let entry = self
            .library
            .with(|lib| {
                lib.entries()
                    .iter()
                    .find(|e| e.manifest_path == manifest)
                    .cloned()
            })
            .flatten();
        if let Some(e) = entry {
            return KitRef::for_entry(&e);
        }
        if let Some((path, r)) = self.resolved_from.lock().clone() {
            if path == manifest {
                return KitRef {
                    abs_path: Some(path),
                    ..r
                };
            }
        }
        KitRef::from_manifest_path(manifest, self.library.root().as_deref())
    }

    fn clear_notes(&self) {
        *self.missing.lock() = None;
        *self.unslotted.lock() = None;
    }
}

/// What a selection started: the manifest a load is on its way for, and
/// the load generation it was given (the editor's ◀/▶ step from it while
/// it decodes).
#[derive(Clone, Debug, PartialEq)]
pub struct StartedLoad {
    pub path: PathBuf,
    pub generation: u64,
}

// ---------------------------------------------------------------------------
// Acting on kit_select
// ---------------------------------------------------------------------------

/// Act on `kit_select` if it moved since it was last acted on: load the
/// slot's kit, or the built-in kit for [`NO_KIT`]. Called by the watcher
/// thread on every wake (a host or control-API write), and by the editor
/// right after its own write. Never on the audio thread.
///
/// `Ok(None)`: nothing to do, or nothing to load.
pub fn apply_pending(bridge: &KitBridge) -> Result<Option<StartedLoad>, String> {
    let sel = &bridge.params.selection;
    let _acting = sel.act.lock();
    let value = bridge.params.kit_select.value();
    if sel.acted.load(Ordering::Acquire) == value {
        return Ok(None);
    }
    act(bridge, value)
}

/// Act on `value` and record it as acted on — after the act, so a
/// reader that sees `acted` caught up also sees the load it started.
/// Under `act`.
fn act(bridge: &KitBridge, value: i32) -> Result<Option<StartedLoad>, String> {
    let out = select(bridge, value);
    bridge
        .params
        .selection
        .acted
        .store(value, Ordering::Release);
    out
}

/// Set `kit_select` to `value` and act on it even if it already holds it —
/// the editor's pick (a pick of the playing kit reloads it, which retries
/// a kit that failed).
pub fn select_now(bridge: &KitBridge, value: i32) -> Result<Option<StartedLoad>, String> {
    let _acting = bridge.params.selection.act.lock();
    bridge.params.kit_select.set_value(value);
    let out = act(bridge, value);
    bridge.request_params_rescan();
    out
}

fn select(bridge: &KitBridge, value: i32) -> Result<Option<StartedLoad>, String> {
    let sel = &bridge.params.selection;
    if value <= NO_KIT {
        sel.clear_notes();
        play_builtin(bridge);
        return Ok(None);
    }
    let entry = sel
        .library
        .with(|lib| lib.by_slot(value as u32).cloned())
        .flatten();
    let Some(entry) = entry else {
        // An empty slot loads nothing and unloads nothing: whatever plays
        // keeps playing (nam-model-library.md §5.1).
        let message = format!("no kit in slot {value}");
        *bridge.kit_status.lock() = KitStatus::Error {
            message: message.clone(),
        };
        return Err(message);
    };
    loadable(&entry)?;
    sel.clear_notes();
    *sel.resolved_from.lock() = None;
    Ok(Some(start_kit(bridge, entry.manifest_path.clone())))
}

/// Whether library kit `entry` can be loaded, and why not.
pub fn loadable(entry: &Entry) -> Result<(), String> {
    match &entry.status {
        EntryStatus::ManifestError(reason) => {
            Err(format!("\"{}\" cannot be loaded: {reason}", entry.name))
        }
        EntryStatus::DuplicateOf(dir) => Err(format!(
            "\"{}\" is a copy of the kit in {}; load that one",
            entry.name,
            dir.display()
        )),
        EntryStatus::Ok | EntryStatus::MissingFiles(_) => Ok(()),
    }
}

/// Load the kit at `manifest` with the current mic and articulation
/// choices: now, when the host runs the plugin; else recorded as the
/// pending kit, which `initialize` loads.
pub fn start_kit(bridge: &KitBridge, manifest: PathBuf) -> StartedLoad {
    // A kit chosen now replaces whatever a reopen would have fallen back to.
    *bridge.kit_fallback.lock() = None;
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    if sr_bits == 0 {
        let mut pending = bridge.pending_kit.lock();
        let generation = bridge.load_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *pending = Some((generation, manifest.clone()));
        bridge.load_progress.begin(generation);
        return StartedLoad {
            path: manifest,
            generation,
        };
    }
    kit_loader::spawn_loader(
        manifest.clone(),
        f32::from_bits(sr_bits),
        bridge,
        bridge.overhead_setup_key.lock().clone(),
        bridge.pad_choices.lock().clone(),
        bridge.articulations(),
    );
    StartedLoad {
        path: manifest,
        generation: bridge.load_generation.load(Ordering::Acquire),
    }
}

/// Load the kit at `manifest`, which has no library slot to name it (an
/// import the index has not slotted, a relinked folder): `kit_select`
/// parks at [`NO_KIT`] and reads as the kit's name.
pub fn load_unslotted_now(bridge: &KitBridge, manifest: PathBuf) -> StartedLoad {
    let _acting = bridge.params.selection.act.lock();
    park_unslotted(bridge, &manifest);
    start_kit(bridge, manifest)
}

/// Under `act`.
fn park_unslotted(bridge: &KitBridge, manifest: &Path) {
    let sel = &bridge.params.selection;
    sel.clear_notes();
    *sel.resolved_from.lock() = None;
    let name = sel.ref_for_manifest(manifest).display_name();
    *sel.unslotted.lock() = Some(name);
    bridge.params.kit_select.set_value(NO_KIT);
    sel.acted.store(NO_KIT, Ordering::Release);
    bridge.request_params_rescan();
}

/// Play the built-in kit: no kit is wanted any more. A load in flight is
/// abandoned; while the host runs the plugin the built-in pads are built
/// off this thread and handed to the audio thread (with the usual swap
/// fade), and an inactive plugin installs them at `initialize`.
pub fn play_builtin(bridge: &KitBridge) {
    let generation = {
        let mut pending = bridge.pending_kit.lock();
        let generation = bridge.load_generation.fetch_add(1, Ordering::AcqRel) + 1;
        *pending = None;
        generation
    };
    *bridge.kit_fallback.lock() = None;
    *bridge.kit_path.lock() = None;
    *bridge.kit_status.lock() = KitStatus::Empty;
    *bridge.params.selection.resolved_from.lock() = None;
    let sr_bits = bridge.sample_rate.load(Ordering::Acquire);
    // Inactive, or the sampler is still on the built-in kit (no loaded kit
    // has been handed off since): what it holds (or will, at
    // `initialize`) is the built-in kit already. Read under the hand-off
    // lock, so a loader that passed its generation check before the bump
    // above has finished handing its kit off (and cleared `builtin_kit`).
    let on_builtin = {
        let _handoff = bridge.kit_handoff.lock();
        bridge.builtin_kit.lock().is_some()
    };
    if sr_bits == 0 || on_builtin {
        bridge.load_progress.idle(generation);
        return;
    }
    bridge.load_progress.begin(generation);
    let rate = f32::from_bits(sr_bits);
    let progress = bridge.load_progress.clone();
    let bridge = bridge.clone();
    let spawned = std::thread::Builder::new()
        .name("resonance-drums-builtin".to_string())
        .spawn(move || {
            // One pad per mapping, in order (an empty pad where the
            // embedded sample will not decode), exactly as
            // `DrumSampler::load_defaults_sourced` builds them.
            let mut pads = Vec::with_capacity(crate::drum_map::PAD_MAPPINGS.len());
            let mut sources = Vec::with_capacity(pads.capacity());
            for mapping in &crate::drum_map::PAD_MAPPINGS {
                match kit_loader::build_fallback_pad_sourced(mapping, rate) {
                    Ok((pad, source)) => {
                        pads.push(pad);
                        sources.push(Some(source));
                    }
                    Err(e) => {
                        tracing::warn!("built-in pad {}: {e}", mapping.name);
                        pads.push(crate::kit::LoadedPad {
                            name: mapping.name.to_string(),
                            choke_group: mapping.choke_group,
                            output_group: mapping.output_group,
                            close_mics: Vec::new(),
                            overhead: None,
                        });
                        sources.push(None);
                    }
                }
            }
            let mut held = kit_loader::HeldTakes::new();
            if let Some(built) = bridge.built_kit.lock().as_ref() {
                built.held_takes(&mut held);
            }
            let (shared_bytes, builtin) = kit_loader::measure_builtin_kit(&pads, &sources, &held);
            let infos = crate::sample_info::infos_for_pads(&pads, rate);
            let bytes = crate::sample_info::total_sample_bytes(&pads) as u64;
            // A newer pick (or another rate) may have taken over; then
            // the pads are simply dropped here.
            kit_loader::hand_off_kit_if_current(&bridge, pads, generation, rate, || {
                *bridge.builtin_kit.lock() = Some(builtin);
                bridge.kit_bytes.store(bytes, Ordering::Relaxed);
                bridge
                    .kit_shared_bytes
                    .store(shared_bytes, Ordering::Relaxed);
                *bridge.pad_samples.lock() = infos;
            });
        });
    if spawned.is_err() {
        progress.failed(generation);
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// What a state's kit references resolved to.
#[derive(Clone, Debug, PartialEq)]
pub struct StateKit {
    /// The manifest to load; `None`: the built-in kit.
    pub path: Option<PathBuf>,
    /// The kit a failed load of `path` falls back to.
    pub fallback: Option<PathBuf>,
    /// The wanted reference resolved to nothing (and no fallback did).
    pub missing: Option<KitRef>,
    /// The reference `path` was resolved from.
    pub from: Option<KitRef>,
}

/// Resolve a state's `kit_ref` / `kit_ref_fallback` (run
/// [`upgrade_v1_state`] first). `None` when the state carries no kit key
/// at all — a params-only preset keeps the current kit.
pub fn resolve_state(state: &Value, sel: &KitSelection) -> Option<StateKit> {
    let wanted = state.get(KIT_REF_KEY)?;
    let wanted = KitRef::from_json(wanted);
    let fallback = state.get(KIT_REF_FALLBACK_KEY).and_then(KitRef::from_json);
    let Some(wanted) = wanted else {
        return Some(StateKit {
            path: None,
            fallback: None,
            missing: None,
            from: None,
        });
    };
    let root = sel.library.root();
    let resolve = |r: &KitRef| {
        // With no library (a headless build), by path alone.
        let by_library = sel
            .library
            .with(|lib| r.resolve(Some(lib), root.as_deref()));
        by_library.unwrap_or_else(|| r.resolve(None, root.as_deref()))
    };
    let fallback_path = fallback.as_ref().and_then(|f| resolve(f).map(|(p, _)| p));
    Some(match resolve(&wanted) {
        Some((path, _)) => StateKit {
            fallback: fallback_path.filter(|f| *f != path),
            path: Some(path),
            missing: None,
            from: Some(wanted),
        },
        // The pick the project was saved during is gone: reopen on the
        // kit that last loaded, as a failed load of it would.
        None => match fallback_path {
            Some(path) => StateKit {
                path: Some(path),
                fallback: None,
                missing: None,
                from: fallback,
            },
            None => StateKit {
                path: None,
                fallback: None,
                missing: Some(wanted),
                from: None,
            },
        },
    })
}

/// Record what a state load resolved: the missing reference, the slot
/// `kit_select` now reads (the kit's library slot, else [`NO_KIT`]), and
/// the name it shows when the kit holds no slot. The kit itself is loaded
/// by the caller — all of it, from resolving the state to starting the
/// load, under [`KitSelection::acting`].
pub fn adopt_state_kit(bridge_params: &crate::params::DrumParams, kit: &StateKit) {
    let sel = &bridge_params.selection;
    sel.clear_notes();
    *sel.missing.lock() = kit.missing.clone();
    *sel.resolved_from.lock() = match (&kit.path, &kit.from) {
        (Some(p), Some(r)) => Some((p.clone(), r.clone())),
        _ => None,
    };
    let slot = kit.path.as_ref().and_then(|path| {
        sel.library
            .with(|lib| {
                lib.entries()
                    .iter()
                    .find(|e| e.manifest_path == *path)
                    .and_then(|e| e.slot)
            })
            .flatten()
    });
    let value = slot.map_or(NO_KIT, |s| s as i32);
    if let (Some(path), None) = (&kit.path, slot) {
        let r = sel.ref_for_manifest(path);
        let root = sel.library.root();
        let inside = root.as_deref().is_some_and(|r| path.starts_with(r));
        let name = r.display_name();
        *sel.unslotted.lock() = Some(if inside {
            name
        } else {
            format!("{name} (external)")
        });
    }
    bridge_params.kit_select.set_value(value);
    sel.acted.store(value, Ordering::Release);
}

/// Throttle for asking the host to re-read `kit_load_progress`: report a
/// change of a twentieth or more, and every arrival at 0 or 1.
pub fn progress_worth_reporting(last: f32, now: f32) -> bool {
    if last == now {
        return false;
    }
    (now - last).abs() >= 0.05 || now == 1.0 || now == 0.0
}

/// One look by the instance's watcher thread: act on a `kit_select` the
/// host or the control API moved, and mirror the load progress into
/// `kit_load_progress` (which `process` also does, every block; this
/// covers an inactive plugin).
pub fn watch(bridge: &KitBridge) {
    match apply_pending(bridge) {
        Ok(_) => {}
        Err(e) => tracing::warn!("kit_select: {e}"),
    }
    bridge
        .params
        .kit_load_progress
        .set_value(bridge.load_progress.fraction());
}
