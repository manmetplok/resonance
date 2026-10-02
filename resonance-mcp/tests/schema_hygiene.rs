//! Regression tests for the *published* MCP tool schemas (epic #200
//! follow-up).
//!
//! Two schema defects reached a live Claude Code client after the epic
//! landed, because the translation tests exercised request/result
//! mapping but never the published schemas themselves:
//!   1. a `serde_json::Value` field (`JobStatus.result`) rendered as the
//!      JSON-Schema boolean `true`, which strict clients reject
//!      ("Invalid input ... at tools.N.outputSchema.properties.result");
//!   2. schemars' Rust numeric `format`s (`uint64`, `uint32`, ...), which
//!      strict clients warn about and drop.
//!
//! These assert every published tool schema (input and output) is free of
//! both, so the surface stays standard-compliant in strict clients.
//!
//! A third (b109a46c, "mcp fix"; code review STATE2-09): some clients (the
//! Claude desktop bridge) forward a tool's schema without its `$defs`, so
//! a field typed only by a dangling `$ref` has no known type and its
//! value arrives as a string, `"42"` for an id, which serde refuses.
//! `server::inline_defs` inlines every ref; the tests below pin that no
//! input schema carries a `$ref`/`$defs` and that the id newtypes inline
//! to `"type": "integer"`.

use resonance_mcp::ResonanceMcp;
use serde_json::Value;

/// Rust numeric `format` values schemars emits that are not standard
/// JSON Schema (kept in sync with `server::NONSTANDARD_NUMERIC_FORMATS`).
const NONSTANDARD_NUMERIC_FORMATS: &[&str] = &[
    "uint", "uint8", "uint16", "uint32", "uint64", "uint128", "int", "int8", "int16", "int32",
    "int64", "int128", "float", "double",
];

/// Visit every node of a JSON value with its JSON-pointer-ish path.
fn walk(node: &Value, path: &str, visit: &mut impl FnMut(&str, &Value)) {
    visit(path, node);
    match node {
        Value::Object(map) => {
            for (k, v) in map {
                walk(v, &format!("{path}/{k}"), visit);
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                walk(v, &format!("{path}[{i}]"), visit);
            }
        }
        _ => {}
    }
}

/// Every published tool schema, labelled `"<tool>.inputSchema"` /
/// `"<tool>.outputSchema"`.
fn published_schemas() -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for tool in ResonanceMcp::combined_router().list_all() {
        out.push((
            format!("{}.inputSchema", tool.name),
            Value::Object((*tool.input_schema).clone()),
        ));
        if let Some(output) = &tool.output_schema {
            out.push((
                format!("{}.outputSchema", tool.name),
                Value::Object((**output).clone()),
            ));
        }
    }
    assert!(
        out.len() >= 54,
        "expected the full tool surface (>=54 schemas), got {}",
        out.len()
    );
    out
}

#[test]
fn no_nonstandard_numeric_formats_in_published_schemas() {
    for (label, schema) in published_schemas() {
        walk(&schema, &label, &mut |path, node| {
            if let Value::Object(map) = node {
                if let Some(Value::String(fmt)) = map.get("format") {
                    assert!(
                        !NONSTANDARD_NUMERIC_FORMATS.contains(&fmt.as_str()),
                        "{path}: non-standard numeric format {fmt:?} leaked into a published \
                         schema; server::normalize_schemas should strip it"
                    );
                }
            }
        });
    }
}

#[test]
fn no_boolean_subschemas_under_properties_or_defs() {
    for (label, schema) in published_schemas() {
        walk(&schema, &label, &mut |path, node| {
            if let Value::Object(map) = node {
                for container in ["properties", "$defs"] {
                    if let Some(Value::Object(entries)) = map.get(container) {
                        for (key, sub) in entries {
                            assert!(
                                !sub.is_boolean(),
                                "{path}/{container}/{key} is a boolean subschema ({sub}); strict \
                                 MCP clients reject it — give the field an explicit object schema"
                            );
                        }
                    }
                }
            }
        });
    }
}

/// Input schemas allowed to keep a `$ref` because their type is
/// recursive (`inline_defs` leaves a self-reference and restores
/// `$defs`). None today; a new entry needs a reason next to it.
const RECURSIVE_INPUT_SCHEMAS: &[&str] = &[];

#[test]
fn no_input_schema_depends_on_ref_or_defs() {
    for (label, schema) in published_schemas() {
        let Some(tool) = label.strip_suffix(".inputSchema") else {
            continue;
        };
        if RECURSIVE_INPUT_SCHEMAS.contains(&tool) {
            continue;
        }
        walk(&schema, &label, &mut |path, node| {
            if let Value::Object(map) = node {
                for key in ["$ref", "$defs", "definitions"] {
                    assert!(
                        !map.contains_key(key),
                        "{path} has `{key}`: clients that drop `$defs` lose the field's type \
                         (ids then arrive as strings); server::inline_defs should inline it"
                    );
                }
            }
        });
    }
}

/// Whether a property schema admits a JSON integer: `"type": "integer"`,
/// a type list holding it (an `Option`), or an `anyOf`/`oneOf` branch
/// that does.
fn admits_integer(schema: &Value) -> bool {
    match schema.get("type") {
        Some(Value::String(t)) if t == "integer" => return true,
        Some(Value::Array(ts)) if ts.iter().any(|t| t == "integer") => return true,
        _ => {}
    }
    ["anyOf", "oneOf"].iter().any(|k| {
        schema
            .get(*k)
            .and_then(Value::as_array)
            .is_some_and(|branches| branches.iter().any(admits_integer))
    })
}

/// Fields typed by an id newtype (`resonance_control::ids`), which
/// serialize as plain JSON numbers.
const ID_FIELDS: &[&str] = &[
    "track_id",
    "bus_id",
    "clip_id",
    "job_id",
    "section_id",
    "definition_id",
    "placement_id",
    "chord_id",
    "send_id",
    "asset_id",
    "reference_id",
    "source_bus_id",
    "source_track_id",
];

#[test]
fn id_newtypes_inline_to_integer() {
    let mut checked = 0;
    for (label, schema) in published_schemas() {
        if !label.ends_with(".inputSchema") {
            continue;
        }
        walk(&schema, &label, &mut |path, node| {
            let Some(Value::Object(props)) = node.get("properties") else {
                return;
            };
            for (key, sub) in props {
                if !ID_FIELDS.contains(&key.as_str()) {
                    continue;
                }
                checked += 1;
                assert!(
                    admits_integer(sub),
                    "{path}/properties/{key} does not say it is an integer ({sub}); a client \
                     that cannot see its type sends the id as a string"
                );
            }
        });
    }
    assert!(checked >= 50, "only {checked} id fields found: the check is vacuous");
}
