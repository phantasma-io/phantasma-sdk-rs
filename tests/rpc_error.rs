//! How the node's answers reach the caller: a JSON-RPC error body is the answer whatever the HTTP
//! status, and a parameterless call still puts an empty parameter list on the wire.

mod common;

use common::CannedNode;
use httpmock::{Method::POST, MockServer};
use phantasma_sdk::{PhantasmaError, PhantasmaRpc};
use serde_json::json;

// The node answers a lookup it cannot satisfy with an HTTP error status AND a JSON-RPC error body.
// The body is the answer, and it reaches the caller as the typed JSON-RPC error - not as a
// transport error with the reason buried in its text.
#[tokio::test]
async fn a_json_rpc_error_inside_an_http_error_response_is_surfaced_as_the_json_rpc_error() {
    for status in [400u16, 500, 200] {
        let server = MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(POST).path("/rpc");
                then.status(status).header("content-type", "application/json").body(
                    r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"Token symbol not found"}}"#,
                );
            })
            .await;
        let err = PhantasmaRpc::new(server.url("/rpc"))
            .get_token_with_id("NOPE", false, 0)
            .await
            .unwrap_err();
        match err {
            PhantasmaError::Rpc { code, message } => {
                assert_eq!(code, Some(-32603), "status {status}");
                assert_eq!(message, "Token symbol not found", "status {status}");
            }
            other => panic!("status {status}: {other:?}"),
        }
    }
}

// Without a JSON-RPC body there is nothing to surface: an HTTP error stays a transport-level error
// naming its status.
#[tokio::test]
async fn an_http_error_without_a_json_rpc_body_stays_a_transport_error() {
    let server = MockServer::start_async().await;
    server
        .mock_async(|when, then| {
            when.method(POST).path("/rpc");
            then.status(502).body("upstream down");
        })
        .await;
    let err = PhantasmaRpc::new(server.url("/rpc"))
        .get_token_with_id("NOPE", false, 0)
        .await
        .unwrap_err();
    match err {
        PhantasmaError::Rpc { code, message } => {
            assert_eq!(code, None);
            assert!(message.starts_with("HTTP 502"), "{message}");
        }
        other => panic!("{other:?}"),
    }
}

// The node answers a request without a params field with HTTP 400 "Parse error", so every
// parameterless wrapper must put an empty list on the wire.
#[tokio::test]
async fn get_gas_config_sends_an_empty_parameter_list() {
    let node = CannedNode::new();
    let result = node.client().get_gas_config().await.unwrap();
    assert_eq!(result.gas_model_version, 2);
    let requests = node.state().requests.clone();
    assert_eq!(requests[0]["method"], "getGasConfig");
    assert_eq!(requests[0]["params"], json!([]));
}
