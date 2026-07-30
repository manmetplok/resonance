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
