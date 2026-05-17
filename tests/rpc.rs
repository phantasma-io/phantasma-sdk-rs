use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use phantasma_sdk::{
    convert_decimals, parse_json_rpc_response, PhantasmaError, PhantasmaRpc, RpcTransport,
};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct MockTransport {
    inner: Arc<MockTransportState>,
}

#[derive(Default)]
struct MockTransportState {
    requests: Mutex<Vec<Value>>,
    responses: Mutex<VecDeque<(u16, Value)>>,
}

impl MockTransport {
    fn with_response(response: (u16, Value)) -> Self {
        let transport = Self::default();
        transport
            .inner
            .responses
            .lock()
            .unwrap()
            .push_back(response);
        transport
    }

    fn with_responses(responses: impl IntoIterator<Item = (u16, Value)>) -> Self {
        let transport = Self::default();
        transport.inner.responses.lock().unwrap().extend(responses);
        transport
    }

    fn requests(&self) -> Vec<Value> {
        self.inner.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl RpcTransport for MockTransport {
    async fn post_json(
        &self,
        _url: &str,
        body: Value,
        _timeout: Duration,
    ) -> phantasma_sdk::Result<(u16, Value)> {
        self.inner.requests.lock().unwrap().push(body);
        self.inner
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| PhantasmaError::Rpc {
                code: None,
                message: "missing mock response".into(),
            })
    }
}

#[tokio::test]
async fn rpc_wrapper_builds_json_rpc_request() {
    let transport = MockTransport::with_response((
        200,
        json!({
            "jsonrpc": "2.0",
            "id": "0",
            "result": {"version": "3.0.0", "commit": "abc123"}
        }),
    ));
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    let result = client.get_version().await.unwrap();

    assert_eq!(result.version, "3.0.0");
    assert_eq!(result.commit, "abc123");
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["jsonrpc"], "2.0");
    assert_eq!(requests[0]["id"], "0");
    assert_eq!(requests[0]["method"], "getVersion");
    assert_eq!(requests[0]["params"], json!([]));
}

#[tokio::test]
async fn send_raw_transaction_accepts_hash_object_results() {
    let transport = MockTransport::with_response((
        200,
        json!({
            "jsonrpc": "2.0",
            "id": "0",
            "result": {"hash": "ABCD"}
        }),
    ));
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    assert_eq!(client.send_raw_transaction("CAFE").await.unwrap(), "ABCD");
    let requests = transport.requests();
    assert_eq!(requests[0]["method"], "sendRawTransaction");
    assert_eq!(requests[0]["params"], json!(["CAFE"]));
}

#[tokio::test]
async fn typed_raw_block_call_preserves_sdk_value_and_response_metadata() {
    // Indexers need the typed block for normalization and the raw JSON-RPC
    // result/envelope for archival and parity checks.
    let block = json!({
        "hash": "ABCD",
        "previousHash": "PREV",
        "height": 42,
        "timestamp": 1000,
        "chainAddress": "PCHAIN",
        "protocol": 18,
        "validatorAddress": "PVALIDATOR",
        "reward": "0",
        "txs": [{
            "hash": "TX1",
            "chainAddress": "PCHAIN",
            "timestamp": 1000,
            "blockHeight": 42,
            "blockHash": "ABCD",
            "script": "CAFE",
            "payload": "504159",
            "events": [{
                "address": "PADDR",
                "contract": "gas",
                "kind": "GasEscrow",
                "name": "GasEscrow",
                "data": "00"
            }],
            "state": "Halt",
            "result": "",
            "fee": "467",
            "signatures": [{"kind": "Ed25519", "data": "SIG"}],
            "sender": "PSENDER",
            "expiration": 1234
        }]
    });
    let transport = MockTransport::with_response((
        200,
        json!({
            "jsonrpc": "2.0",
            "id": "0",
            "result": block
        }),
    ));
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    let result = client
        .get_block_by_height_with_raw("main", 42)
        .await
        .unwrap();

    assert_eq!(result.value.hash, "ABCD");
    assert_eq!(result.value.txs[0].hash, "TX1");
    assert_eq!(result.value.txs[0].payload, "504159");
    assert_eq!(result.raw_result["hash"], "ABCD");
    assert_eq!(result.raw_result["txs"][0]["sender"], "PSENDER");
    assert_eq!(
        result.raw_result["txs"][0]["events"][0]["name"],
        "GasEscrow"
    );
    assert_eq!(result.raw_envelope["result"]["hash"], "ABCD");
    assert_eq!(result.endpoint, "http://localhost:5172/rpc");
    assert_eq!(result.method, "getBlockByHeight");
    assert_eq!(result.http_status, 200);
    assert!(result.canonical_result_bytes > 0);
    assert!(result.canonical_envelope_bytes > 0);

    let requests = transport.requests();
    assert_eq!(requests[0]["method"], "getBlockByHeight");
    assert_eq!(requests[0]["params"], json!(["main", 42]));
}

#[tokio::test]
async fn typed_raw_block_height_call_preserves_scalar_result_shape() {
    // Block height can arrive as a string from current nodes; the typed value
    // should be coerced while the raw result keeps the original shape.
    let transport = MockTransport::with_response((
        200,
        json!({
            "jsonrpc": "2.0",
            "id": "0",
            "result": "123"
        }),
    ));
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    let result = client.get_block_height_with_raw("main").await.unwrap();

    assert_eq!(result.value, 123);
    assert_eq!(result.raw_result, json!("123"));
    assert_eq!(result.method, "getBlockHeight");
    assert_eq!(result.http_status, 200);
}

#[tokio::test]
async fn typed_raw_transaction_by_block_call_preserves_sdk_parameter_order() {
    // Explorer can hydrate missing transaction details without duplicating the
    // SDK's JSON-RPC method name or its historical parameter order.
    let transport = MockTransport::with_response((
        200,
        json!({
            "jsonrpc": "2.0",
            "id": "0",
            "result": {"hash": "TX1", "state": "Success"}
        }),
    ));
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    let result = client
        .get_transaction_by_block_hash_and_index_with_raw("BLOCK", 3, "main")
        .await
        .unwrap();

    assert_eq!(result.value.hash, "TX1");
    assert_eq!(result.raw_result["hash"], "TX1");
    assert_eq!(result.method, "getTransactionByBlockHashAndIndex");

    let requests = transport.requests();
    assert_eq!(requests[0]["method"], "getTransactionByBlockHashAndIndex");
    assert_eq!(requests[0]["params"], json!(["main", "BLOCK", 3]));
}

#[tokio::test]
async fn rpc_alias_methods_preserve_python_parameter_order() {
    fn ok(result: Value) -> (u16, Value) {
        (200, json!({"jsonrpc": "2.0", "id": "0", "result": result}))
    }

    let transport = MockTransport::with_responses([
        ok(json!(0)),
        ok(json!({"result": [], "cursor": ""})),
        ok(json!({"result": [], "cursor": ""})),
        ok(json!({"result": [], "cursor": ""})),
        ok(json!({"result": [], "cursor": ""})),
        ok(json!({"result": [], "cursor": ""})),
        ok(json!({"result": [], "cursor": ""})),
        ok(json!({"result": [], "cursor": ""})),
        ok(json!([])),
        ok(json!({})),
    ]);
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    client
        .get_block_transaction_count_by_hash_on_chain("side", "abc")
        .await
        .unwrap();
    client
        .get_token_nfts_with_series_id(7, 8, "series", 50, "cursor", false)
        .await
        .unwrap();
    client
        .get_account_fungible_tokens_with_address_type("Pabc", "SOUL", 7, 10, "", false, "User")
        .await
        .unwrap();
    client
        .get_account_nfts_with_address_type("Pabc", "ART", 7, 8, 10, "", true, false, "User")
        .await
        .unwrap();
    client
        .get_account_owned_tokens("Pabc", "", 0, 100, "", false)
        .await
        .unwrap();
    client
        .get_account_owned_tokens_with_address_type("Pabc", "ART", 7, 100, "", false, "User")
        .await
        .unwrap();
    client
        .get_account_owned_token_series("Pabc", "ART", 7, 100, "", false)
        .await
        .unwrap();
    client
        .get_account_owned_token_series_with_address_type("Pabc", "ART", 7, 100, "", false, "User")
        .await
        .unwrap();
    client.get_nfts_text("ART", "1,2", false).await.unwrap();
    client
        .get_token_series_by_id("ART", "series")
        .await
        .unwrap();

    let requests = transport.requests();
    assert_eq!(requests[0]["method"], "getBlockTransactionCountByHash");
    assert_eq!(requests[0]["params"], json!(["side", "abc"]));
    assert_eq!(requests[1]["method"], "getTokenNFTs");
    assert_eq!(
        requests[1]["params"],
        json!([7, 8, 50, "cursor", false, "series"])
    );
    assert_eq!(requests[2]["method"], "getAccountFungibleTokens");
    assert_eq!(
        requests[2]["params"],
        json!(["Pabc", "SOUL", 7, 10, "", false, "User"])
    );
    assert_eq!(requests[3]["method"], "getAccountNFTs");
    assert_eq!(
        requests[3]["params"],
        json!(["Pabc", "ART", 7, 8, 10, "", true, false, "User"])
    );
    assert_eq!(requests[4]["method"], "getAccountOwnedTokens");
    assert_eq!(
        requests[4]["params"],
        json!(["Pabc", "", 0, 100, "", false])
    );
    assert_eq!(requests[5]["method"], "getAccountOwnedTokens");
    assert_eq!(
        requests[5]["params"],
        json!(["Pabc", "ART", 7, 100, "", false, "User"])
    );
    assert_eq!(requests[6]["method"], "getAccountOwnedTokenSeries");
    assert_eq!(
        requests[6]["params"],
        json!(["Pabc", "ART", 7, 100, "", false])
    );
    assert_eq!(requests[7]["method"], "getAccountOwnedTokenSeries");
    assert_eq!(
        requests[7]["params"],
        json!(["Pabc", "ART", 7, 100, "", false, "User"])
    );
    assert_eq!(requests[8]["method"], "getNFTs");
    assert_eq!(requests[8]["params"], json!(["ART", "1,2", false]));
    assert_eq!(requests[9]["method"], "getTokenSeriesById");
    assert_eq!(requests[9]["params"], json!(["ART", 0, "series", 0]));
}

#[tokio::test]
async fn rpc_decodes_reference_shapes_and_coerces_scalars() {
    fn ok(result: Value) -> (u16, Value) {
        (200, json!({"jsonrpc": "2.0", "id": "0", "result": result}))
    }

    let transport = MockTransport::with_responses([
        ok(json!({"version": "3.0.0", "commit": "abc123", "buildTimeUtc": "now"})),
        ok(json!("12")),
        ok(json!("true")),
        ok(
            json!({"seriesId": "1", "carbonTokenId": "7", "carbonSeriesId": "8", "metadata": [{"Key": "name", "Value": "A"}]}),
        ),
        ok(json!({"hash": "C0DE"})),
        ok(json!({"isStored": true, "featureLevel": 5, "gasConstructor": "1"})),
    ]);
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    let version = client.get_version().await.unwrap();
    assert_eq!(version.build_time_utc, "now");
    assert_eq!(client.get_block_height("main").await.unwrap(), 12);
    assert!(client.write_archive_base64("hash", 1, "").await.unwrap());
    let series = client.get_token_series_by_id("ART", "1").await.unwrap();
    assert_eq!(series.carbon_token_id, "7");
    assert_eq!(series.carbon_series_id, "8");
    assert_eq!(series.metadata[0].key, "name");
    assert_eq!(
        client.send_carbon_transaction(&[0xca, 0xfe]).await.unwrap(),
        "C0DE"
    );
    let config = client.get_phantasma_vm_config("main").await.unwrap();
    assert!(config.is_stored);
    assert_eq!(config.feature_level, 5);
    assert_eq!(config.gas_constructor, "1");

    let requests = transport.requests();
    assert_eq!(requests[4]["method"], "sendCarbonTransaction");
    assert_eq!(requests[4]["params"], json!(["cafe"]));
}

#[test]
fn json_rpc_parser_fails_closed_on_malformed_responses() {
    assert_eq!(
        parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "id": 0, "result": true})).unwrap(),
        json!(true)
    );
    assert!(parse_json_rpc_response(200, json!([])).is_err());
    assert!(
        parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "id": "1", "result": true})).is_err()
    );
    assert!(
        parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "id": null, "result": true}))
            .is_err()
    );
    assert!(
        parse_json_rpc_response(500, json!({"jsonrpc": "2.0", "id": "0", "result": true})).is_err()
    );
    assert!(parse_json_rpc_response(
        200,
        json!({"jsonrpc": "2.0", "id": "0", "error": {"code": -32601, "message": "missing"}})
    )
    .is_err());
}

#[test]
fn rpc_helpers_cover_common_result_shapes() {
    assert_eq!(convert_decimals("100000000", 8), "1");
    assert_eq!(convert_decimals("123456789", 8), "1.23456789");
    assert_eq!(convert_decimals("-1", 8), "-0.00000001");

    let result = phantasma_sdk::ScriptResult {
        result: "030101".to_string(),
        results: vec!["030101".to_string()],
        ..Default::default()
    };
    assert_eq!(
        result.decode_result().unwrap().as_number().unwrap(),
        1.into()
    );
    assert_eq!(
        result.decode_results(0).unwrap().as_number().unwrap(),
        1.into()
    );
}
