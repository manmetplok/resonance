//! One conformance suite for every library built on
//! `resonance_common::content_index` (code review ARCH2-04): the NAM model
//! library and the drum-kit library must agree on lookup, slot reuse,
//! reload and locking, because agents and presets rely on the same
//! semantics through `amp_models.*` and `drum_kits.*`. Each case runs
//! against both through a small [`Lib`] adapter.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use resonance_common::{drumkit_library, nam_library};

/// What the conformance cases need from a library.
trait Lib: Sized {
    const TAG: &'static str;
    /// Add one item under `root` displayed as `name`; its bytes (so its
    /// id) depend on `seed`. Returns the path the library keys it by.
    fn add(root: &Path, name: &str, seed: u32) -> PathBuf;
    fn open(root: &Path) -> Self;
    fn rescan(&mut self);
    fn reload_if_changed(&mut self) -> bool;
    fn delete(&mut self, key: &Path);
    /// `(id, slot)` of the entry free text resolves to.
    fn find(&self, text: &str) -> Option<(String, Option<u32>)>;
    /// `(key path, id, slot)` of every entry.
    fn entries(&self) -> Vec<(PathBuf, String, Option<u32>)>;
    fn lock_file(root: &Path) -> PathBuf;

    fn slot_of(&self, key: &Path) -> Option<u32> {
        self.entries().into_iter().find(|(k, _, _)| k == key).and_then(|(_, _, s)| s)
    }
}

struct Nam(nam_library::Library);

impl Lib for Nam {
    const TAG: &'static str = "nam";
    fn add(root: &Path, name: &str, seed: u32) -> PathBuf {
        let path = root.join(nam_library::IMPORTED_DIR).join(format!("m{seed}.nam"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = format!(
            r#"{{"version":"0.5.4","architecture":"WaveNet","config":{{}},"weights":[{seed}.0],
                "sample_rate":48000,"metadata":{{"name":"{name}"}}}}"#
        );
        std::fs::write(&path, text).unwrap();
        path
    }
    fn open(root: &Path) -> Self {
        Nam(nam_library::Library::open(root))
    }
    fn rescan(&mut self) {
        self.0.rescan().unwrap();
    }
    fn reload_if_changed(&mut self) -> bool {
        self.0.reload_if_changed()
    }
    fn delete(&mut self, key: &Path) {
        self.0.delete(key).unwrap();
    }
    fn find(&self, text: &str) -> Option<(String, Option<u32>)> {
        self.0.find(text).map(|e| (e.id.clone(), e.slot))
    }
    fn entries(&self) -> Vec<(PathBuf, String, Option<u32>)> {
        self.0.entries().iter().map(|e| (e.path.clone(), e.id.clone(), e.slot)).collect()
    }
    fn lock_file(root: &Path) -> PathBuf {
        root.join(nam_library::LOCK_FILE)
    }
}

struct Kits(drumkit_library::Library);

impl Lib for Kits {
    const TAG: &'static str = "kit";
    fn add(root: &Path, name: &str, seed: u32) -> PathBuf {
        let dir = root.join(format!("kit{seed}"));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = serde_json::json!({
            "Snare": {"01_SD_SM57": {"brand": "Shure", "channel": "01", "mic": "SM57",
                "position": "SD", "rounds": {"RR01": {"Vel01": "s.wav"}}}},
            "_meta": {"name": name, "seed": seed},
        });
        std::fs::write(dir.join("s.wav"), [0u8; 16]).unwrap();
        std::fs::write(dir.join(drumkit_library::MANIFEST_FILE), manifest.to_string()).unwrap();
        dir
    }
    fn open(root: &Path) -> Self {
        Kits(drumkit_library::Library::open(root))
    }
    fn rescan(&mut self) {
        self.0.rescan().unwrap();
    }
    fn reload_if_changed(&mut self) -> bool {
        self.0.reload_if_changed()
    }
    fn delete(&mut self, key: &Path) {
        self.0.delete(key).unwrap();
    }
    fn find(&self, text: &str) -> Option<(String, Option<u32>)> {
        self.0.find(text).map(|e| (e.id.clone(), e.slot))
    }
    fn entries(&self) -> Vec<(PathBuf, String, Option<u32>)> {
        self.0.entries().iter().map(|e| (e.dir.clone(), e.id.clone(), e.slot)).collect()
    }
    fn lock_file(root: &Path) -> PathBuf {
        root.join(drumkit_library::LOCK_FILE)
    }
}

fn temp_root<L: Lib>(case: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "resonance-content-index-{}-{case}-{}",
        L::TAG,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Exact folded name, then a ≥6-hex id prefix, then a unique prefix,
/// then a unique substring; an ambiguous exact name resolves to nothing.
fn find_resolves_unambiguously<L: Lib>() {
    let root = temp_root::<L>("find");
    L::add(&root, "Crunch Amp", 1);
    L::add(&root, "Crunch Amp", 2);
    L::add(&root, "Clean Café", 3);
    let mut lib = L::open(&root);
    lib.rescan();
    assert_eq!(lib.find("crunch amp"), None, "{}: two entries share the name", L::TAG);
    let clean = lib.find("CLEAN CAFE").expect("folded exact name");
    assert_eq!(lib.find("Clean C").as_ref(), Some(&clean), "{}: unique prefix", L::TAG);
    assert_eq!(lib.find("an Ca").as_ref(), Some(&clean), "{}: unique substring", L::TAG);
    assert_eq!(lib.find(&clean.0[..8]).as_ref(), Some(&clean), "{}: id prefix", L::TAG);
    assert_eq!(lib.find("Cr"), None, "{}: an ambiguous prefix", L::TAG);
    assert_eq!(lib.find("   "), None);
    let _ = std::fs::remove_dir_all(&root);
}

/// A freed slot is not reused while the high-water mark has room, and an
/// item that comes back gets its old slot.
fn slots_are_not_reused_and_return_home<L: Lib>() {
    let root = temp_root::<L>("slots");
    let a = L::add(&root, "A", 1);
    let b = L::add(&root, "B", 2);
    let c = L::add(&root, "C", 3);
    let mut lib = L::open(&root);
    lib.rescan();
    let (sa, sb, sc) = (lib.slot_of(&a), lib.slot_of(&b), lib.slot_of(&c));
    assert_eq!([sa, sb, sc], [Some(0), Some(1), Some(2)], "{}: scan order", L::TAG);
    lib.delete(&b);
    assert_eq!(lib.entries().len(), 2);
    let d = L::add(&root, "D", 4);
    lib.rescan();
    assert_eq!(lib.slot_of(&d), Some(3), "{}: no reuse below the high-water mark", L::TAG);
    let b = L::add(&root, "B", 2);
    lib.rescan();
    assert_eq!(lib.slot_of(&b), sb, "{}: a returning item gets its slot back", L::TAG);
    let _ = std::fs::remove_dir_all(&root);
}

/// A second reader sees another writer's index on `reload_if_changed`
/// (one stat otherwise), and an identical item is a duplicate with no slot.
fn reload_follows_the_index_stamp<L: Lib>() {
    let root = temp_root::<L>("reload");
    L::add(&root, "First", 1);
    let mut writer = L::open(&root);
    writer.rescan();
    let mut reader = L::open(&root);
    assert!(!reader.reload_if_changed(), "{}: nothing moved", L::TAG);
    assert_eq!(reader.entries().len(), 1);
    L::add(&root, "Second", 2);
    writer.rescan();
    assert!(reader.reload_if_changed(), "{}: the writer moved the stamp", L::TAG);
    assert_eq!(reader.entries().len(), 2);
    assert!(!reader.reload_if_changed());
    let _ = std::fs::remove_dir_all(&root);
}

/// Every index write waits for `library.lock`: a rescan that has work to
/// do blocks while another holder has the lock, then completes.
fn rescans_wait_for_the_lock<L: Lib>() {
    let root = temp_root::<L>("lock");
    L::add(&root, "Locked", 1);
    let lock_path = L::lock_file(&root);
    let holder = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .unwrap();
    holder.lock().unwrap();
    let hold = Duration::from_millis(300);
    let release = std::thread::spawn(move || {
        std::thread::sleep(hold);
        holder.unlock().unwrap();
    });
    let started = Instant::now();
    let mut lib = L::open(&root);
    lib.rescan();
    let waited = started.elapsed();
    release.join().unwrap();
    assert!(waited >= hold - Duration::from_millis(50), "{}: rescan did not wait ({waited:?})", L::TAG);
    assert_eq!(lib.entries().len(), 1);
    let _ = std::fs::remove_dir_all(&root);
}

macro_rules! conformance {
    ($($case:ident),* $(,)?) => {
        mod nam {
            $(#[test] fn $case() { super::$case::<super::Nam>() })*
        }
        mod kits {
            $(#[test] fn $case() { super::$case::<super::Kits>() })*
        }
    };
}

conformance!(
    find_resolves_unambiguously,
    slots_are_not_reused_and_return_home,
    reload_follows_the_index_stamp,
    rescans_wait_for_the_lock,
);
