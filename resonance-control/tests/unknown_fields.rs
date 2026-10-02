//! Request params refuse fields they do not know (code review ARCH2-06).
//!
//! A misspelled or wrong-surface field (`beats` for `beat`) used to be
//! dropped by serde, and the call was acknowledged as a success that did
//! nothing. Every `*Params` type is `#[serde(deny_unknown_fields)]`;
//! results and views stay tolerant, so an older client still reads a
//! newer app's replies.

use std::path::Path;

use resonance_control::methods::{automation, presets, track, transport};
use resonance_control::{ErrorKind, Request};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

/// Every `pub struct …Params` in the crate's sources that derives
/// `Deserialize`, with whether its attributes carry `deny_unknown_fields`.
fn params_types() -> Vec<(String, bool)> {
    fn walk(dir: &Path, out: &mut Vec<(String, bool)>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let Some(rest) = line.trim_start().strip_prefix("pub struct ") else {
                    continue;
                };
                let name: String = rest
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.ends_with("Params") {
                    continue;
                }
                let attrs: Vec<&str> = lines[..i]
                    .iter()
                    .rev()
                    .take_while(|l| {
                        let t = l.trim_start();
                        t.starts_with("#[") || t.starts_with("//")
                    })
                    .copied()
                    .collect();
                let attrs = attrs.join("\n");
                if attrs.contains("Deserialize") {
                    let file = path.file_name().unwrap().to_string_lossy();
                    out.push((format!("{file}::{name}"), attrs.contains("deny_unknown_fields")));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

#[test]
fn every_params_type_denies_unknown_fields() {
    let types = params_types();
    assert!(types.len() > 100, "found only {} params types", types.len());
    let tolerant: Vec<&str> = types
        .iter()
        .filter(|(_, deny)| !deny)
        .map(|(n, _)| n.as_str())
        .collect();
    assert!(
        tolerant.is_empty(),
        "request params must be #[serde(deny_unknown_fields)]: {tolerant:?}"
    );
}

/// `value` parses as `T` through the wire path (`Request::params`).
fn parses<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    let request = Request::new(1, "test.method", &value).unwrap();
    request.params::<T>().map_err(|e| {
        assert_eq!(e.kind(), ErrorKind::InvalidParams);
        e.message
    })
}

fn refuses<T: DeserializeOwned>(value: Value, field: &str) {
    let message = match parses::<T>(value.clone()) {
        Ok(_) => panic!("{value} was accepted"),
        Err(m) => m,
    };
    assert!(
        message.contains(&format!("unknown field `{field}`")),
        "{value}: the error names the field: {message}"
    );
}

#[test]
fn a_misspelled_field_is_refused_and_named() {
    refuses::<transport::SetTempoParams>(json!({"bpm": 120.0, "tempo": 1}), "tempo");
    refuses::<track::RenameParams>(json!({"track_id": 1, "name": "x", "colour": 3}), "colour");
}

/// `#[serde(flatten)]` params check the fields their flattened part
/// takes too: neither those nor the outer ones are reported unknown, and
/// anything else is.
#[test]
fn flattened_params_refuse_unknown_fields_and_keep_the_known_ones() {
    // The canonical case: `beats` for `beat` used to seek to bar 3, beat 1.
    refuses::<transport::SeekParams>(json!({"bar": 3, "beats": 2}), "beats");
    let seek: transport::SeekParams = parses(json!({"bar": 3, "beat": 2.0})).unwrap();
    assert_eq!(seek.position.beat, Some(2.0));

    refuses::<automation::LanesParams>(json!({"track_id": 1, "parm": "Cutoff"}), "parm");
    let lanes: automation::LanesParams =
        parses(json!({"track_id": 1, "param": "Cutoff", "range": {}})).unwrap();
    assert_eq!(lanes.target.param.as_deref(), Some("Cutoff"));

    refuses::<presets::SearchParams>(json!({"query": "pad", "favorites": true}), "favorites");
    let search: presets::SearchParams =
        parses(json!({"plugin_id": "a.b", "query": "pad", "favorites_only": true})).unwrap();
    assert!(search.filter.favorites_only);

    refuses::<track::PluginPresetsParams>(json!({"track_id": 1, "serch": "x"}), "serch");
    let list: track::PluginPresetsParams =
        parses(json!({"track_id": 1, "occurrence": 0, "limit": 5})).unwrap();
    assert_eq!(list.filter.limit, Some(5));
}

/// The request-only building blocks nested inside params are strict too:
/// `{"start": {"bars": 2}}` is refused, not read as the song start.
#[test]
fn nested_request_specs_refuse_unknown_fields() {
    refuses::<transport::LoopSetParams>(
        json!({"start": {"bar": 1}, "end": {"bars": 5}}),
        "bars",
    );
    refuses::<automation::LanesParams>(json!({"range": {"begin": {"bar": 1}}}), "begin");
}

#[test]
fn omitted_params_still_parse_for_all_optional_params() {
    let request = Request::without_params(1, presets::SEARCH);
    request
        .params::<presets::SearchParams>()
        .expect("omitted params are an empty object");
}
