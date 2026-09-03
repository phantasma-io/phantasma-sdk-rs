//! A canned node for the RPC-side fee tests: a transport whose answers are scripted - the gas
//! config, the token lookups a pre-flight makes, the account queries an infusion read makes, and
//! the broadcast, which records what it was given.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use async_trait::async_trait;
use phantasma_sdk::{
    build_burn_non_fungible_tx, build_token_info, build_token_metadata, build_transfer_fungible_tx,
    bytes32_from_public_key, deserialize, serialize, BurnNonFungibleParams, Bytes32,
    GovernanceContractMethod, IntX, ModuleId, PhantasmaKeys, PhantasmaRpc, RegisterNameArgs,
    RpcTransport, SignedTxMsg, SmallString, TransferFungibleParams, TxLimits, TxMsg, TxMsgCall,
    TxPayload, TxType,
};
use serde_json::{json, Value};

/// The getGasConfig response of the mainnet gas-model-v2 configuration (special resolution #79).
pub const MAINNET_GAS_CONFIG_JSON: &str = r#"{
  "gasModelVersion": 2,
  "blockRateTarget": 2000,
  "expiryWindow": 3600000,
  "unitsPerBlockDataByte": 25,
  "gasConfig": {
    "version": 1,
    "maxNameLength": 255,
    "maxTokenSymbolLength": 255,
    "feeShift": 0,
    "maxStructureSize": 1048576,
    "feeMultiplier": "10000",
    "gasTokenId": "1",
    "dataTokenId": "2",
    "minimumGasOffer": "10",
    "dataEscrowPerRow": "200000",
    "gasFeeTransfer": "10",
    "gasFeeQuery": "10",
    "gasFeeCreateTokenBase": "10000000000",
    "gasFeeCreateTokenSymbol": "10000000000",
    "gasFeeCreateTokenSeries": "2500000000",
    "gasFeePerByte": "250000",
    "gasFeeRegisterName": "10000000000000",
    "gasBurnRatioMul": "1",
    "gasBurnRatioShift": 0,
    "minimumGasBill": "10000000",
    "gasProducerRatioMul": "0",
    "gasProducerRatioShift": 0,
    "gasDappRatioMul": "0",
    "gasDappRatioShift": 0,
    "policyFeeCreateTokenBase": "100000000000000",
    "policyFeeCreateTokenSymbol": "100000000000000",
    "policyFeeCreateTokenSeries": "25000000000000",
    "policyFeeRegisterName": "100000000000000000",
    "legacyDataEscrowPerRow": "2"
  }
}"#;

pub struct NodeState {
    /// Every request body, as it went on the wire.
    pub requests: Vec<Value>,
    /// The envelopes the node was asked to broadcast, hex.
    pub sent: Vec<String>,
    /// Tokens the node knows, by symbol, as getToken results.
    pub tokens: HashMap<String, Value>,
    pub lookups: usize,
    pub gas_config_reads: usize,
    pub infusion_reads: usize,
    /// What the node answers for something that is not there; changed to simulate a broken node.
    pub lookup_error: String,
    /// Whether the node answers lookups at all; false simulates one that cannot serve getToken.
    pub reachable: bool,
    /// The node's refusal of the broadcast, when set.
    pub send_error: Option<String>,
    /// The node's refusal of getGasConfig, when set.
    pub gas_config_error: Option<String>,
    /// Fungible balances per Carbon-hex address; an address the node knows nothing about is empty.
    pub fungible: HashMap<String, Vec<Value>>,
    /// NFT tokens held per Carbon-hex address, with the instance count each.
    pub owned_nfts: HashMap<String, Vec<(Value, String)>>,
}

#[derive(Clone)]
pub struct CannedNode {
    state: Arc<Mutex<NodeState>>,
}

impl Default for CannedNode {
    fn default() -> Self {
        Self::new()
    }
}

impl CannedNode {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(NodeState {
                requests: Vec::new(),
                sent: Vec::new(),
                tokens: HashMap::new(),
                lookups: 0,
                gas_config_reads: 0,
                infusion_reads: 0,
                lookup_error: "Token symbol not found".into(),
                reachable: true,
                send_error: None,
                gas_config_error: None,
                fungible: HashMap::new(),
                owned_nfts: HashMap::new(),
            })),
        }
    }

    pub fn state(&self) -> MutexGuard<'_, NodeState> {
        self.state.lock().unwrap()
    }

    pub fn client(&self) -> PhantasmaRpc<CannedNode> {
        PhantasmaRpc::with_transport("http://canned.invalid/rpc", self.clone())
    }

    pub fn sent(&self) -> Vec<String> {
        self.state().sent.clone()
    }

    /// The first broadcast envelope, decoded.
    pub fn decode_sent(&self) -> SignedTxMsg {
        let sent = self.sent();
        assert!(!sent.is_empty(), "nothing was sent");
        deserialize(hex::decode(&sent[0]).unwrap()).unwrap()
    }

    fn answer(
        state: &mut NodeState,
        method: &str,
        params: &[Value],
    ) -> Result<Value, (i64, String)> {
        match method {
            "getGasConfig" => {
                state.gas_config_reads += 1;
                if let Some(error) = &state.gas_config_error {
                    return Err((-32603, error.clone()));
                }
                Ok(serde_json::from_str(MAINNET_GAS_CONFIG_JSON).unwrap())
            }
            "getToken" => {
                state.lookups += 1;
                if !state.reachable {
                    return Err((-32603, state.lookup_error.clone()));
                }
                // A live node resolves by id without looking at the symbol; that path is the
                // pre-flight's control, and it answers for the gas token whatever the caller's
                // symbol turns out to be.
                if params.len() == 3 && params[2].as_u64() != Some(0) {
                    return Ok(json!({"symbol": "KCAL", "carbonId": "1"}));
                }
                let symbol = params[0].as_str().unwrap_or_default();
                match state.tokens.get(symbol) {
                    Some(token) => Ok(token.clone()),
                    None => Err((-32603, state.lookup_error.clone())),
                }
            }
            "sendCarbonTransaction" => {
                if let Some(error) = &state.send_error {
                    return Err((-32603, error.clone()));
                }
                state.sent.push(params[0].as_str().unwrap().to_string());
                Ok(json!("HASH"))
            }
            "getAccountFungibleTokens" => {
                state.infusion_reads += 1;
                let address = params[0].as_str().unwrap();
                let balances = state.fungible.get(address).cloned().unwrap_or_default();
                Ok(json!({"result": balances, "cursor": null}))
            }
            "getAccountOwnedTokens" => {
                let address = params[0].as_str().unwrap();
                let tokens: Vec<Value> = state
                    .owned_nfts
                    .get(address)
                    .map(|held| held.iter().map(|(token, _)| token.clone()).collect())
                    .unwrap_or_default();
                Ok(json!({"result": tokens, "cursor": null}))
            }
            "getTokenBalance" => {
                let address = params[0].as_str().unwrap();
                let symbol = params[1].as_str().unwrap();
                let instances = state
                    .owned_nfts
                    .get(address)
                    .and_then(|held| held.iter().find(|(token, _)| token["symbol"] == symbol))
                    .map(|(_, instances)| instances.clone())
                    .unwrap_or_else(|| "0".into());
                Ok(json!({"chain": "main", "symbol": symbol, "amount": instances, "decimals": 0}))
            }
            other => panic!("unexpected RPC method {other}"),
        }
    }
}

#[async_trait]
impl RpcTransport for CannedNode {
    async fn post_json(
        &self,
        _url: &str,
        body: Value,
        _timeout: Duration,
    ) -> phantasma_sdk::Result<(u16, Value)> {
        let mut state = self.state();
        state.requests.push(body.clone());
        let id = body["id"].clone();
        let method = body["method"].as_str().unwrap_or_default().to_string();
        let params = body["params"].as_array().cloned().unwrap_or_default();
        Ok(match Self::answer(&mut state, &method, &params) {
            Ok(result) => (200, json!({"jsonrpc": "2.0", "id": id, "result": result})),
            Err((code, message)) => (
                200,
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
            ),
        })
    }
}

/// Deterministic, non-funded keys: the owner and a gas payer.
pub fn keys() -> (PhantasmaKeys, PhantasmaKeys) {
    (
        PhantasmaKeys::from_wif("KwPpBSByydVKqStGHAnZzQofCqhDmD2bfRgc9BmZqM3ZmsdWJw4d").unwrap(),
        PhantasmaKeys::from_wif("KwVG94yjfVg1YKFyRxAGtug93wdRbmLnqqrFV6Yd2CiA9KZDAp4H").unwrap(),
    )
}

pub fn address_of(keys: &PhantasmaKeys) -> Bytes32 {
    bytes32_from_public_key(&keys.public_key()).unwrap()
}

pub fn transfer_tx(owner: Bytes32, to: Bytes32, gas_payer: Option<Bytes32>, max_gas: u64) -> TxMsg {
    build_transfer_fungible_tx(TransferFungibleParams {
        limits: TxLimits {
            max_gas,
            ..TxLimits::default()
        },
        from: owner,
        gas_payer,
        to,
        token_id: 1,
        amount: 5,
    })
}

pub fn create_token_tx(owner: Bytes32, symbol: &str) -> TxMsg {
    let metadata = build_token_metadata(&[
        ("name", "Send probe"),
        ("icon", "data:image/png;base64,iVBORw0KGgo="),
        ("url", "https://example.invalid/p"),
        ("description", "x"),
    ])
    .unwrap();
    let info = build_token_info(
        symbol,
        IntX::from(0i64),
        false,
        2,
        owner,
        metadata,
        Vec::new(),
    )
    .unwrap();
    phantasma_sdk::build_create_token_tx(info, owner, TxLimits::default()).unwrap()
}

pub fn register_name_tx(owner: Bytes32, name: &str) -> TxMsg {
    let args = RegisterNameArgs {
        address: owner,
        name: SmallString::new(name).unwrap(),
    };
    TxMsg {
        tx_type: TxType::Call,
        expiry: 1_787_000_000_000,
        max_gas: 0,
        max_data: 0,
        gas_from: owner,
        payload: SmallString::default(),
        msg: TxPayload::Call(TxMsgCall {
            module_id: ModuleId::Governance as u32,
            method_id: GovernanceContractMethod::RegisterName as u32,
            args: serialize(&args).unwrap(),
            sections: None,
        }),
    }
}

pub fn burn_tx(owner: Bytes32, token_id: u64, instance_id: u64) -> TxMsg {
    build_burn_non_fungible_tx(BurnNonFungibleParams {
        from: owner,
        token_id,
        instance_id,
        ..BurnNonFungibleParams::default()
    })
}
