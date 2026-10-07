//! Installing voicebanks from the plok.org index over the wire
//! (vocal-mcp-control-and-quality.md §5, slice 7): `vocal.voicebanks`
//! lists the index's banks as `available` until one is installed;
//! `vocal.voicebank_install` is a job that downloads the bank (and the
//! shared vocoder it names), resumes a cut download, refuses a file whose
//! sha256 is not the index's, and carries the terms in its result. No
//! undo entry, no revision bump.
//!
//! Every test runs its own loopback server and its own voicebank folders,
//! so nothing touches the network or the user's data dir.

use std::path::Path;
use std::time::Duration;

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

/// A test app on private voicebank / vocoder folders under `root`.
fn app_on(root: &Path) -> Resonance {
    let (mut app, _task) = Resonance::new_for_test_on(ViewMode::Arrange);
    app.test_set_voicebank_roots(VoicebankRoots {
        banks: Some(root.join("voicebanks")),
        vocoders: Some(root.join("vocoders")),
        dev_models: Vec::new(),
    });
    app
}

/// A zipped fixture bank that names the shared vocoder instead of
/// bundling one, inside a wrapper directory.
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

struct Fixture {
    server: PlokServer,
    bank: Vec<u8>,
}

/// A server with one bank (`fixbank`, pinned by sha256) and the vocoder it
/// needs in its index.
fn serve_index(bank_sha: Option<String>) -> Fixture {
    let server = PlokServer::start();
    let bank = bank_zip("Fixbank");
    let vocoder = vocoder_zip(SHARED_VOCODER);
    let index = json!({
        "drumkits": [],
        "voicebanks": [{
            "id": "fixbank",
            "name": "Fixbank",
            "version": "v1",
            "languages": ["en"],
            "speakers": [],
            "size": "1 KB",
            "bytes": bank.len(),
            "sha256": bank_sha.unwrap_or_else(|| sha256_hex(&bank)),
            "url": server.url("/upstream/Fixbank_v1.zip"),
            "terms_url": "https://example.invalid/fixbank/terms",
            "credit": "Fixbank by Test Creator",
            "commercial": "free_with_credit",
            "vocoder": SHARED_VOCODER,
            "added": "2026-10-07"
        }],
        "vocoders": [{
            "id": SHARED_VOCODER,
            "name": "Fixture HiFiGAN",
            "version": "1",
            "bytes": vocoder.len(),
            "sha256": sha256_hex(&vocoder),
            "url": server.url("/upstream/vocoder.oudep"),
            "terms_url": "https://example.invalid/vocoder/terms",
            "credit": "Fixture vocoder",
            "commercial": "permission"
        }]
    });
    server.put("/index.json", serde_json::to_vec(&index).unwrap());
    server.put("/upstream/Fixbank_v1.zip", bank.clone());
    server.put("/upstream/vocoder.oudep", vocoder);
    Fixture { server, bank }
}

fn voicebanks(app: &mut Resonance) -> proto::VoicebankList {
    call(app, proto::VOICEBANKS, Value::Null)
        .result()
        .expect("vocal.voicebanks")
}

fn install(app: &mut Resonance, id: &str) -> JobStatus {
    let started: JobStarted = call(app, proto::VOICEBANK_INSTALL, json!({ "id": id }))
        .result()
        .expect("vocal.voicebank_install starts");
    assert!(app.test_settle_voicebank_library(SETTLE), "the install settles");
    app.control_jobs()
        .status(started.job_id.0)
        .expect("the job is known")
}

fn result_of(status: &JobStatus) -> proto::VoicebankInstallResult {
    assert_eq!(status.state, JobState::Done, "{status:?}");
    serde_json::from_value(status.result.clone().expect("a result")).expect("result shape")
}

/// `.part` files or staging folders left behind in `dir`.
fn leftovers(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains(".part") || n.contains(".staging") || n.contains(".meta"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn available_lists_index_banks_then_install_moves_them_to_installed() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = serve_index(None);
    let mut app = app_on(tmp.path());
    app.test_set_voicebank_index(&fx.server.url("/index.json"));

    // The first call starts the fetch off the update loop.
    let first = voicebanks(&mut app);
    assert!(first.available.is_empty());
    assert_eq!(first.index_status.as_deref(), Some("fetching"));
    assert!(app.test_settle_voicebank_library(SETTLE));
    let list = voicebanks(&mut app);
    assert_eq!(list.index_status.as_deref(), Some("fresh"));
    let entry = list
        .available
        .iter()
        .find(|b| b.id == "fixbank")
        .expect("fixbank is available");
    assert_eq!(entry.credit.as_deref(), Some("Fixbank by Test Creator"));
    assert_eq!(entry.commercial.as_deref(), Some("free_with_credit"));
    assert_eq!(entry.vocoder.as_deref(), Some(SHARED_VOCODER));
    assert!(list.installed.iter().all(|b| b.id != "fixbank"));

    let result = result_of(&install(&mut app, "fixbank"));
    assert_eq!(result.id, "fixbank");

    let after = voicebanks(&mut app);
    assert!(after.available.iter().all(|b| b.id != "fixbank"));
    let bank = after
        .installed
        .iter()
        .find(|b| b.id == "fixbank")
        .expect("fixbank is installed");
    assert!(bank.vocoder_installed, "{bank:?}");
    assert_eq!(bank.vocoder.as_deref(), Some(SHARED_VOCODER));
    // The terms the install accepted are the bank's from now on.
    assert_eq!(bank.credit.as_deref(), Some("Fixbank by Test Creator"));
    assert_eq!(bank.terms_url.as_deref(), Some("https://example.invalid/fixbank/terms"));
    assert_eq!(bank.version.as_deref(), Some("v1"));
}

#[test]
fn install_fetches_the_named_vocoder_and_reports_the_terms() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = serve_index(None);
    let mut app = app_on(tmp.path());
    app.test_set_voicebank_index(&fx.server.url("/index.json"));
    let revision = app.revision();
    let undo = app.test_can_undo();

    let result = result_of(&install(&mut app, "fixbank"));
    assert_eq!(result.terms_url.as_deref(), Some("https://example.invalid/fixbank/terms"));
    assert_eq!(result.credit.as_deref(), Some("Fixbank by Test Creator"));
    assert_eq!(result.commercial.as_deref(), Some("free_with_credit"));
    assert_eq!(result.vocoder.as_deref(), Some(SHARED_VOCODER));
    assert!(result.vocoder_installed);
    assert!(result.vocoder_fetched, "the vocoder came from the index too");
    assert!(!result.already_installed);

    // Unpacked without its wrapper directory or the macOS litter.
    let bank_dir = tmp.path().join("voicebanks/fixbank");
    assert!(bank_dir.join("dsconfig.yaml").is_file());
    assert!(!bank_dir.join("__MACOSX").exists());
    let vocoder_dir = tmp.path().join("vocoders").join(SHARED_VOCODER);
    assert!(vocoder_dir.join("vocoder.yaml").is_file());
    assert!(app.test_installed_voicebanks().contains(&"fixbank".to_owned()));
    assert!(leftovers(&tmp.path().join("voicebanks")).is_empty());

    // Machine state, not the project's.
    assert_eq!(app.revision(), revision, "no revision bump");
    assert_eq!(app.test_can_undo(), undo, "no undo entry");

    // Again: nothing to download.
    let again = result_of(&install(&mut app, "fixbank"));
    assert!(again.already_installed);
    assert!(!again.vocoder_fetched);
    assert_eq!(fx.server.requests("/upstream/Fixbank_v1.zip").len(), 1);
}

#[test]
fn a_changed_upstream_file_installs_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = serve_index(Some("0".repeat(64)));
    let mut app = app_on(tmp.path());
    app.test_set_voicebank_index(&fx.server.url("/index.json"));

    let status = install(&mut app, "fixbank");
    assert_eq!(status.state, JobState::Error, "{status:?}");
    let error = status.error.expect("an error");
    assert!(
        error.message.contains("upstream file changed; the index needs updating"),
        "{}",
        error.message
    );
    assert!(!tmp.path().join("voicebanks/fixbank").exists());
    assert!(!app.test_installed_voicebanks().contains(&"fixbank".to_owned()));
    assert!(leftovers(&tmp.path().join("voicebanks")).is_empty());
}

#[test]
fn a_cut_download_resumes_where_it_stopped() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = serve_index(None);
    let mut app = app_on(tmp.path());
    app.test_set_voicebank_index(&fx.server.url("/index.json"));
    let cut = fx.bank.len() / 2;
    fx.server
        .script("/upstream/Fixbank_v1.zip", vec![Act::CutAfter(cut)]);

    let first = install(&mut app, "fixbank");
    assert_eq!(first.state, JobState::Error, "the cut transfer fails");
    assert!(!app.test_installed_voicebanks().contains(&"fixbank".to_owned()));

    let second = result_of(&install(&mut app, "fixbank"));
    assert_eq!(second.id, "fixbank");
    let requests = fx.server.requests("/upstream/Fixbank_v1.zip");
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].range.as_deref(), Some(format!("bytes={cut}-").as_str()));
    assert_eq!(requests[1].status, 206, "only the rest was fetched");
    assert!(app.test_installed_voicebanks().contains(&"fixbank".to_owned()));
}

#[test]
fn an_id_the_index_does_not_list_is_not_found() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = serve_index(None);
    let mut app = app_on(tmp.path());
    app.test_set_voicebank_index(&fx.server.url("/index.json"));

    // Before the index is known the job finds out itself.
    let status = install(&mut app, "nosuchbank");
    assert_eq!(status.state, JobState::Error);
    assert_eq!(status.error.unwrap().kind, Some(ErrorKind::NotFound));

    // Once it is known, the call refuses up front.
    let err = call(&mut app, proto::VOICEBANK_INSTALL, json!({ "id": "nosuchbank" }))
        .result::<Value>()
        .expect_err("refused");
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.to_string().contains("fixbank"), "{err}");
}

#[test]
fn install_needs_no_open_project() {
    let tmp = tempfile::tempdir().unwrap();
    let fx = serve_index(None);
    let mut app = app_on(tmp.path());
    app.test_set_active_project(false);
    app.test_set_voicebank_index(&fx.server.url("/index.json"));
    let result = result_of(&install(&mut app, "fixbank"));
    assert_eq!(result.id, "fixbank");
}
