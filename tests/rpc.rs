use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use httpmock::{Method::POST, MockServer};
use phantasma_sdk::{
    convert_decimals, parse_json_rpc_response, parse_json_rpc_response_for_request, PhantasmaError,
    PhantasmaRpc, RpcTransport,
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

fn assert_rpc_error_contains<T>(result: phantasma_sdk::Result<T>, expected: &str) {
    match result {
        Err(PhantasmaError::Rpc { message, .. }) => {
            assert!(
                message.contains(expected),
                "expected RPC error containing {expected:?}, got {message:?}"
            );
        }
        Err(err) => panic!("expected RPC error containing {expected:?}, got {err:?}"),
        Ok(_) => panic!("expected RPC error containing {expected:?}, got success"),
    }
}

fn assert_rpc_error_message(err: PhantasmaError, expected: &str) {
    match err {
        PhantasmaError::Rpc { message, .. } => {
            assert!(
                message.contains(expected),
                "expected RPC error containing {expected:?}, got {message:?}"
            );
        }
        other => panic!("expected RPC error containing {expected:?}, got {other:?}"),
    }
}

#[tokio::test]
async fn rpc_wrapper_builds_json_rpc_request() {
    let transport = MockTransport::with_response((
        200,
        json!({
            "jsonrpc": "2.0",
            "id": "1",
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
    assert_eq!(requests[0]["id"], 1);
    assert_eq!(requests[0]["method"], "getVersion");
    assert_eq!(requests[0]["params"], json!([]));
}

#[tokio::test]
async fn reqwest_transport_rejects_response_body_above_configured_limit() {
    let body = r#"{"jsonrpc":"2.0","id":"1","result":{"version":"3.0.0","commit":"abc123"}}"#;
    let server = MockServer::start();
    let mock = server.mock(|when, then| {
        when.method(POST).path("/rpc");
        then.status(200)
            .header("content-type", "application/json")
            .body(body);
    });
    let client = PhantasmaRpc::new(server.url("/rpc")).with_max_response_bytes(body.len() - 1);

    assert_rpc_error_contains(client.get_version().await, "response body exceeds");

    mock.assert();
}

#[tokio::test]
async fn rpc_client_increments_request_ids_for_sequential_normal_calls() {
    // Normal public calls must generate a fresh id and validate each response
    // against the id produced for that specific request.
    let transport = MockTransport::with_responses([
        (
            200,
            json!({"jsonrpc": "2.0", "id": "1", "result": {"version": "3.0.0", "commit": "first"}}),
        ),
        (
            200,
            json!({"jsonrpc": "2.0", "id": "2", "result": {"version": "3.0.1", "commit": "second"}}),
        ),
    ]);
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    let first = client.get_version().await.unwrap();
    let second = client.get_version().await.unwrap();

    assert_eq!(first.commit, "first");
    assert_eq!(second.commit, "second");
    let requests = transport.requests();
    assert_eq!(requests[0]["id"], 1);
    assert_eq!(requests[1]["id"], 2);
}

#[tokio::test]
async fn rpc_client_rejects_stale_response_id_after_counter_advances() {
    // A stale response echo from the first request must not unlock a later
    // result once the client has advanced to a different request id.
    let transport = MockTransport::with_responses([
        (
            200,
            json!({"jsonrpc": "2.0", "id": "1", "result": {"version": "3.0.0", "commit": "first"}}),
        ),
        (
            200,
            json!({"jsonrpc": "2.0", "id": "1", "result": {"version": "bad", "commit": "stale"}}),
        ),
    ]);
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    assert_eq!(client.get_version().await.unwrap().commit, "first");
    assert_rpc_error_contains(client.get_version().await, "response id mismatch");

    let requests = transport.requests();
    assert_eq!(requests[0]["id"], 1);
    assert_eq!(requests[1]["id"], 2);
}

#[tokio::test]
async fn rpc_client_rejects_uncorrelated_response_ids_on_public_call_paths() {
    let invalid_envelopes = [
        (
            "numeric zero",
            json!({"jsonrpc": "2.0", "id": 0, "result": {"version": "bad", "commit": "bad"}}),
            "response id mismatch",
        ),
        (
            "string zero",
            json!({"jsonrpc": "2.0", "id": "0", "result": {"version": "bad", "commit": "bad"}}),
            "response id mismatch",
        ),
        (
            "wrong string",
            json!({"jsonrpc": "2.0", "id": "2", "result": {"version": "bad", "commit": "bad"}}),
            "response id mismatch",
        ),
        (
            "missing",
            json!({"jsonrpc": "2.0", "result": {"version": "bad", "commit": "bad"}}),
            "missing id",
        ),
        (
            "null",
            json!({"jsonrpc": "2.0", "id": null, "result": {"version": "bad", "commit": "bad"}}),
            "response id mismatch",
        ),
        (
            "object",
            json!({"jsonrpc": "2.0", "id": {"request": 1}, "result": {"version": "bad", "commit": "bad"}}),
            "response id mismatch",
        ),
    ];

    for (label, envelope, expected_error) in invalid_envelopes {
        for call_path in ["call_value", "call_with_raw", "call_typed"] {
            let transport = MockTransport::with_response((200, envelope.clone()));
            let client =
                PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

            match call_path {
                "call_value" => {
                    assert_rpc_error_contains(
                        client.call_value("getVersion", vec![]).await,
                        expected_error,
                    );
                }
                "call_with_raw" => {
                    assert_rpc_error_contains(
                        client.call_with_raw::<Value>("getVersion", vec![]).await,
                        expected_error,
                    );
                }
                "call_typed" => {
                    assert_rpc_error_contains(
                        client
                            .call::<phantasma_sdk::BuildInfoResult>("getVersion", vec![])
                            .await,
                        expected_error,
                    );
                }
                _ => unreachable!(),
            }

            let requests = transport.requests();
            assert_eq!(requests.len(), 1, "{label} via {call_path}");
            assert_eq!(requests[0]["id"], 1, "{label} via {call_path}");
        }
    }
}

#[tokio::test]
async fn send_raw_transaction_accepts_hash_object_results() {
    let transport = MockTransport::with_response((
        200,
        json!({
            "jsonrpc": "2.0",
            "id": "1",
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
            "carbonTxType": 15,
            "carbonTxData": "BEEF",
            "payload": "504159",
            "events": [{
                "address": "PADDR",
                "contract": "gas",
                "kind": "GasEscrow",
                "name": "GasEscrow",
                "data": "00"
            }],
            "extendedEvents": [{
                "address": "PADDR",
                "contract": "token",
                "kind": "TokenCreate",
                "data": {"symbol": "SOUL"}
            }],
            "state": "Halt",
            "result": "",
            "debugComment": "ok",
            "fee": "467",
            "signatures": [{"kind": "Ed25519", "data": "SIG"}],
            "sender": "PSENDER",
            "gasPayer": "PGASPAYER",
            "gasTarget": "PGASTARGET",
            "gasPrice": "1",
            "gasLimit": "2100000000",
            "expiration": 1234
        }]
    });
    let transport = MockTransport::with_response((
        200,
        json!({
            "jsonrpc": "2.0",
            "id": "1",
            "result": block
        }),
    ));
    let client = PhantasmaRpc::with_transport("http://localhost:5172/rpc", transport.clone());

    let result = client
        .get_block_by_height_with_raw("main", 42)
        .await
        .unwrap();

    assert_eq!(result.value.hash, "ABCD");
    assert_eq!(result.value.txs[0].sender, "PSENDER");
    assert_eq!(result.value.txs[0].gas_payer, "PGASPAYER");
    assert_eq!(result.value.txs[0].gas_target, "PGASTARGET");
    assert_eq!(result.value.txs[0].gas_price, "1");
    assert_eq!(result.value.txs[0].gas_limit, "2100000000");
    assert_eq!(result.value.txs[0].debug_comment.as_deref(), Some("ok"));
    assert_eq!(result.value.txs[0].carbon_tx_type, 15);
    assert_eq!(result.value.txs[0].carbon_tx_data, "BEEF");
    assert_eq!(result.value.txs[0].events[0].name, "GasEscrow");
    assert_eq!(result.value.txs[0].extended_events[0].kind, "TokenCreate");
    assert_eq!(
        result.value.txs[0].extended_events[0].data["symbol"],
        "SOUL"
    );
    assert_eq!(result.raw_result["hash"], "ABCD");
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
            "id": "1",
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
            "id": "1",
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
    // This sequence exercises many wrapper calls against one client, so each
    // mock response must echo the next generated request id instead of hiding
    // stale-id bugs behind a fixed fixture.
    let mut next_id = 1_u64;
    let mut ok = |result: Value| {
        let id = next_id;
        next_id += 1;
        (
            200,
            json!({"jsonrpc": "2.0", "id": id.to_string(), "result": result}),
        )
    };

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
        ok(json!({"result": [], "cursor": ""})),
        ok(json!({"address": "Pmember", "isMember": true})),
        ok(json!({"result": [], "cursor": ""})),
        ok(json!({"address": "Pmember", "isMember": true})),
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
    client.get_organizations(2, "cursor", true).await.unwrap();
    client
        .get_organization_members("masters", 2, "", true)
        .await
        .unwrap();
    client
        .get_organization_member("masters", "Pmember", true, "Carbon")
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
    assert_eq!(requests[10]["method"], "getOrganizations");
    assert_eq!(requests[10]["params"], json!([2, "cursor", true]));
    assert_eq!(requests[11]["method"], "getOrganizationMembers");
    assert_eq!(requests[11]["params"], json!(["masters", 2, "", true]));
    assert_eq!(requests[12]["method"], "getOrganizationMember");
    assert_eq!(
        requests[12]["params"],
        json!(["masters", "Pmember", true, "Carbon"])
    );
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(request["id"], json!(index + 1));
    }
}

#[tokio::test]
async fn rpc_decodes_reference_shapes_and_coerces_scalars() {
    // The DTO fixture walk uses one client for several calls; echoed ids must
    // follow the generated request ids so the parser contract is actually tested.
    let mut next_id = 1_u64;
    let mut ok = |result: Value| {
        let id = next_id;
        next_id += 1;
        (
            200,
            json!({"jsonrpc": "2.0", "id": id.to_string(), "result": result}),
        )
    };

    let transport = MockTransport::with_responses([
        ok(json!({"version": "3.0.0", "commit": "abc123", "buildTimeUtc": "now"})),
        ok(json!("12")),
        ok(json!("true")),
        ok(
            json!({"seriesId": "1", "carbonTokenId": "7", "carbonSeriesId": "8", "metadata": [{"key": "name", "value": "A"}]}),
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
    for (index, request) in requests.iter().enumerate() {
        assert_eq!(request["id"], json!(index + 1));
    }
}

#[test]
fn rpc_dtos_decode_current_response_shapes_without_stale_aliases() {
    let event = json!({
        "address": "PADDR",
        "contract": "gas",
        "kind": "GasEscrow",
        "name": "GasEscrow",
        "data": "00"
    });
    let extended_event = json!({
        "address": "PADDR",
        "contract": "token",
        "kind": "TokenMint",
        "data": {
            "symbol": "CROWN",
            "seriesId": "0",
            "carbonSeriesId": 1
        }
    });
    let tx = json!({
        "hash": "TX1",
        "chainAddress": "PCHAIN",
        "timestamp": 1000,
        "blockHeight": 42,
        "blockHash": "ABCD",
        "script": "CAFE",
        "carbonTxType": 15,
        "carbonTxData": "BEEF",
        "payload": "504159",
        "events": [event.clone()],
        "extendedEvents": [extended_event.clone()],
        "state": "Halt",
        "result": "",
        "fee": "467",
        "signatures": [{"kind": "Ed25519", "data": "SIG"}],
        "sender": "PSENDER",
        "gasPayer": "PGASPAYER",
        "gasTarget": "PGASTARGET",
        "gasPrice": "1",
        "gasLimit": "2100000000",
        "expiration": 1234
    });

    let tx_result: phantasma_sdk::TransactionResult = serde_json::from_value(tx.clone()).unwrap();
    assert_eq!(tx_result.carbon_tx_type, 15);
    assert_eq!(tx_result.carbon_tx_data, "BEEF");
    assert_eq!(tx_result.sender, "PSENDER");
    assert_eq!(tx_result.gas_payer, "PGASPAYER");
    assert_eq!(tx_result.gas_target, "PGASTARGET");
    assert_eq!(tx_result.gas_price, "1");
    assert_eq!(tx_result.gas_limit, "2100000000");
    assert_eq!(tx_result.events[0].name, "GasEscrow");
    assert_eq!(tx_result.extended_events[0].data["symbol"], "CROWN");
    assert_eq!(tx_result.debug_comment, None);

    let mut tx_with_debug = tx;
    tx_with_debug["debugComment"] = json!("accepted");
    let tx_result: phantasma_sdk::TransactionResult =
        serde_json::from_value(tx_with_debug).unwrap();
    assert_eq!(tx_result.debug_comment.as_deref(), Some("accepted"));

    let stale_signature: phantasma_sdk::SignatureResult =
        serde_json::from_value(json!({"Kind": "Ed25519", "Data": "SIG"})).unwrap();
    assert_eq!(stale_signature.kind, "");
    assert_eq!(stale_signature.data, "");
    let current_signature: phantasma_sdk::SignatureResult =
        serde_json::from_value(json!({"kind": "Ed25519", "data": "SIG"})).unwrap();
    assert_eq!(current_signature.kind, "Ed25519");
    assert_eq!(current_signature.data, "SIG");

    let stale_property: phantasma_sdk::TokenPropertyResult =
        serde_json::from_value(json!({"Key": "name", "Value": "Crown"})).unwrap();
    assert_eq!(stale_property.key, "");
    assert_eq!(stale_property.value, "");
    let current_property: phantasma_sdk::TokenPropertyResult =
        serde_json::from_value(json!({"key": "name", "value": "Crown"})).unwrap();
    assert_eq!(current_property.key, "name");
    assert_eq!(current_property.value, "Crown");

    let block: phantasma_sdk::BlockResult = serde_json::from_value(json!({
        "hash": "ABCD",
        "previousHash": "PREV",
        "height": 42,
        "timestamp": 1000,
        "chainAddress": "PCHAIN",
        "protocol": 18,
        "validatorAddress": "PVALIDATOR",
        "reward": "0",
        "txs": [tx_result]
    }))
    .unwrap();
    assert_eq!(block.txs[0].hash, "TX1");
    assert_eq!(block.events, None);
    assert_eq!(block.oracles, None);

    let balance: phantasma_sdk::BalanceResult = serde_json::from_value(json!({
        "chain": "main",
        "amount": "1",
        "symbol": "SOUL",
        "decimals": 8
    }))
    .unwrap();
    assert_eq!(balance.ids, None);
    let cursor_page: phantasma_sdk::CursorPaginatedResult<Vec<phantasma_sdk::BalanceResult>> =
        serde_json::from_value(json!({"result": []})).unwrap();
    assert_eq!(cursor_page.cursor, None);
    let account: phantasma_sdk::AccountResult = serde_json::from_value(json!({
        "address": "PADDR",
        "name": "",
        "stakes": {"amount": "0", "time": 0, "unclaimed": "0"},
        "stake": "0",
        "unclaimed": "0",
        "validator": "",
        "storage": {"available": 0, "used": 0, "avatar": "", "archives": []},
        "balances": [balance]
    }))
    .unwrap();
    assert_eq!(account.relay, None);
    assert_eq!(account.txs, None);

    let archive: phantasma_sdk::ArchiveResult =
        serde_json::from_value(json!({"time": 0, "size": 0, "blockCount": 0})).unwrap();
    assert_eq!(archive.hash, None);
    assert_eq!(archive.name, None);
    assert_eq!(archive.encryption, None);
    assert_eq!(archive.missing_blocks, None);
    assert_eq!(archive.owners, None);

    let chain: phantasma_sdk::ChainResult = serde_json::from_value(json!({"height": 0})).unwrap();
    assert_eq!(chain.name, None);
    assert_eq!(chain.contracts, None);
    let nexus: phantasma_sdk::NexusResult = serde_json::from_value(json!({"protocol": 0})).unwrap();
    assert_eq!(nexus.name, None);
    assert_eq!(nexus.tokens, None);
    let organization: phantasma_sdk::OrganizationResult =
        serde_json::from_value(json!({})).unwrap();
    assert_eq!(organization.member_count, None);
    assert!(organization.metadata.is_empty());
    let organization: phantasma_sdk::OrganizationResult = serde_json::from_value(json!({
        "name": "masters",
        "owner": "Powner",
        "carbonOwner": "0xowner",
        "memberCount": "2",
        "metadata": [{"key": "role", "value": "validators"}]
    }))
    .unwrap();
    assert_eq!(organization.carbon_owner.as_deref(), Some("0xowner"));
    assert_eq!(organization.member_count.as_deref(), Some("2"));
    assert_eq!(organization.metadata[0].key, "role");
    let organization_member: phantasma_sdk::OrganizationMemberResult =
        serde_json::from_value(json!({
            "address": "Pmember",
            "carbonAddress": "0xmember",
            "isMember": true,
            "memberTime": 123
        }))
        .unwrap();
    assert!(organization_member.is_member);
    assert_eq!(organization_member.member_time, Some(123));
    let leaderboard: phantasma_sdk::LeaderboardResult = serde_json::from_value(json!({})).unwrap();
    assert_eq!(leaderboard.name, None);
    assert_eq!(leaderboard.rows, None);

    let contract: phantasma_sdk::ContractResult = serde_json::from_value(json!({
        "name": "account",
        "address": "SADDR",
        "script": "0B",
        "methods": [{
            "name": "LookUpName",
            "returnType": "Object",
            "parameters": [{"name": "name", "type": "String"}]
        }],
        "events": [{"name": "Created", "returnType": "None", "value": 1, "description": ""}]
    }))
    .unwrap();
    assert_eq!(contract.owner, None);
    assert_eq!(
        contract.methods.as_ref().unwrap()[0].parameters[0].type_name,
        "String"
    );
    assert_eq!(contract.events.as_ref().unwrap()[0].return_type, "None");

    let token: phantasma_sdk::TokenResult = serde_json::from_value(json!({
        "symbol": "CROWN",
        "name": "Phantasma Crown",
        "decimals": 0,
        "currentSupply": "10998",
        "maxSupply": "0",
        "burnedSupply": "595",
        "address": "SADDR",
        "owner": "PADDR",
        "flags": "Transferable, Burnable",
        "series": [],
        "carbonId": "4",
        "metadata": [{"key": "name", "value": "Phantasma Crown"}],
        "tokenSchemas": {
            "seriesMetadata": {"fields": [{"name": "_i", "schema": {"type": "Int256"}}], "flags": 3},
            "rom": {"fields": [], "flags": 2},
            "ram": {"fields": [], "flags": 0}
        }
    }))
    .unwrap();
    assert_eq!(token.carbon_id, "4");
    assert_eq!(token.metadata.as_ref().unwrap()[0].key, "name");
    assert!(token.token_schemas.is_some());
    assert_eq!(token.external, None);
    assert_eq!(token.price, None);
    let stale_token: phantasma_sdk::TokenResult = serde_json::from_value(json!({
        "symbol": "BAD",
        "carbonID": "4"
    }))
    .unwrap();
    assert_eq!(stale_token.carbon_id, "");

    let series: phantasma_sdk::TokenSeriesResult = serde_json::from_value(json!({
        "seriesId": "0",
        "carbonTokenId": "4",
        "carbonSeriesId": "1",
        "ownerAddress": "PADDR",
        "maxMint": "0",
        "mintCount": "11593",
        "currentSupply": "10998",
        "maxSupply": "0",
        "metadata": [{"key": "mode", "value": "0"}]
    }))
    .unwrap();
    assert_eq!(series.series_id, "0");
    assert_eq!(series.carbon_series_id, "1");
    assert_eq!(series.burned_supply, None);
    assert_eq!(series.methods, None);

    let nft_json = json!({
        "id": "102027540816489236327796452815702733520646114324490783683230488899035835189818",
        "series": "0",
        "carbonTokenId": "4",
        "carbonSeriesId": "1",
        "carbonNftAddress": "0000000000000000000000000000000104000000000000000100000000010000",
        "mint": "256",
        "chainName": "main",
        "ownerAddress": "POWNER",
        "creatorAddress": "PCREATOR",
        "ram": "",
        "rom": "220100",
        "status": "Active",
        "infusion": [{"key": "KCAL", "value": "3121772258"}],
        "properties": []
    });
    let nft: phantasma_sdk::NftResult = serde_json::from_value(nft_json.clone()).unwrap();
    assert_eq!(nft.id, nft_json["id"].as_str().unwrap());
    assert_eq!(nft.carbon_series_id, "1");
    assert_eq!(nft.infusion[0].key, "KCAL");
    let token_data: phantasma_sdk::TokenDataResult = serde_json::from_value(nft_json).unwrap();
    assert_eq!(token_data.series, "0");
    assert_eq!(token_data.carbon_series_id, "1");

    let auction: phantasma_sdk::AuctionResult = serde_json::from_value(json!({
        "creatorAddress": "PSELLER",
        "chainAddress": "SCHAIN",
        "startDate": 1,
        "endDate": 2,
        "baseSymbol": "CROWN",
        "quoteSymbol": "KCAL",
        "tokenId": "1",
        "price": "10",
        "endPrice": "10",
        "extensionPeriod": "0",
        "type": "Fixed",
        "rom": "",
        "ram": "",
        "listingFee": "0",
        "currentWinner": ""
    }))
    .unwrap();
    assert_eq!(auction.type_name, "Fixed");

    let script: phantasma_sdk::ScriptResult = serde_json::from_value(json!({
        "events": [event],
        "result": "030101",
        "results": ["030101"],
        "oracles": [],
        "error": ""
    }))
    .unwrap();
    assert_eq!(script.error.as_deref(), Some(""));
    assert_eq!(script.state, None);
    assert_eq!(script.gas, None);

    let config: phantasma_sdk::PhantasmaVmConfigResult = serde_json::from_value(json!({
        "isStored": true,
        "featureLevel": 1,
        "gasConstructor": "10",
        "gasNexus": "1000",
        "gasOrganization": "200",
        "gasAccount": "100",
        "gasLeaderboard": "100",
        "gasStandard": "50",
        "gasOracle": "100",
        "fuelPerContractDeploy": "2000"
    }))
    .unwrap();
    assert!(config.is_stored);
    assert_eq!(config.fuel_per_contract_deploy, "2000");
}

#[test]
fn json_rpc_parser_fails_closed_on_malformed_responses() {
    // Only the exact request id may unlock the result; missing/null/mismatched ids fail closed.
    assert_eq!(
        parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "id": 1, "result": true})).unwrap(),
        json!(true)
    );
    assert_eq!(
        parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "id": "1", "result": true})).unwrap(),
        json!(true)
    );
    assert!(parse_json_rpc_response(200, json!([])).is_err());
    assert!(
        parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "id": 0, "result": true})).is_err()
    );
    assert!(
        parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "id": "2", "result": true})).is_err()
    );
    assert!(parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "result": true})).is_err());
    assert!(
        parse_json_rpc_response(200, json!({"jsonrpc": "2.0", "id": null, "result": true}))
            .is_err()
    );
    assert!(
        parse_json_rpc_response(500, json!({"jsonrpc": "2.0", "id": "1", "result": true})).is_err()
    );
    assert!(parse_json_rpc_response(
        200,
        json!({"jsonrpc": "2.0", "id": "1", "error": {"code": -32601, "message": "missing"}})
    )
    .is_err());
    assert_eq!(
        parse_json_rpc_response_for_request(
            200,
            json!({"jsonrpc": "2.0", "id": "2", "result": true}),
            2
        )
        .unwrap(),
        json!(true)
    );
    assert!(parse_json_rpc_response_for_request(
        200,
        json!({"jsonrpc": "2.0", "id": "1", "result": true}),
        2
    )
    .is_err());
}

#[test]
fn json_rpc_parser_validates_id_before_error_status_and_result() {
    let result = parse_json_rpc_response(
        200,
        json!({
            "jsonrpc": "2.0",
            "id": 0,
            "error": {"code": -32603, "message": "server error from another request"}
        }),
    );
    assert_rpc_error_message(result.unwrap_err(), "response id mismatch");

    let result = parse_json_rpc_response(
        500,
        json!({
            "jsonrpc": "2.0",
            "id": "0",
            "result": true
        }),
    );
    assert_rpc_error_message(result.unwrap_err(), "response id mismatch");

    let result = parse_json_rpc_response(
        200,
        json!({
            "jsonrpc": "2.0",
            "id": {"unexpected": 1},
            "result": true
        }),
    );
    assert_rpc_error_message(result.unwrap_err(), "response id mismatch");
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

#[tokio::test]
async fn reqwest_transport_sends_api_key_header() {
    let body = r#"{"jsonrpc":"2.0","id":"1","result":{"version":"3.0.0","commit":"abc123"}}"#;
    let server = MockServer::start();
    // The mock only matches when the X-Api-Key header is present with the configured value.
    let mock = server.mock(|when, then| {
        when.method(POST)
            .path("/rpc")
            .header("X-Api-Key", "test-key");
        then.status(200)
            .header("content-type", "application/json")
            .body(body);
    });
    let client = PhantasmaRpc::new(server.url("/rpc")).with_api_key("test-key");

    let result = client.get_version().await.unwrap();

    assert_eq!(result.version, "3.0.0");
    mock.assert();
}

#[test]
fn parse_json_rpc_response_surfaces_http_rejections() {
    // 401 keys-only gate returns {"error":"..."} with no JSON-RPC id; surface status + message,
    // not a misleading "missing id".
    let err = parse_json_rpc_response_for_request(401, json!({ "error": "API key required" }), 7)
        .unwrap_err();
    assert_rpc_error_message(err, "HTTP 401");
    let err = parse_json_rpc_response_for_request(401, json!({ "error": "API key required" }), 7)
        .unwrap_err();
    assert_rpc_error_message(err, "API key required");

    // 429 rate limit with an opaque JSON body.
    let err = parse_json_rpc_response_for_request(429, json!({}), 7).unwrap_err();
    assert_rpc_error_message(err, "HTTP 429");
}
