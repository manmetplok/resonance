//! The published shape of the plugin-parameter tools (ba todo #1290).
//!
//! A tool's schema is the whole of what an MCP client knows before it
//! calls: if `choices` is not in the output schema a strict client drops
//! it, and if `value` is declared as a number a client will never send
//! `"Low-pass"` even though the app accepts it. These assert the
//! parameter *meaning* fields survive publication, on all three chains.

use resonance_mcp::ResonanceMcp;
use serde_json::Value;

/// `(input, output)` schema of a published tool.
fn schemas(name: &str) -> (Value, Option<Value>) {
    let tool = ResonanceMcp::combined_router()
        .list_all()
        .into_iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("{name} is not a published tool"));
    (
        Value::Object((*tool.input_schema).clone()),
        tool.output_schema
            .as_ref()
            .map(|s| Value::Object((**s).clone())),
    )
}

/// Follow a one-level `$ref` into the schema's `$defs`, which is where
/// schemars puts a type shared by several tools (`ParamValue`). Returns
/// `node` unchanged when it is not a reference.
fn resolve(root: &Value, node: &Value) -> Value {
    let Some(Value::String(reference)) = node.get("$ref") else {
        return node.clone();
    };
    let name = reference
        .rsplit('/')
        .next()
        .expect("a $ref has a last segment");
    root.get("$defs")
        .and_then(|defs| defs.get(name))
        .cloned()
        .unwrap_or_else(|| panic!("{reference} is not defined in the published schema"))
}

/// Every `"type"` string appearing anywhere under `node`.
fn types_under(node: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(node: &Value, out: &mut Vec<String>) {
        match node {
            Value::Object(map) => {
                if let Some(Value::String(t)) = map.get("type") {
                    out.push(t.clone());
                }
                for v in map.values() {
                    walk(v, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    walk(node, &mut out);
    out
}

/// Every property name appearing anywhere in a schema — the definitions
/// of the entry/param views are nested under `$defs`, so a flat scan is
/// what tells us a field reached publication at all.
fn property_names(schema: &Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(node: &Value, out: &mut Vec<String>) {
        match node {
            Value::Object(map) => {
                if let Some(Value::Object(props)) = map.get("properties") {
                    out.extend(props.keys().cloned());
                }
                for v in map.values() {
                    walk(v, out);
                }
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    walk(schema, &mut out);
    out
}

#[test]
fn the_param_views_publish_what_a_value_means() {
    for tool in [
        "track_plugin_params",
        "bus_plugin_params",
        "master_plugin_params",
    ] {
        let (_, output) = schemas(tool);
        let output = output.unwrap_or_else(|| panic!("{tool} publishes an output schema"));
        let names = property_names(&output);
        for field in ["text", "unit", "module", "stepped", "choices", "hidden"] {
            assert!(
                names.iter().any(|n| n == field),
                "{tool}'s output schema does not publish {field:?}; a strict client will drop \
                 it and the parameter goes back to being a bare number"
            );
        }
        // The number is still there, of course.
        for field in ["id", "name", "value", "min", "max", "default"] {
            assert!(names.iter().any(|n| n == field), "{tool} lost {field:?}");
        }
    }
}

#[test]
fn a_set_takes_a_number_or_a_choice_label() {
    for tool in [
        "track_set_plugin_param",
        "bus_set_plugin_param",
        "master_set_plugin_param",
    ] {
        let (input, _) = schemas(tool);
        let value = input
            .get("properties")
            .and_then(|p| p.get("value"))
            .unwrap_or_else(|| panic!("{tool} takes a value"));
        // schemars hoists a shared enum into `$defs` and points the
        // property at it, so the accepted types are one hop away.
        let value = resolve(&input, value);
        let types = types_under(&value);
        assert!(
            types.iter().any(|t| t == "number"),
            "{tool}'s value must still accept a number: {value}"
        );
        assert!(
            types.iter().any(|t| t == "string"),
            "{tool}'s value must accept a choice label too, or a client will never send one: \
             {value}"
        );
    }
}

#[test]
fn the_descriptions_tell_a_client_the_fields_exist() {
    let router = ResonanceMcp::combined_router();
    let described = |name: &str| -> String {
        router
            .list_all()
            .into_iter()
            .find(|t| t.name == name)
            .and_then(|t| t.description.as_deref().map(str::to_owned))
            .unwrap_or_default()
    };
    // A field nothing points at is a field nobody reads: the read tool
    // has to say the meaning is there, and the write tool has to say a
    // label is accepted.
    assert!(described("track_plugin_params").contains("choices"));
    assert!(described("track_set_plugin_param").contains("choices"));
}
