//! JSON-RPC envelope: exact wire shapes, round-trips, error kinds, and
//! forward-compat tolerance.

use resonance_control::methods::control::HelloParams;
use resonance_control::{ErrorKind, Request, RequestId, Response, RpcError};
use serde_json::json;

#[test]
fn request_serializes_to_documented_shape() {
    let request = Request::new(
        1i64,
        "control.hello",
        &HelloParams {
            protocol_version: resonance_control::PROTOCOL_VERSION,
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "control.hello",
            "params": {"protocol_version": 1}
        })
    );
}

#[test]
fn request_roundtrips_with_string_id() {
    let request = Request::without_params("req-7", "song.summary");
    let wire = serde_json::to_string(&request).unwrap();
    let back: Request = serde_json::from_str(&wire).unwrap();
    assert_eq!(back, request);
    assert_eq!(back.id, RequestId::String("req-7".to_owned()));
    assert!(back.params.is_none());
}

#[test]
fn request_tolerates_unknown_fields_and_missing_jsonrpc() {
    let back: Request = serde_json::from_value(json!({
        "id": 42,
        "method": "transport.play",
        "some_future_field": {"nested": true}
    }))
    .unwrap();
    assert_eq!(back.jsonrpc, "2.0");
    assert_eq!(back.id, RequestId::Number(42));
    assert_eq!(back.method, "transport.play");
}

#[test]
fn typed_params_parse_and_reject() {
    let request = Request::new(1i64, "control.hello", &json!({"protocol_version": 3})).unwrap();
    let params: HelloParams = request.params().unwrap();
    assert_eq!(params.protocol_version, 3);

    let bad = Request::new(2i64, "control.hello", &json!({"protocol_version": "x"})).unwrap();
    let err = bad.params::<HelloParams>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
    assert!(err.message.contains("control.hello"));
}

#[test]
fn missing_params_parse_as_unit() {
    let request = Request::without_params(1i64, "transport.play");
    request.params::<()>().unwrap();
}

#[test]
fn missing_params_parse_as_all_default_structs() {
    use resonance_control::methods::project::NewParams;

    // Omitted params fall back to `{}` for structs whose fields are all
    // optional/defaulted (e.g. a bare `project.new`).
    let request = Request::without_params(1i64, "project.new");
    let params: NewParams = request.params().unwrap();
    assert_eq!(params, NewParams::default());

    // Explicit params must still be well-formed: the `{}` fallback only
    // applies when params were omitted entirely.
    let bad = Request::new(2i64, "project.new", &json!(42)).unwrap();
    let err = bad.params::<NewParams>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidParams);
}

#[test]
fn new_params_accept_the_template_id_alias() {
    use resonance_control::methods::project::NewParams;

    let params: NewParams = serde_json::from_value(json!({"template_id": "beatmaking"})).unwrap();
    assert_eq!(params.template.as_deref(), Some("beatmaking"));
    // Canonical serialization stays on the primary field name.
    assert_eq!(
        serde_json::to_value(&params).unwrap(),
        json!({"template": "beatmaking", "confirm": false})
    );
}

#[test]
fn response_success_roundtrip() {
    let response = Response::success(9i64, &json!({"revision": 5})).unwrap();
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(
        wire,
        json!({"jsonrpc": "2.0", "id": 9, "result": {"revision": 5}})
    );
    let back: Response = serde_json::from_value(wire).unwrap();
    let result: serde_json::Value = back.result().unwrap();
    assert_eq!(result, json!({"revision": 5}));
}

#[test]
fn error_response_has_documented_shape() {
    let response = Response::failure(
        Some(3i64.into()),
        RpcError::not_found("no track with id 12"),
    );
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "error": {
                "code": 1000,
                "message": "no track with id 12",
                "data": {"kind": "not_found", "detail": "no track with id 12"}
            }
        })
    );
}

#[test]
fn parse_error_response_serializes_null_id() {
    let response = Response::failure(None, RpcError::parse_error("bad json"));
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["id"], serde_json::Value::Null);
    assert_eq!(wire["error"]["code"], json!(-32700));
}

#[test]
fn result_on_error_response_surfaces_the_error() {
    let response = Response::failure(Some(1i64.into()), RpcError::busy("export in flight"));
    let err = response.result::<serde_json::Value>().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Busy);
    assert_eq!(err.message, "export in flight");
}

#[test]
fn every_error_kind_has_a_stable_wire_string() {
    for (kind, wire) in [
        (ErrorKind::NotFound, "not_found"),
        (ErrorKind::InvalidParams, "invalid_params"),
        (ErrorKind::NeedsConfirmation, "needs_confirmation"),
        (ErrorKind::Busy, "busy"),
        (ErrorKind::Unsupported, "unsupported"),
        (ErrorKind::Internal, "internal"),
    ] {
        assert_eq!(serde_json::to_value(kind).unwrap(), json!(wire));
        assert_eq!(kind.as_str(), wire);
        let back: ErrorKind = serde_json::from_value(json!(wire)).unwrap();
        assert_eq!(back, kind);
    }
}

#[test]
fn unknown_error_kind_deserializes_to_unknown() {
    let kind: ErrorKind = serde_json::from_value(json!("kind_from_the_future")).unwrap();
    assert_eq!(kind, ErrorKind::Unknown);
}

#[test]
fn method_not_found_uses_standard_code() {
    let err = RpcError::method_not_found("song.everything");
    assert_eq!(err.code, -32601);
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert!(err.message.contains("song.everything"));
}
