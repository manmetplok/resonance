//! The voicebank library's guards (vocal-mcp-control-and-quality review
//! findings): index ids that are not plain folder names are refused, two
//! installs that need one vocoder fetch it once, a finished install's
//! control job still settles when the same bank is installed again before
//! the tick, an index that cannot be fetched is a failure (not "no such
//! bank"), leftovers of interrupted installs are swept, and an install
//! record is only trusted for the folder (and, on a built-in bank, the
//! index file) it names.
//!
//! Every test runs its own loopback server and its own voicebank folders.

use std::path::{Path, PathBuf};
use std::time::Duration;

use resonance_app::compose::vocal_svs::registry::{InstallPhase, PlokIndex, PlokSidecar};
use resonance_app::compose::vocal_svs::VoicebankRoots;
use resonance_app::state::ViewMode;
use resonance_app::{write_voicebank_fixture, Resonance, VoicebankFixture};
use resonance_control::job::{JobStarted, JobState, JobStatus};
use resonance_control::methods::vocal as proto;
use resonance_control::{ErrorKind, Request, Response};
use serde_json::{json, Value};

use crate::common::plok_server::{sha256_hex, vocoder_zip, zip_dir, Act, PlokServer};
use crate::common::roundtrip;

const SHARED_VOCODER: &str = "fixture_shared_hifigan";
const SETTLE: Duration = Duration::from_secs(20);

fn call(app: &mut Resonance, method: &str, params: Value) -> Response {
    roundtrip(app, Request::new(1, method, &params).expect("params"))
}

fn roots(root: &Path) -> VoicebankRoots {
    VoicebankRoots {
        banks: Some(root.join("voicebanks")),
        vocoders: Some(root.join("vocoders")),
        dev_models: Vec::new(),
    }
}

fn app_on(root: &Path) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_voicebank_roots(roots(root));
    app
}

/// A zipped fixture bank naming the shared vocoder.
fn bank_zip(name: &str) -> Vec<u8> {
    let tmp = tempfile::tempdir().unwrap();
    let spec = VoicebankFixture {
        vocoder: false,
        vocoder_name: Some(SHARED_VOCODER.to_owned()),
        name: Some(name.to_owned()),
        ..VoicebankFixture::default()
    };
    write_voicebank_fixture(tmp.path(), &spec);
    zip_dir(tmp.path(), &format!("{name}_v1"))
}

fn bank_entry(server: &PlokServer, id: &str, zip: &[u8]) -> Value {
    json!({
        "id": id,
        "name": id,
        "version": "v1",
        "languages": ["en"],
        "bytes": zip.len(),
        "sha256": sha256_hex(zip),
        "url": server.url(&format!("/upstream/{id}.zip")),
        "terms_url": "https://example.invalid/terms",
        "credit": format!("{id} by Test Creator"),
        "commercial": "free_with_credit",
        "vocoder": SHARED_VOCODER
    })
}

/// A server whose index lists `ids` (each a zipped bank needing the
/// shared vocoder) plus `extra` raw entries, and the vocoder.
fn serve(ids: &[&str], extra: Vec<Value>) -> PlokServer {
    let server = PlokServer::start();
    let mut banks: Vec<Value> = ids
        .iter()
        .map(|id| {
            let zip = bank_zip(id);
            server.put(&format!("/upstream/{id}.zip"), zip.clone());
            bank_entry(&server, id, &zip)
        })
        .collect();
    banks.extend(extra);
    let vocoder = vocoder_zip(SHARED_VOCODER);
    let index = json!({
        "voicebanks": banks,
        "vocoders": [{
            "id": SHARED_VOCODER,
            "name": "Fixture HiFiGAN",
            "bytes": vocoder.len(),
            "sha256": sha256_hex(&vocoder),
            "url": server.url("/upstream/vocoder.oudep"),
        }, {
            "id": "../evil_vocoder",
            "name": "Evil",
            "url": server.url("/upstream/vocoder.oudep"),
        }]
    });
    server.put("/index.json", serde_json::to_vec(&index).unwrap());
    server.put("/upstream/vocoder.oudep", vocoder);
    server
}

fn start_install(app: &mut Resonance, id: &str) -> u64 {
    let started: JobStarted = call(app, proto::VOICEBANK_INSTALL, json!({ "id": id }))
        .result()
        .expect("vocal.voicebank_install starts");
    started.job_id.0
}

fn status(app: &Resonance, job: u64) -> JobStatus {
    app.control_jobs().status(job).expect("the job is known")
}

fn result_of(status: &JobStatus) -> proto::VoicebankInstallResult {
    assert_eq!(status.state, JobState::Done, "{status:?}");
    serde_json::from_value(status.result.clone().expect("a result")).expect("result shape")
}

fn voicebanks(app: &mut Resonance) -> proto::VoicebankList {
    call(app, proto::VOICEBANKS, Value::Null)
        .result()
        .expect("vocal.voicebanks")
}

/// Every path under `dir`, relative.
fn tree(dir: &Path) -> Vec<PathBuf> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            out.push(p.strip_prefix(base).unwrap().to_path_buf());
            if p.is_dir() {
                walk(base, &p, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

const BAD_IDS: [&str; 5] = ["../x", "/abs", "a/b", ".hidden", ".."];

#[test]
fn index_ids_that_are_not_folder_names_are_dropped_and_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let library = tmp.path().join("library");
    let server = PlokServer::start();
    let evil = bank_zip("Evil");
    server.put("/upstream/evil.zip", evil.clone());
    let extra: Vec<Value> = BAD_IDS
        .iter()
        .map(|id| {
            let mut e = bank_entry(&server, "evil", &evil);
            e["id"] = json!(id);
            e
        })
        .collect();
    let index = json!({ "voicebanks": extra, "vocoders": [] });
    server.put("/index.json", serde_json::to_vec(&index).unwrap());

    // Parsed: none of them survive.
    let parsed: PlokIndex = serde_json::from_value(index.clone()).unwrap();
    assert!(parsed.voicebanks.is_empty(), "{:?}", parsed.voicebanks);

    let mut app = app_on(&library);
    app.test_set_voicebank_index(&server.url("/index.json"));
    let _ = voicebanks(&mut app);
    assert!(app.test_settle_voicebank_library(SETTLE));
    assert!(voicebanks(&mut app).available.is_empty());

    for id in BAD_IDS {
        // Over the wire: refused up front as not_found.
        let err = call(&mut app, proto::VOICEBANK_INSTALL, json!({ "id": id }))
            .result::<Value>()
            .expect_err("refused");
        assert_eq!(err.kind(), ErrorKind::NotFound, "{id}: {err}");
        // The library itself refuses too.
        assert!(app
            .test_voicebank_library()
            .start_install(id, roots(&library), None)
            .is_err());
    }
    assert!(app.test_settle_voicebank_library(SETTLE));
    assert!(server.requests("/upstream/evil.zip").is_empty());
    // Nothing was written anywhere but the library's own folders.
    let outside: Vec<PathBuf> = tree(tmp.path())
        .into_iter()
        .filter(|p| !p.starts_with("library"))
        .collect();
    assert!(outside.is_empty(), "{outside:?}");
    assert!(!library.join("voicebanks/evil").exists());
}

#[test]
fn delete_refuses_a_folder_whose_name_is_not_an_id() {
    use resonance_app::compose::vocal_svs::registry::library::delete_user_bank;
    let tmp = tempfile::tempdir().unwrap();
    let banks = tmp.path().join("voicebanks");
    let hidden = banks.join(".hidden");
    std::fs::create_dir_all(&hidden).unwrap();
    let outside = tmp.path().join("precious");
    std::fs::create_dir_all(&outside).unwrap();
    assert!(delete_user_bank(&banks, &hidden).is_err());
    assert!(delete_user_bank(&banks, &banks.join("..").join("precious")).is_err());
    assert!(delete_user_bank(&banks, &outside).is_err());
    assert!(hidden.is_dir() && outside.is_dir());
}

#[test]
fn two_installs_needing_one_vocoder_fetch_it_once() {
    let tmp = tempfile::tempdir().unwrap();
    let server = serve(&["fixbank", "otherbank"], Vec::new());
    // A slow vocoder: the second install reaches it while the first is
    // still fetching.
    server.script("/upstream/vocoder.oudep", vec![Act::DelayMs(600)]);
    let mut app = app_on(tmp.path());
    app.test_set_voicebank_index(&server.url("/index.json"));

    let a = start_install(&mut app, "fixbank");
    let b = start_install(&mut app, "otherbank");
    assert!(app.test_settle_voicebank_library(SETTLE));
    let (ra, rb) = (result_of(&status(&app, a)), result_of(&status(&app, b)));
    assert!(ra.vocoder_installed && rb.vocoder_installed, "{ra:?} {rb:?}");
    assert_eq!(
        u8::from(ra.vocoder_fetched) + u8::from(rb.vocoder_fetched),
        1,
        "exactly one install fetched the vocoder"
    );
    assert_eq!(server.requests("/upstream/vocoder.oudep").len(), 1);
    let installed = app.test_installed_voicebanks();
    assert!(installed.contains(&"fixbank".into()) && installed.contains(&"otherbank".into()));
    for list in voicebanks(&mut app).installed {
        assert!(list.vocoder_installed, "{list:?}");
    }
    for dir in ["voicebanks", "vocoders"] {
        let left: Vec<String> = std::fs::read_dir(tmp.path().join(dir))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with('.') && n != ".plok-index.json")
            .collect();
        assert!(left.is_empty(), "{dir}: {left:?}");
    }
}

#[test]
fn a_finished_install_still_settles_when_the_bank_is_installed_again_before_the_tick() {
    let tmp = tempfile::tempdir().unwrap();
    let server = serve(&["fixbank"], Vec::new());
    let mut app = app_on(tmp.path());
    app.test_set_voicebank_index(&server.url("/index.json"));

    let first = start_install(&mut app, "fixbank");
    // Wait for it to finish without running the tick that collects it.
    let deadline = std::time::Instant::now() + SETTLE;
    while !app.test_voicebank_library().has_finished() {
        assert!(std::time::Instant::now() < deadline, "the first install finishes");
        std::thread::sleep(Duration::from_millis(5));
    }
    let second = start_install(&mut app, "fixbank");
    assert!(app.test_settle_voicebank_library(SETTLE));
    let one = result_of(&status(&app, first));
    assert!(!one.already_installed);
    let two = result_of(&status(&app, second));
    assert!(two.already_installed, "{two:?}");
}

#[test]
fn an_index_that_cannot_be_fetched_fails_the_install_rather_than_not_finding_the_bank() {
    let tmp = tempfile::tempdir().unwrap();
    let server = PlokServer::start();
    let mut app = app_on(tmp.path());
    app.test_set_voicebank_index(&server.url("/no-such-index.json"));
    let job = start_install(&mut app, "fixbank");
    assert!(app.test_settle_voicebank_library(SETTLE));
    let s = status(&app, job);
    assert_eq!(s.state, JobState::Error, "{s:?}");
    let error = s.error.expect("an error");
    assert_ne!(error.kind, Some(ErrorKind::NotFound), "{error:?}");
    assert!(error.message.contains("plok.org index"), "{}", error.message);
}

/// A process id that is certainly not running.
#[cfg(unix)]
fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[cfg(unix)]
#[test]
fn the_sweep_clears_interrupted_installs_and_restores_a_cut_replace() {
    let tmp = tempfile::tempdir().unwrap();
    let roots = roots(tmp.path());
    let banks = roots.banks.clone().unwrap();
    let vocoders = roots.vocoders.clone().unwrap();
    let own = std::process::id();
    let dead = dead_pid();
    let mk = |p: PathBuf| {
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("file"), b"x").unwrap();
        p
    };
    let own_staging = mk(banks.join(format!(".fixbank.staging-{own}-aa")));
    let dead_staging = mk(banks.join(format!(".gone.staging-{dead}-bb")));
    let live_staging = mk(banks.join(".alive.staging-1-cc")); // pid 1 runs
    let legacy_staging = mk(banks.join(".legacy.staging-0123456789abcdef"));
    let deleting = mk(banks.join(format!(".zap.deleting-{own}-dd")));
    // A replace cut between its renames: the old install is aside and the
    // new one never landed.
    let cut = banks.join(format!(".lilia.old-{own}-ee"));
    write_voicebank_fixture(&cut, &VoicebankFixture::shipped("lilia"));
    // A replace that landed but whose old copy was never removed.
    let landed = mk(banks.join(format!(".aurora.old-{dead}-ff")));
    let aurora = mk(banks.join("aurora"));
    let vocoder_staging = mk(vocoders.join(format!(".{SHARED_VOCODER}.staging-{own}-gg")));
    let unrelated = mk(banks.join("notes"));
    std::fs::write(banks.join(".plok-index.json"), b"{}").unwrap();

    let mut app = app_on(tmp.path());
    // While an install of this process runs, its own leftovers stay.
    app.test_show_voicebank_installing("x", InstallPhase::Starting);
    app.test_voicebank_library().sweep_leftovers_now(&roots);
    assert!(own_staging.exists(), "a live install's staging was swept");
    assert!(!dead_staging.exists(), "a dead process's staging survived");
    assert!(live_staging.exists(), "another live process's staging was swept");
    let _ = app.test_voicebank_library().take_finished();
    app.test_voicebank_library().state().jobs.clear();

    app.test_voicebank_library().sweep_leftovers_now(&roots);
    for gone in [&own_staging, &legacy_staging, &deleting, &landed, &vocoder_staging] {
        assert!(!gone.exists(), "{} survived", gone.display());
    }
    assert!(live_staging.exists());
    assert!(!cut.exists());
    assert!(banks.join("lilia/dsconfig.yaml").is_file(), "the cut replace is put back");
    assert!(aurora.join("file").is_file() && unrelated.exists());
    assert!(banks.join(".plok-index.json").is_file());
    app.test_rescan_voicebanks();
    assert!(app.test_installed_voicebanks().contains(&"lilia".to_owned()));
}

fn write_record(dir: &Path, record: &PlokSidecar) {
    record.write(dir).unwrap();
}

fn record(id: &str, sha: Option<&str>) -> PlokSidecar {
    PlokSidecar {
        id: id.to_owned(),
        name: id.to_owned(),
        url: "https://example.invalid/x.zip".into(),
        sha256: sha.map(str::to_owned),
        credit: Some("Forged credit".into()),
        terms_url: Some("https://example.invalid/forged".into()),
        commercial: Some("free_with_credit".into()),
        ..PlokSidecar::default()
    }
}

#[test]
fn an_install_record_is_trusted_only_for_its_own_folder_and_a_pinned_builtin_file() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = app_on(tmp.path());
    let banks = tmp.path().join("voicebanks");
    // Meiji is a built-in bank whose terms ask for permission.
    let meiji = banks.join("meiji");
    write_voicebank_fixture(&meiji, &VoicebankFixture::shipped("meiji"));
    write_record(&meiji, &record("meiji", Some(&"a".repeat(64))));
    // A user bank carrying another bank's record, and one with its own.
    let copied = banks.join("copied");
    write_voicebank_fixture(&copied, &VoicebankFixture::default());
    write_record(&copied, &record("somethingelse", None));
    let own = banks.join("ownbank");
    write_voicebank_fixture(&own, &VoicebankFixture::default());
    write_record(&own, &record("ownbank", None));
    app.test_rescan_voicebanks();

    let entry = |app: &mut Resonance, id: &str| {
        voicebanks(app)
            .installed
            .into_iter()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("{id} is installed"))
    };
    // No index: the built-in table's terms win over the record.
    let m = entry(&mut app, "meiji");
    assert_eq!(m.commercial.as_deref(), Some("permission"));
    assert_eq!(m.credit.as_deref(), Some("Gahata Meiji from LUNAI Project"));
    // A record naming another id is ignored.
    assert_eq!(entry(&mut app, "copied").credit, None);
    assert_eq!(entry(&mut app, "ownbank").credit.as_deref(), Some("Forged credit"));

    // An index pinning another file: the built-in still wins.
    let server = PlokServer::start();
    let put_index = |sha: &str| {
        let index = json!({ "voicebanks": [{
            "id": "meiji", "name": "Meiji", "url": server.url("/m.zip"), "sha256": sha
        }]});
        server.put("/index.json", serde_json::to_vec(&index).unwrap());
    };
    put_index(&"b".repeat(64));
    app.test_set_voicebank_index(&server.url("/index.json"));
    let _ = voicebanks(&mut app);
    assert!(app.test_settle_voicebank_library(SETTLE));
    assert_eq!(entry(&mut app, "meiji").commercial.as_deref(), Some("permission"));

    // The index pins the file the record names: the accepted terms win.
    put_index(&"A".repeat(64));
    app.test_set_voicebank_index(&server.url("/index.json"));
    let _ = voicebanks(&mut app);
    assert!(app.test_settle_voicebank_library(SETTLE));
    let m = entry(&mut app, "meiji");
    assert_eq!(m.commercial.as_deref(), Some("free_with_credit"));
    assert_eq!(m.credit.as_deref(), Some("Forged credit"));
}
