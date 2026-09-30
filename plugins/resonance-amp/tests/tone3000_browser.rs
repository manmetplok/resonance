//! Tone3000 browser: query construction, architecture filter, and
//! pagination (ba todo #1315).
//!
//! No network is touched. The request-building and response-folding
//! logic is pure, and the JSON below is shaped after real
//! `/api/v1/tones/search` and `/api/v1/models` bodies, so the decode
//! path is covered too.

use resonance_amp::editor::tone3000_panel::{model_row_action, tones_heading, ModelRowAction};
use resonance_amp::tone3000::auth::build_authorize_url;
use resonance_amp::tone3000::client::{
    merge_model_pages, merge_search_pages, models_query_params, search_query_params,
    ArchitectureFilter,
};
use resonance_amp::tone3000::types::{Model, PaginatedResponse, Tone};
use resonance_amp::tone3000::worker::{apply_search_page, sanitize_filename, sidecar_for, State};

/// Recorded-shape search response. Field set matches what the API
/// returns; extra keys are present on purpose so the "ignore unknown
/// fields" behaviour stays covered.
fn search_body(page: u32, total: u32, total_pages: u32, ids: &[i64]) -> String {
    let rows: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r#"{{"id":{id},"title":"Tone {id}","description":"d","gear":"amp",
                    "platform":"nam","architecture":"1","models_count":3,
                    "downloads_count":42,"favorites_count":7,
                    "user":{{"id":9,"username":"someone"}}}}"#
            )
        })
        .collect();
    format!(
        r#"{{"data":[{}],"page":{page},"page_size":25,"total":{total},"total_pages":{total_pages}}}"#,
        rows.join(",")
    )
}

fn decode_search(json: &str) -> PaginatedResponse<Tone> {
    serde_json::from_str(json).expect("search body decodes")
}

fn models_body(ids: &[i64]) -> String {
    let rows: Vec<String> = ids
        .iter()
        .map(|id| {
            format!(
                r#"{{"id":{id},"tone_id":7,"name":"model {id}","size":"standard",
                    "model_url":"https://example.invalid/{id}.nam"}}"#
            )
        })
        .collect();
    format!(
        r#"{{"data":[{}],"page":1,"page_size":100,"total":{},"total_pages":1}}"#,
        rows.join(","),
        ids.len()
    )
}

fn decode_models(json: &str) -> PaginatedResponse<Model> {
    serde_json::from_str(json).expect("models body decodes")
}

fn param<'a>(params: &'a [(&'static str, String)], key: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.as_str())
}

// ---------------------------------------------------------------- filter

#[test]
fn default_filter_is_all_architectures() {
    assert_eq!(ArchitectureFilter::default(), ArchitectureFilter::All);
}

#[test]
fn all_filter_covers_a1_custom_and_a2() {
    // The API takes a single architecture value, and omitting it means
    // "A1 + custom". So covering everything is exactly two requests:
    // the legacy default plus an explicit A2 pass.
    assert_eq!(
        ArchitectureFilter::All.query_values(),
        &[None, Some("2")][..]
    );
}

#[test]
fn narrow_filters_pin_exactly_one_architecture() {
    assert_eq!(ArchitectureFilter::A1.query_values(), &[Some("1")][..]);
    assert_eq!(ArchitectureFilter::A2.query_values(), &[Some("2")][..]);
    assert_eq!(
        ArchitectureFilter::Custom.query_values(),
        &[Some("custom")][..]
    );
}

#[test]
fn every_filter_has_a_distinct_label() {
    let mut labels: Vec<&str> = ArchitectureFilter::ALL.iter().map(|a| a.label()).collect();
    let count = labels.len();
    labels.sort_unstable();
    labels.dedup();
    assert_eq!(labels.len(), count, "labels must be unique: {labels:?}");
}

// ------------------------------------------------------------ query build

#[test]
fn search_query_omits_architecture_when_unfiltered() {
    // The regression this todo is about: the browser used to pin
    // `architecture=2` unconditionally, hiding most of the catalogue.
    let params = search_query_params("plexi", "trending", None, 1);
    assert!(
        param(&params, "architecture").is_none(),
        "architecture must be absent, got {params:?}"
    );
    assert_eq!(param(&params, "query"), Some("plexi"));
    assert_eq!(param(&params, "sort"), Some("trending"));
    assert_eq!(param(&params, "format"), Some("nam"));
    // Deliberate, recorded choice: this plugin plays amp/rig captures.
    assert_eq!(param(&params, "gears"), Some("amp_amp-cab"));
}

#[test]
fn search_query_sends_the_requested_page() {
    let params = search_query_params("", "newest", Some("1"), 4);
    assert_eq!(param(&params, "page"), Some("4"));
    assert_eq!(param(&params, "architecture"), Some("1"));
    // 25 is the documented maximum for /tones/search.
    assert_eq!(param(&params, "page_size"), Some("25"));
}

#[test]
fn search_query_clamps_page_zero_to_one() {
    let params = search_query_params("", "trending", None, 0);
    assert_eq!(param(&params, "page"), Some("1"));
}

#[test]
fn models_query_carries_tone_and_optional_architecture() {
    let unfiltered = models_query_params(42, None);
    assert_eq!(param(&unfiltered, "tone_id"), Some("42"));
    assert!(param(&unfiltered, "architecture").is_none());

    let a2 = models_query_params(42, Some("2"));
    assert_eq!(param(&a2, "architecture"), Some("2"));
}

// ----------------------------------------------------------------- merge

#[test]
fn merge_sums_totals_and_keeps_first_seen_order() {
    let merged = merge_search_pages(
        vec![
            decode_search(&search_body(1, 100, 4, &[1, 2, 3])),
            decode_search(&search_body(1, 40, 2, &[4, 5])),
        ],
        1,
    );
    assert_eq!(
        merged.tones.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(merged.total, Some(140));
    // Walking every architecture takes as many pages as the deepest one.
    assert_eq!(merged.total_pages, Some(4));
    assert_eq!(merged.page, 1);
}

#[test]
fn merge_drops_tones_returned_by_more_than_one_architecture() {
    let merged = merge_search_pages(
        vec![
            decode_search(&search_body(2, 10, 1, &[1, 2])),
            decode_search(&search_body(2, 10, 1, &[2, 3])),
        ],
        2,
    );
    assert_eq!(
        merged.tones.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(merged.page, 2);
}

#[test]
fn merge_tolerates_missing_pagination_fields() {
    let body = r#"{"data":[{"id":1}]}"#;
    let merged = merge_search_pages(vec![decode_search(body)], 1);
    assert_eq!(merged.tones.len(), 1);
    assert_eq!(merged.total, None);
    assert_eq!(merged.total_pages, None);
    assert_eq!(merged.tones[0].display_title(), "(untitled)");
    assert_eq!(merged.tones[0].display_author(), "unknown");
}

#[test]
fn model_merge_dedupes_across_architectures() {
    let models = merge_model_pages(vec![
        decode_models(&models_body(&[11, 12])),
        decode_models(&models_body(&[12, 13])),
    ]);
    assert_eq!(
        models.iter().map(|m| m.id).collect::<Vec<_>>(),
        vec![11, 12, 13]
    );
    assert_eq!(models[0].display_label(), "model 11 (standard)");
}

// ------------------------------------------------------------ pagination

#[test]
fn first_page_replaces_and_reports_more_available() {
    let mut state = State::default();
    let page = merge_search_pages(vec![decode_search(&search_body(1, 60, 3, &[1, 2, 3]))], 1);
    apply_search_page(&mut state, page, false);

    assert_eq!(state.tones.len(), 3);
    assert_eq!(state.total_tones, Some(60));
    assert_eq!(state.page, 1);
    assert!(state.has_more, "3 pages reported, we have page 1");
}

#[test]
fn load_more_appends_and_stops_on_the_last_page() {
    let mut state = State::default();
    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(&search_body(1, 5, 2, &[1, 2, 3]))], 1),
        false,
    );
    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(&search_body(2, 5, 2, &[4, 5]))], 2),
        true,
    );

    assert_eq!(
        state.tones.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![1, 2, 3, 4, 5]
    );
    assert_eq!(state.page, 2);
    assert!(!state.has_more, "page 2 of 2 is the end");
}

#[test]
fn load_more_never_duplicates_a_row_already_on_screen() {
    let mut state = State::default();
    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(&search_body(1, 9, 3, &[1, 2]))], 1),
        false,
    );
    // The server re-served id 2 on the next page (ranking shifted).
    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(&search_body(2, 9, 3, &[2, 3]))], 2),
        true,
    );

    assert_eq!(
        state.tones.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[test]
fn a_fresh_search_clears_the_previous_selection() {
    let mut state = State::default();
    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(&search_body(1, 2, 1, &[1, 2]))], 1),
        false,
    );
    state.selected_tone = Some(1);
    state.models = decode_models(&models_body(&[11])).data;

    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(&search_body(1, 1, 1, &[9]))], 1),
        false,
    );

    assert_eq!(state.tones.iter().map(|t| t.id).collect::<Vec<_>>(), vec![9]);
    assert_eq!(state.selected_tone, None);
    assert!(state.models.is_empty());
    assert!(!state.has_more);
}

#[test]
fn without_total_pages_the_running_count_decides() {
    let mut state = State::default();
    let body = r#"{"data":[{"id":1},{"id":2}],"page":1,"total":5}"#;
    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(body)], 1),
        false,
    );
    assert!(state.has_more, "2 of 5 shown");

    let body2 = r#"{"data":[{"id":3},{"id":4},{"id":5}],"page":2,"total":5}"#;
    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(body2)], 2),
        true,
    );
    assert!(!state.has_more, "5 of 5 shown");
}

#[test]
fn an_empty_page_with_no_totals_ends_pagination() {
    let mut state = State::default();
    let body = r#"{"data":[],"page":1}"#;
    apply_search_page(
        &mut state,
        merge_search_pages(vec![decode_search(body)], 1),
        false,
    );
    assert!(!state.has_more);
    assert!(state.tones.is_empty());
}

// -------------------------------------------------------- authorize URL

/// The pin that survived the first pass at this todo. `architecture=2`
/// lived in `build_authorize_url` as a bare literal rather than via the
/// `ARCHITECTURE_A2` constant, so lifting the pin from the two API
/// endpoints — and grepping the constant to check — missed it entirely.
/// This is the search-path assertion above, mirrored onto the one
/// request that has no other test.
#[test]
fn authorize_url_omits_architecture() {
    let url = build_authorize_url("http://localhost:47834/", "chal", "st8");
    assert!(
        !url.contains("architecture"),
        "authorize URL must not pin an architecture, got {url}"
    );
}

#[test]
fn authorize_url_keeps_the_oauth_parameters_and_gear_scope() {
    let url = build_authorize_url("http://localhost:47834/", "chal", "st8");
    // PKCE + CSRF essentials, percent-encoded redirect included.
    assert!(
        url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A47834%2F"),
        "{url}"
    );
    assert!(url.contains("response_type=code"), "{url}");
    assert!(url.contains("code_challenge=chal"), "{url}");
    assert!(url.contains("code_challenge_method=S256"), "{url}");
    assert!(url.contains("state=st8"), "{url}");
    // Same deliberate, recorded scope as the search query.
    assert!(url.contains("gears=amp_amp-cab"), "{url}");
    assert!(url.contains("format=nam"), "{url}");
    // No stray whitespace from the multi-line format! continuation.
    assert!(!url.contains(' '), "{url}");
}

// -------------------------------------------------------------- heading

#[test]
fn heading_shows_the_real_total_not_just_what_is_loaded() {
    assert_eq!(tones_heading(25, Some(1284)), "Tones (25 of 1284)");
    assert_eq!(tones_heading(12, Some(12)), "Tones (12)");
    assert_eq!(tones_heading(3, None), "Tones (3)");
}

// ------------------------------------------------------- library (L1)

fn one_model(id: i64, tone_id: i64) -> Model {
    serde_json::from_str(&format!(
        r#"{{"id":{id},"tone_id":{tone_id},"name":"BE100","size":"standard","model_url":"https://example.invalid/x.nam"}}"#
    ))
    .unwrap()
}

#[test]
fn download_file_names_are_unchanged_by_the_library() {
    // Existing downloads keep their names, so the migration's slot order
    // and every stored path stay valid.
    assert_eq!(
        sanitize_filename("Friedman BE100 (standard)", 48121),
        "Friedman_BE100_standard_48121.nam"
    );
    assert_eq!(sanitize_filename("Ünïcode!", 7), "ncode_7.nam");
    assert_eq!(sanitize_filename("", 9), "model_9.nam");
}

#[test]
fn a_download_sidecar_carries_the_tone_metadata_the_file_lacks() {
    let tone: Tone = serde_json::from_str(
        r#"{"id":1934,"title":"Friedman BE-100","gear":"amp","user":{"username":"jsmith"}}"#,
    )
    .unwrap();
    let model = one_model(48121, 1934);
    let sc = sidecar_for(&model, Some(&tone), 1_790_000_000);
    assert_eq!(sc.source, "tone3000");
    assert_eq!((sc.tone_id, sc.model_id), (Some(1934), Some(48121)));
    assert_eq!(sc.tone_title.as_deref(), Some("Friedman BE-100"));
    assert_eq!(sc.author.as_deref(), Some("jsmith"));
    assert_eq!(sc.size.as_deref(), Some("standard"));
    assert_eq!(sc.model_name.as_deref(), Some("BE100"));
    assert!(sc.downloaded_at.unwrap().starts_with("2026-"));

    // The tone may have scrolled out of the list: the ids still make it.
    let bare = sidecar_for(&model, None, 0);
    assert_eq!(bare.model_id, Some(48121));
    assert_eq!(bare.tone_title, None);
}

#[test]
fn an_installed_model_row_offers_load_instead_of_download() {
    let root =
        std::env::temp_dir().join(format!("resonance-amp-t3k-rows-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let dl = root.join("tone3000");
    std::fs::create_dir_all(&dl).unwrap();
    let path = dl.join("BE100_standard_48121.nam");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/a1/wavenet.nam"),
        &path,
    )
    .unwrap();
    resonance_common::nam_library::write_sidecar(
        &path,
        &sidecar_for(&one_model(48121, 1934), None, 0),
    )
    .unwrap();
    let lib = resonance_common::nam_library::Library::open_and_scan(&root).unwrap();

    assert_eq!(
        model_row_action(&one_model(48121, 1934), &lib),
        ModelRowAction::Load {
            path: path.clone(),
            slot: Some(0)
        }
    );
    assert_eq!(model_row_action(&one_model(1, 1934), &lib), ModelRowAction::Download);
    let _ = std::fs::remove_dir_all(&root);
}
