//! Async JSON-RPC client and response DTOs.
//!
//! The client keeps request construction and response validation centralized so
//! higher-level wrappers inherit the same id checks, error extraction, scalar
//! coercions, and transaction-hash handling.

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::Engine;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::carbon::{
    parse_create_token_result, parse_create_token_series_result, serialize, sign_tx_msg, Bytes32,
    SignedTxMsg, TxMsg,
};
use crate::crypto::PhantasmaKeys;
use crate::encoding::{decode_hex, encode_hex};
use crate::error::{rpc, PhantasmaError, Result};
use crate::transaction::{tx_state_is_fault, tx_state_is_success, Transaction};
use crate::vm::VMObject;

const INITIAL_JSON_RPC_REQUEST_ID: u64 = 1;
pub const DEFAULT_MAX_RPC_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[async_trait]
pub trait RpcTransport: Send + Sync {
    /// Sends one JSON value and returns the HTTP status plus decoded JSON body.
    ///
    /// Tests inject a mock transport here so wrapper behavior can be verified
    /// without a live node.
    async fn post_json(&self, url: &str, body: Value, timeout: Duration) -> Result<(u16, Value)>;
}

#[derive(Debug, Clone)]
pub struct ReqwestTransport {
    client: reqwest::Client,
    max_response_bytes: usize,
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self {
            client: reqwest::Client::default(),
            max_response_bytes: DEFAULT_MAX_RPC_RESPONSE_BYTES,
        }
    }
}

#[async_trait]
impl RpcTransport for ReqwestTransport {
    async fn post_json(&self, url: &str, body: Value, timeout: Duration) -> Result<(u16, Value)> {
        let response = self
            .client
            .post(url)
            .timeout(timeout)
            .json(&body)
            .send()
            .await?;
        let (status, text) = read_limited_response_text(response, self.max_response_bytes).await?;
        let value = serde_json::from_str::<Value>(&text).map_err(|err| {
            if status >= 400 {
                PhantasmaError::Rpc {
                    code: None,
                    message: format!("HTTP {status}: response body is not valid JSON: {err}"),
                }
            } else {
                PhantasmaError::Rpc {
                    code: None,
                    message: format!("response body is not valid JSON: {err}"),
                }
            }
        })?;
        Ok((status, value))
    }
}

async fn read_limited_response_text(
    mut response: reqwest::Response,
    max_response_bytes: usize,
) -> Result<(u16, String)> {
    if max_response_bytes == 0 {
        return rpc(None, "max response size must be positive");
    }
    let status = response.status().as_u16();
    if let Some(content_length) = response.content_length() {
        if content_length > max_response_bytes as u64 {
            return rpc(
                None,
                format!("response body exceeds {max_response_bytes} bytes"),
            );
        }
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > max_response_bytes {
            return rpc(
                None,
                format!("response body exceeds {max_response_bytes} bytes"),
            );
        }
        body.extend_from_slice(&chunk);
    }

    let text = String::from_utf8(body).map_err(|err| PhantasmaError::Rpc {
        code: None,
        message: format!("response body is not valid UTF-8: {err}"),
    })?;
    Ok((status, text))
}

#[derive(Debug, Clone)]
pub struct PhantasmaRpc<T = ReqwestTransport> {
    endpoint: String,
    timeout: Duration,
    transport: T,
    next_request_id: Arc<AtomicU64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RpcCallResult<T> {
    /// Typed SDK model decoded from the JSON-RPC `result` field.
    pub value: T,
    /// Exact parsed JSON value from the JSON-RPC `result` field.
    pub raw_result: Value,
    /// Parsed full JSON-RPC response envelope returned by the transport.
    pub raw_envelope: Value,
    /// Endpoint used for this request.
    pub endpoint: String,
    /// JSON-RPC method name used for this request.
    pub method: String,
    /// HTTP status reported by the transport.
    pub http_status: u16,
    /// Byte length of the parsed JSON-RPC `result` after canonical JSON
    /// serialization. This is not the original byte-for-byte HTTP body length.
    pub canonical_result_bytes: usize,
    /// Byte length of the parsed response envelope after canonical JSON
    /// serialization. This is not the original byte-for-byte HTTP body length.
    pub canonical_envelope_bytes: usize,
}

impl<T> RpcCallResult<T> {
    /// Replaces the typed value while preserving raw response metadata.
    pub fn map_value<U>(self, value: U) -> RpcCallResult<U> {
        RpcCallResult {
            value,
            raw_result: self.raw_result,
            raw_envelope: self.raw_envelope,
            endpoint: self.endpoint,
            method: self.method,
            http_status: self.http_status,
            canonical_result_bytes: self.canonical_result_bytes,
            canonical_envelope_bytes: self.canonical_envelope_bytes,
        }
    }
}

impl PhantasmaRpc<ReqwestTransport> {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout: Duration::from_secs(30),
            transport: ReqwestTransport::default(),
            next_request_id: Arc::new(AtomicU64::new(INITIAL_JSON_RPC_REQUEST_ID)),
        }
    }

    pub fn mainnet() -> Self {
        Self::new("https://pharpc1.phantasma.info/rpc")
    }

    pub fn testnet() -> Self {
        Self::new("https://testnet.phantasma.info/rpc")
    }

    pub fn with_max_response_bytes(mut self, max_response_bytes: usize) -> Self {
        self.transport.max_response_bytes = max_response_bytes;
        self
    }
}

impl<T: RpcTransport> PhantasmaRpc<T> {
    pub fn with_transport(endpoint: impl Into<String>, transport: T) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout: Duration::from_secs(30),
            transport,
            next_request_id: Arc::new(AtomicU64::new(INITIAL_JSON_RPC_REQUEST_ID)),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn next_json_rpc_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn post_json_rpc(&self, method: &str, params: Vec<Value>) -> Result<(u16, Value, u64)> {
        let request_id = self.next_json_rpc_request_id();
        let (status, body) = self
            .transport
            .post_json(
                &self.endpoint,
                json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "method": method,
                    "params": params,
                }),
                self.timeout,
            )
            .await?;
        Ok((status, body, request_id))
    }

    pub async fn call_value_with_raw(
        &self,
        method: &str,
        params: Vec<Value>,
    ) -> Result<RpcCallResult<Value>> {
        let (status, body, request_id) = self.post_json_rpc(method, params).await?;
        let canonical_envelope_bytes = serde_json::to_vec(&body)?.len();
        let raw_result = parse_json_rpc_response_for_request(status, body.clone(), request_id)?;
        let canonical_result_bytes = serde_json::to_vec(&raw_result)?.len();
        Ok(RpcCallResult {
            value: raw_result.clone(),
            raw_result,
            raw_envelope: body,
            endpoint: self.endpoint.clone(),
            method: method.to_string(),
            http_status: status,
            canonical_result_bytes,
            canonical_envelope_bytes,
        })
    }

    pub async fn call_value(&self, method: &str, params: Vec<Value>) -> Result<Value> {
        let (status, body, request_id) = self.post_json_rpc(method, params).await?;
        parse_json_rpc_response_for_request(status, body, request_id)
    }

    pub async fn call_with_raw<R: DeserializeOwned>(
        &self,
        method: &str,
        params: Vec<Value>,
    ) -> Result<RpcCallResult<R>> {
        let response = self.call_value_with_raw(method, params).await?;
        let value = serde_json::from_value(response.raw_result.clone()).map_err(|err| {
            PhantasmaError::Rpc {
                code: None,
                message: err.to_string(),
            }
        })?;
        Ok(response.map_value(value))
    }

    pub async fn call<R: DeserializeOwned>(&self, method: &str, params: Vec<Value>) -> Result<R> {
        let value = self.call_value(method, params).await?;
        serde_json::from_value(value).map_err(|err| PhantasmaError::Rpc {
            code: None,
            message: err.to_string(),
        })
    }

    pub async fn get_version(&self) -> Result<VersionResult> {
        self.call("getVersion", vec![]).await
    }

    pub async fn get_account(&self, address: &str) -> Result<AccountResult> {
        self.call("getAccount", vec![json!(address), json!(false)])
            .await
    }

    pub async fn get_account_with_address_type(
        &self,
        address: &str,
        extended: bool,
        check_address_reserved_byte: bool,
        address_type: &str,
    ) -> Result<AccountResult> {
        self.call(
            "getAccount",
            vec![
                json!(address),
                json!(extended),
                json!(check_address_reserved_byte),
                json!(address_type),
            ],
        )
        .await
    }

    pub async fn get_accounts<S: AsRef<str> + Sync>(
        &self,
        addresses: &[S],
        extended: bool,
    ) -> Result<Vec<AccountResult>> {
        let text = addresses
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join(",");
        self.get_accounts_text(&text, extended).await
    }

    pub async fn get_accounts_text(
        &self,
        addresses: &str,
        extended: bool,
    ) -> Result<Vec<AccountResult>> {
        self.call("getAccounts", vec![json!(addresses), json!(extended)])
            .await
    }

    pub async fn get_accounts_with_address_type(
        &self,
        addresses: &str,
        extended: bool,
        check_address_reserved_byte: bool,
        address_type: &str,
    ) -> Result<Vec<AccountResult>> {
        self.call(
            "getAccounts",
            vec![
                json!(addresses),
                json!(extended),
                json!(check_address_reserved_byte),
                json!(address_type),
            ],
        )
        .await
    }

    pub async fn lookup_name(&self, name: &str) -> Result<String> {
        self.call("lookUpName", vec![json!(name)]).await
    }

    pub async fn look_up_name(&self, name: &str) -> Result<String> {
        self.lookup_name(name).await
    }

    pub async fn get_platforms(&self) -> Result<Vec<PlatformResult>> {
        self.call("getPlatforms", vec![]).await
    }

    pub async fn get_chains(&self, extended: bool) -> Result<Vec<ChainResult>> {
        self.call("getChains", vec![json!(extended)]).await
    }

    pub async fn get_chain(&self, chain: &str, extended: bool) -> Result<ChainResult> {
        self.call("getChain", vec![json!(chain), json!(extended)])
            .await
    }

    pub async fn get_nexus(&self, extended: bool) -> Result<NexusResult> {
        self.call("getNexus", vec![json!(extended)]).await
    }

    pub async fn get_address_transactions(
        &self,
        address: &str,
        page: u32,
        page_size: u32,
    ) -> Result<PaginatedResult<AddressTransactionsResult>> {
        self.call(
            "getAddressTransactions",
            vec![json!(address), json!(page), json!(page_size)],
        )
        .await
    }

    pub async fn get_address_transaction_count(&self, address: &str, chain: &str) -> Result<u64> {
        coerce_u64(
            self.call_value(
                "getAddressTransactionCount",
                vec![json!(address), json!(chain)],
            )
            .await?,
        )
    }

    pub async fn get_block_by_height(&self, chain: &str, height: u64) -> Result<BlockResult> {
        self.call("getBlockByHeight", vec![json!(chain), json!(height)])
            .await
    }

    pub async fn get_block_by_height_with_raw(
        &self,
        chain: &str,
        height: u64,
    ) -> Result<RpcCallResult<BlockResult>> {
        self.call_with_raw("getBlockByHeight", vec![json!(chain), json!(height)])
            .await
    }

    pub async fn get_block_by_hash(&self, hash: &str) -> Result<BlockResult> {
        self.call("getBlockByHash", vec![json!(hash)]).await
    }

    pub async fn get_block_by_hash_with_raw(
        &self,
        hash: &str,
    ) -> Result<RpcCallResult<BlockResult>> {
        self.call_with_raw("getBlockByHash", vec![json!(hash)])
            .await
    }

    pub async fn get_block_transaction_count_by_hash_on_chain(
        &self,
        chain: &str,
        hash: &str,
    ) -> Result<u64> {
        coerce_u64(
            self.call_value(
                "getBlockTransactionCountByHash",
                vec![json!(chain), json!(hash)],
            )
            .await?,
        )
    }

    pub async fn get_block_transaction_count_by_hash(
        &self,
        hash: &str,
        chain: &str,
    ) -> Result<u64> {
        self.get_block_transaction_count_by_hash_on_chain(chain, hash)
            .await
    }

    pub async fn get_latest_block(&self, chain: &str) -> Result<BlockResult> {
        self.call("getLatestBlock", vec![json!(chain)]).await
    }

    pub async fn get_latest_block_with_raw(
        &self,
        chain: &str,
    ) -> Result<RpcCallResult<BlockResult>> {
        self.call_with_raw("getLatestBlock", vec![json!(chain)])
            .await
    }

    pub async fn get_block_height(&self, chain: &str) -> Result<u64> {
        coerce_u64(
            self.call_value("getBlockHeight", vec![json!(chain)])
                .await?,
        )
    }

    pub async fn get_block_height_with_raw(&self, chain: &str) -> Result<RpcCallResult<u64>> {
        let response = self
            .call_value_with_raw("getBlockHeight", vec![json!(chain)])
            .await?;
        let value = coerce_u64(response.raw_result.clone())?;
        Ok(response.map_value(value))
    }

    pub async fn get_transaction_by_block_hash_and_index(
        &self,
        block_hash: &str,
        index: u32,
        chain: &str,
    ) -> Result<TransactionResult> {
        self.call(
            "getTransactionByBlockHashAndIndex",
            vec![json!(chain), json!(block_hash), json!(index)],
        )
        .await
    }

    pub async fn get_transaction_by_block_hash_and_index_with_raw(
        &self,
        block_hash: &str,
        index: u32,
        chain: &str,
    ) -> Result<RpcCallResult<TransactionResult>> {
        self.call_with_raw(
            "getTransactionByBlockHashAndIndex",
            vec![json!(chain), json!(block_hash), json!(index)],
        )
        .await
    }

    pub async fn get_transaction_by_block_hash_and_index_on_chain(
        &self,
        chain: &str,
        block_hash: &str,
        index: u32,
    ) -> Result<TransactionResult> {
        self.get_transaction_by_block_hash_and_index(block_hash, index, chain)
            .await
    }

    pub async fn get_transaction(&self, hash: &str) -> Result<TransactionResult> {
        self.call("getTransaction", vec![json!(hash)]).await
    }

    pub async fn get_transaction_with_raw(
        &self,
        hash: &str,
    ) -> Result<RpcCallResult<TransactionResult>> {
        self.call_with_raw("getTransaction", vec![json!(hash)])
            .await
    }

    pub async fn get_contract(&self, contract_name: &str, chain: &str) -> Result<ContractResult> {
        self.call("getContract", vec![json!(chain), json!(contract_name)])
            .await
    }

    pub async fn get_contract_by_name(&self, chain: &str, name: &str) -> Result<ContractResult> {
        self.get_contract(name, chain).await
    }

    pub async fn get_contract_by_address(
        &self,
        chain: &str,
        address: &str,
    ) -> Result<ContractResult> {
        self.call("getContractByAddress", vec![json!(chain), json!(address)])
            .await
    }

    pub async fn get_contracts(&self, chain: &str, extended: bool) -> Result<Vec<ContractResult>> {
        self.call("getContracts", vec![json!(chain), json!(extended)])
            .await
    }

    pub async fn get_organization(
        &self,
        name: &str,
        include_member_count: bool,
    ) -> Result<OrganizationResult> {
        self.call(
            "getOrganization",
            vec![json!(name), json!(include_member_count)],
        )
        .await
    }

    pub async fn get_organizations(
        &self,
        page_size: u32,
        cursor: &str,
        include_member_count: bool,
    ) -> Result<CursorPaginatedResult<Vec<OrganizationResult>>> {
        self.call(
            "getOrganizations",
            vec![json!(page_size), json!(cursor), json!(include_member_count)],
        )
        .await
    }

    pub async fn get_organization_members(
        &self,
        name: &str,
        page_size: u32,
        cursor: &str,
        include_member_time: bool,
    ) -> Result<CursorPaginatedResult<Vec<OrganizationMemberResult>>> {
        self.call(
            "getOrganizationMembers",
            vec![
                json!(name),
                json!(page_size),
                json!(cursor),
                json!(include_member_time),
            ],
        )
        .await
    }

    pub async fn get_organization_member(
        &self,
        name: &str,
        address: &str,
        check_address_reserved_byte: bool,
        address_type: &str,
    ) -> Result<OrganizationMemberResult> {
        self.call(
            "getOrganizationMember",
            vec![
                json!(name),
                json!(address),
                json!(check_address_reserved_byte),
                json!(address_type),
            ],
        )
        .await
    }

    pub async fn get_leaderboard(&self, name: &str) -> Result<LeaderboardResult> {
        self.call("getLeaderboard", vec![json!(name)]).await
    }

    pub async fn get_token(&self, symbol: &str, extended: bool) -> Result<TokenResult> {
        self.call("getToken", vec![json!(symbol), json!(extended)])
            .await
    }

    pub async fn get_token_with_id(
        &self,
        symbol: &str,
        extended: bool,
        token_id: u64,
    ) -> Result<TokenResult> {
        self.call(
            "getToken",
            vec![json!(symbol), json!(extended), json!(token_id)],
        )
        .await
    }

    pub async fn get_tokens(&self, extended: bool) -> Result<Vec<TokenResult>> {
        self.call("getTokens", vec![json!(extended)]).await
    }

    pub async fn get_tokens_by_owner(
        &self,
        owner: &str,
        extended: bool,
    ) -> Result<Vec<TokenResult>> {
        self.call("getTokens", vec![json!(extended), json!(owner)])
            .await
    }

    pub async fn get_tokens_by_owner_with_address_type(
        &self,
        owner: &str,
        address_type: &str,
        extended: bool,
    ) -> Result<Vec<TokenResult>> {
        self.call(
            "getTokens",
            vec![json!(extended), json!(owner), json!(address_type)],
        )
        .await
    }

    pub async fn get_tokens_as_map(&self, extended: bool) -> Result<HashMap<String, TokenResult>> {
        Ok(self
            .get_tokens(extended)
            .await?
            .into_iter()
            .map(|token| (token.symbol.clone(), token))
            .collect())
    }

    pub async fn get_token_data(&self, symbol: &str, token_id: &str) -> Result<TokenDataResult> {
        self.call("getTokenData", vec![json!(symbol), json!(token_id)])
            .await
    }

    pub async fn get_token_balance(
        &self,
        address: &str,
        symbol: &str,
        chain: &str,
        extended: bool,
    ) -> Result<BalanceResult> {
        self.call(
            "getTokenBalance",
            vec![json!(address), json!(symbol), json!(chain), json!(extended)],
        )
        .await
    }

    pub async fn get_token_balance_with_address_type(
        &self,
        address: &str,
        symbol: &str,
        chain: &str,
        check_address_reserved_byte: bool,
        address_type: &str,
    ) -> Result<BalanceResult> {
        self.call(
            "getTokenBalance",
            vec![
                json!(address),
                json!(symbol),
                json!(chain),
                json!(check_address_reserved_byte),
                json!(address_type),
            ],
        )
        .await
    }

    pub async fn get_token_balance_checked(
        &self,
        address: &str,
        symbol: &str,
        chain: &str,
        check_address_reserved_byte: bool,
    ) -> Result<BalanceResult> {
        self.call(
            "getTokenBalance",
            vec![
                json!(address),
                json!(symbol),
                json!(chain),
                json!(check_address_reserved_byte),
            ],
        )
        .await
    }

    pub async fn get_token_series(
        &self,
        symbol: &str,
        carbon_token_id: u64,
        page_size: u32,
        cursor: &str,
    ) -> Result<CursorPaginatedResult<Vec<TokenSeriesResult>>> {
        self.call(
            "getTokenSeries",
            vec![
                json!(symbol),
                json!(carbon_token_id),
                json!(page_size),
                json!(cursor),
            ],
        )
        .await
    }

    pub async fn get_token_series_by_id(
        &self,
        symbol: &str,
        series_id: &str,
    ) -> Result<TokenSeriesResult> {
        self.get_token_series_by_ids(symbol, 0, series_id, 0).await
    }

    pub async fn get_token_series_by_ids(
        &self,
        symbol: &str,
        carbon_token_id: u64,
        series_id: &str,
        carbon_series_id: u64,
    ) -> Result<TokenSeriesResult> {
        self.call(
            "getTokenSeriesById",
            vec![
                json!(symbol),
                json!(carbon_token_id),
                json!(series_id),
                json!(carbon_series_id),
            ],
        )
        .await
    }

    pub async fn get_token_nfts(
        &self,
        carbon_token_id: u64,
        carbon_series_id: u64,
        page_size: u32,
        cursor: &str,
        extended: bool,
        series_id: &str,
    ) -> Result<CursorPaginatedResult<Vec<TokenDataResult>>> {
        self.call(
            "getTokenNFTs",
            vec![
                json!(carbon_token_id),
                json!(carbon_series_id),
                json!(page_size),
                json!(cursor),
                json!(extended),
                json!(series_id),
            ],
        )
        .await
    }

    pub async fn get_token_nfts_with_series_id(
        &self,
        carbon_token_id: u64,
        carbon_series_id: u64,
        series_id: &str,
        page_size: u32,
        cursor: &str,
        extended: bool,
    ) -> Result<CursorPaginatedResult<Vec<TokenDataResult>>> {
        self.get_token_nfts(
            carbon_token_id,
            carbon_series_id,
            page_size,
            cursor,
            extended,
            series_id,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_account_fungible_tokens(
        &self,
        address: &str,
        symbol: &str,
        carbon_token_id: u64,
        page_size: u32,
        cursor: &str,
        check_address_reserved_byte: bool,
    ) -> Result<CursorPaginatedResult<Vec<BalanceResult>>> {
        self.call(
            "getAccountFungibleTokens",
            vec![
                json!(address),
                json!(symbol),
                json!(carbon_token_id),
                json!(page_size),
                json!(cursor),
                json!(check_address_reserved_byte),
            ],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_account_fungible_tokens_with_address_type(
        &self,
        address: &str,
        symbol: &str,
        carbon_token_id: u64,
        page_size: u32,
        cursor: &str,
        check_address_reserved_byte: bool,
        address_type: &str,
    ) -> Result<CursorPaginatedResult<Vec<BalanceResult>>> {
        self.call(
            "getAccountFungibleTokens",
            vec![
                json!(address),
                json!(symbol),
                json!(carbon_token_id),
                json!(page_size),
                json!(cursor),
                json!(check_address_reserved_byte),
                json!(address_type),
            ],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_account_nfts(
        &self,
        address: &str,
        symbol: &str,
        carbon_token_id: u64,
        carbon_series_id: u64,
        page_size: u32,
        cursor: &str,
        extended: bool,
        check_address_reserved_byte: bool,
    ) -> Result<CursorPaginatedResult<Vec<TokenDataResult>>> {
        self.call(
            "getAccountNFTs",
            vec![
                json!(address),
                json!(symbol),
                json!(carbon_token_id),
                json!(carbon_series_id),
                json!(page_size),
                json!(cursor),
                json!(extended),
                json!(check_address_reserved_byte),
            ],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_account_nfts_with_address_type(
        &self,
        address: &str,
        symbol: &str,
        carbon_token_id: u64,
        carbon_series_id: u64,
        page_size: u32,
        cursor: &str,
        extended: bool,
        check_address_reserved_byte: bool,
        address_type: &str,
    ) -> Result<CursorPaginatedResult<Vec<TokenDataResult>>> {
        self.call(
            "getAccountNFTs",
            vec![
                json!(address),
                json!(symbol),
                json!(carbon_token_id),
                json!(carbon_series_id),
                json!(page_size),
                json!(cursor),
                json!(extended),
                json!(check_address_reserved_byte),
                json!(address_type),
            ],
        )
        .await
    }

    pub async fn get_account_owned_tokens(
        &self,
        address: &str,
        symbol: &str,
        carbon_token_id: u64,
        page_size: u32,
        cursor: &str,
        check_address_reserved_byte: bool,
    ) -> Result<CursorPaginatedResult<Vec<TokenResult>>> {
        self.call(
            "getAccountOwnedTokens",
            vec![
                json!(address),
                json!(symbol),
                json!(carbon_token_id),
                json!(page_size),
                json!(cursor),
                json!(check_address_reserved_byte),
            ],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_account_owned_tokens_with_address_type(
        &self,
        address: &str,
        symbol: &str,
        carbon_token_id: u64,
        page_size: u32,
        cursor: &str,
        check_address_reserved_byte: bool,
        address_type: &str,
    ) -> Result<CursorPaginatedResult<Vec<TokenResult>>> {
        self.call(
            "getAccountOwnedTokens",
            vec![
                json!(address),
                json!(symbol),
                json!(carbon_token_id),
                json!(page_size),
                json!(cursor),
                json!(check_address_reserved_byte),
                json!(address_type),
            ],
        )
        .await
    }

    pub async fn get_account_owned_token_series(
        &self,
        address: &str,
        symbol: &str,
        carbon_token_id: u64,
        page_size: u32,
        cursor: &str,
        check_address_reserved_byte: bool,
    ) -> Result<CursorPaginatedResult<Vec<TokenSeriesResult>>> {
        self.call(
            "getAccountOwnedTokenSeries",
            vec![
                json!(address),
                json!(symbol),
                json!(carbon_token_id),
                json!(page_size),
                json!(cursor),
                json!(check_address_reserved_byte),
            ],
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn get_account_owned_token_series_with_address_type(
        &self,
        address: &str,
        symbol: &str,
        carbon_token_id: u64,
        page_size: u32,
        cursor: &str,
        check_address_reserved_byte: bool,
        address_type: &str,
    ) -> Result<CursorPaginatedResult<Vec<TokenSeriesResult>>> {
        self.call(
            "getAccountOwnedTokenSeries",
            vec![
                json!(address),
                json!(symbol),
                json!(carbon_token_id),
                json!(page_size),
                json!(cursor),
                json!(check_address_reserved_byte),
                json!(address_type),
            ],
        )
        .await
    }

    pub async fn get_auctions_count(&self, chain: &str, symbol: &str) -> Result<u64> {
        coerce_u64(
            self.call_value("getAuctionsCount", vec![json!(chain), json!(symbol)])
                .await?,
        )
    }

    pub async fn get_auctions(
        &self,
        chain: &str,
        symbol: &str,
        page: u32,
        page_size: u32,
    ) -> Result<PaginatedResult<Vec<AuctionResult>>> {
        self.call(
            "getAuctions",
            vec![json!(chain), json!(symbol), json!(page), json!(page_size)],
        )
        .await
    }

    pub async fn get_auction(
        &self,
        chain: &str,
        symbol: &str,
        token_id: &str,
    ) -> Result<AuctionResult> {
        self.call(
            "getAuction",
            vec![json!(chain), json!(symbol), json!(token_id)],
        )
        .await
    }

    pub async fn get_nft(
        &self,
        symbol: &str,
        token_id: &str,
        extended: bool,
    ) -> Result<TokenDataResult> {
        self.call(
            "getNFT",
            vec![json!(symbol), json!(token_id), json!(extended)],
        )
        .await
    }

    pub async fn get_nfts_text(
        &self,
        symbol: &str,
        token_ids: &str,
        extended: bool,
    ) -> Result<Vec<TokenDataResult>> {
        self.call(
            "getNFTs",
            vec![json!(symbol), json!(token_ids), json!(extended)],
        )
        .await
    }

    pub async fn get_nfts<S: AsRef<str> + Sync>(
        &self,
        symbol: &str,
        token_ids: &[S],
        extended: bool,
    ) -> Result<Vec<TokenDataResult>> {
        let text = token_ids
            .iter()
            .map(AsRef::as_ref)
            .collect::<Vec<_>>()
            .join(",");
        self.get_nfts_text(symbol, &text, extended).await
    }

    pub async fn get_archive(&self, hash: &str) -> Result<ArchiveResult> {
        self.call("getArchive", vec![json!(hash)]).await
    }

    pub async fn write_archive_base64(
        &self,
        hash: &str,
        block_index: u32,
        content: &str,
    ) -> Result<bool> {
        coerce_bool(
            self.call_value(
                "writeArchive",
                vec![json!(hash), json!(block_index), json!(content)],
            )
            .await?,
        )
    }

    pub async fn write_archive(
        &self,
        hash: &str,
        block_index: u32,
        content: &[u8],
    ) -> Result<bool> {
        self.write_archive_base64(
            hash,
            block_index,
            &base64::engine::general_purpose::STANDARD.encode(content),
        )
        .await
    }

    pub async fn read_archive(&self, hash: &str, block_index: u32) -> Result<String> {
        self.call("readArchive", vec![json!(hash), json!(block_index)])
            .await
    }

    pub async fn invoke_raw_script(&self, chain: &str, script_hex: &str) -> Result<ScriptResult> {
        self.call("invokeRawScript", vec![json!(chain), json!(script_hex)])
            .await
    }

    pub async fn send_raw_transaction(&self, tx_hex: &str) -> Result<String> {
        extract_hash(
            self.call_value("sendRawTransaction", vec![json!(tx_hex)])
                .await?,
        )
    }

    pub async fn send_transaction(&self, tx: &Transaction) -> Result<String> {
        self.send_raw_transaction(&encode_hex(tx.to_bytes(true)))
            .await
    }

    pub async fn sign_and_send_transaction(
        &self,
        keys: &PhantasmaKeys,
        nexus: &str,
        script: &[u8],
        chain: &str,
        payload: &[u8],
        expiration: Option<u32>,
    ) -> Result<String> {
        let expiration = expiration.unwrap_or_else(default_expiration_seconds);
        let mut tx = Transaction::new(nexus, chain, script.to_vec(), expiration)
            .with_payload(payload.to_vec());
        tx.sign(keys);
        self.send_transaction(&tx).await
    }

    pub async fn sign_and_send_built_transaction(
        &self,
        tx: &mut Transaction,
        keys: &PhantasmaKeys,
    ) -> Result<String> {
        tx.sign(keys);
        self.send_transaction(tx).await
    }

    pub async fn send_carbon_transaction(&self, tx: &[u8]) -> Result<String> {
        extract_hash(
            self.call_value("sendCarbonTransaction", vec![json!(hex::encode(tx))])
                .await?,
        )
    }

    pub async fn send_signed_tx_msg(&self, tx: &SignedTxMsg) -> Result<String> {
        self.send_carbon_transaction(&serialize(tx)?).await
    }

    pub fn sign_carbon_transaction(
        &self,
        msg: &TxMsg,
        keys: &PhantasmaKeys,
    ) -> Result<SignedTxMsg> {
        sign_tx_msg(msg, keys)
    }

    pub async fn sign_and_send_carbon_transaction(
        &self,
        msg: &TxMsg,
        keys: &PhantasmaKeys,
    ) -> Result<String> {
        self.send_signed_tx_msg(&self.sign_carbon_transaction(msg, keys)?)
            .await
    }

    pub async fn build_sign_send_tx_msg(
        &self,
        msg: &TxMsg,
        keys: &PhantasmaKeys,
    ) -> Result<String> {
        self.send_signed_tx_msg(&sign_tx_msg(msg, keys)?).await
    }

    pub async fn send_create_token_tx(
        &self,
        msg: &TxMsg,
        keys: &PhantasmaKeys,
    ) -> Result<(String, Option<u64>)> {
        let tx_hash = self.build_sign_send_tx_msg(msg, keys).await?;
        Ok((tx_hash, None))
    }

    pub async fn get_phantasma_vm_config(&self, chain: &str) -> Result<PhantasmaVmConfigResult> {
        self.call("getPhantasmaVmConfig", vec![json!(chain)]).await
    }

    pub fn parse_create_token_result(&self, result_hex: &str) -> Result<u64> {
        parse_create_token_result(result_hex)
    }

    pub fn parse_create_token_series_result(&self, result_hex: &str) -> Result<u32> {
        parse_create_token_series_result(result_hex)
    }
}

pub fn parse_json_rpc_response(status: u16, body: Value) -> Result<Value> {
    parse_json_rpc_response_for_request(status, body, INITIAL_JSON_RPC_REQUEST_ID)
}

pub fn parse_json_rpc_response_for_request(
    status: u16,
    body: Value,
    expected_request_id: u64,
) -> Result<Value> {
    let Some(object) = body.as_object() else {
        return rpc(None, "JSON-RPC response must be an object");
    };
    let id = object.get("id").ok_or_else(|| PhantasmaError::Rpc {
        code: None,
        message: "missing id".into(),
    })?;
    if !json_rpc_id_matches(id, expected_request_id) {
        return rpc(None, format!("response id mismatch: {id}"));
    }
    if let Some(error) = object.get("error") {
        if let Some(error_object) = error.as_object() {
            let code = error_object.get("code").and_then(Value::as_i64);
            let message = error_object
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("JSON-RPC error");
            return rpc(code, message);
        }
        if let Some(message) = error.as_str() {
            return rpc(None, message);
        }
        return rpc(None, error.to_string());
    }
    if status >= 400 {
        return rpc(None, format!("HTTP {status}"));
    }
    object
        .get("result")
        .cloned()
        .ok_or_else(|| PhantasmaError::Rpc {
            code: None,
            message: "missing result".into(),
        })
}

fn json_rpc_id_matches(id: &Value, expected: u64) -> bool {
    if let Some(number) = id.as_u64() {
        return number == expected;
    }
    if let Some(text) = id.as_str() {
        return text == expected.to_string();
    }
    false
}

fn extract_hash(value: Value) -> Result<String> {
    // Send endpoints have returned both a plain hash string and an object with
    // a `hash` field across RPC implementations. Treat embedded error objects
    // as failures instead of reporting an empty hash.
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    if let Some(object) = value.as_object() {
        if let Some(error) = object.get("error") {
            return rpc(None, error.to_string());
        }
        if let Some(hash) = object.get("hash").and_then(Value::as_str) {
            return Ok(hash.to_string());
        }
    }
    rpc(None, "send transaction response does not contain a hash")
}

fn coerce_u64(value: Value) -> Result<u64> {
    // Several read-only RPC fields are numerically typed in SDK contracts but
    // arrive as strings from current nodes. Coerce only unambiguous positive
    // integer shapes and reject everything else.
    if let Some(value) = value.as_u64() {
        return Ok(value);
    }
    if let Some(value) = value.as_i64() {
        if value >= 0 {
            return Ok(value as u64);
        }
    }
    if let Some(value) = value.as_str() {
        return value.parse::<u64>().map_err(|_| PhantasmaError::Rpc {
            code: None,
            message: format!("expected integer-compatible RPC value, got {value:?}"),
        });
    }
    rpc(
        None,
        format!("expected integer-compatible RPC value, got {value}"),
    )
}

fn coerce_bool(value: Value) -> Result<bool> {
    // Current RPC responses may encode booleans as JSON booleans, 0/1 numbers,
    // or string equivalents. Any other spelling is treated as malformed data.
    if let Some(value) = value.as_bool() {
        return Ok(value);
    }
    if let Some(value) = value.as_i64() {
        return Ok(value != 0);
    }
    if let Some(value) = value.as_str() {
        return match value.to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => rpc(
                None,
                format!("expected boolean-compatible RPC value, got {value:?}"),
            ),
        };
    }
    rpc(
        None,
        format!("expected boolean-compatible RPC value, got {value}"),
    )
}

fn default_expiration_seconds() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_add(20 * 60) as u32
}

pub fn convert_decimals(amount: &str, decimals: u32) -> String {
    if decimals == 0 {
        return amount.to_string();
    }
    let negative = amount.starts_with('-');
    let digits = amount.trim_start_matches('-').trim_start_matches('0');
    let mut digits = if digits.is_empty() {
        "0".to_string()
    } else {
        digits.to_string()
    };
    let decimals = decimals as usize;
    if digits.len() <= decimals {
        digits = format!("{}{}", "0".repeat(decimals + 1 - digits.len()), digits);
    }
    let split = digits.len() - decimals;
    let whole = &digits[..split];
    let fractional = digits[split..].trim_end_matches('0');
    let value = if fractional.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{fractional}")
    };
    if negative && value != "0" {
        format!("-{value}")
    } else {
        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BuildInfoResult {
    pub version: String,
    pub commit: String,
    pub build_time_utc: String,
}

pub type VersionResult = BuildInfoResult;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BalanceResult {
    pub chain: String,
    pub amount: String,
    pub symbol: String,
    pub decimals: u32,
    pub ids: Option<Vec<String>>,
}

impl BalanceResult {
    pub fn decimal_amount(&self) -> String {
        convert_decimals(&self.amount, self.decimals)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct InteropResult {
    pub local: String,
    pub external: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PlatformResult {
    pub platform: String,
    pub chain: String,
    pub fuel: String,
    pub tokens: Vec<String>,
    pub interop: Vec<InteropResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct StakeResult {
    pub amount: String,
    pub time: u64,
    pub unclaimed: String,
}

impl StakeResult {
    pub fn decimal_amount(&self) -> String {
        convert_decimals(&self.amount, 8)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ArchiveResult {
    pub hash: Option<String>,
    pub name: Option<String>,
    pub size: u64,
    pub time: u64,
    pub encryption: Option<String>,
    pub block_count: u64,
    pub missing_blocks: Option<Vec<u64>>,
    pub owners: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct StorageResult {
    pub available: u64,
    pub used: u64,
    pub avatar: String,
    pub archives: Vec<ArchiveResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AccountResult {
    pub address: String,
    pub name: String,
    pub stakes: StakeResult,
    pub stake: String,
    pub unclaimed: String,
    pub relay: Option<String>,
    pub validator: String,
    pub storage: StorageResult,
    pub balances: Vec<BalanceResult>,
    pub txs: Option<Vec<String>>,
}

impl AccountResult {
    pub fn token_balance(&self, symbol: &str) -> Option<&BalanceResult> {
        self.balances
            .iter()
            .find(|balance| balance.symbol == symbol)
    }

    pub fn get_token_balance(&mut self, symbol: &str, decimals: u32) -> &BalanceResult {
        if let Some(index) = self
            .balances
            .iter()
            .position(|balance| balance.symbol == symbol)
        {
            return &self.balances[index];
        }
        self.balances.push(BalanceResult {
            chain: "main".to_string(),
            amount: "0".to_string(),
            symbol: symbol.to_string(),
            decimals,
            ids: None,
        });
        self.balances.last().expect("balance was just inserted")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AddressTransactionsResult {
    pub address: String,
    pub txs: Vec<TransactionResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PaginatedResult<T> {
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub total_pages: u32,
    pub result: Option<T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CursorPaginatedResult<T> {
    pub result: Option<T>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct EventResult {
    pub address: String,
    pub contract: String,
    pub kind: String,
    pub name: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct EventExResult {
    pub address: String,
    pub contract: String,
    pub kind: String,
    pub data: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct OracleResult {
    pub url: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SignatureResult {
    pub kind: String,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TransactionResult {
    pub hash: String,
    pub chain_address: String,
    pub timestamp: u64,
    pub block_height: u64,
    pub block_hash: String,
    pub script: String,
    pub carbon_tx_type: u32,
    pub carbon_tx_data: String,
    pub payload: String,
    pub events: Vec<EventResult>,
    pub extended_events: Vec<EventExResult>,
    pub state: String,
    pub result: String,
    pub debug_comment: Option<String>,
    pub fee: String,
    pub signatures: Vec<SignatureResult>,
    pub sender: String,
    pub gas_payer: String,
    pub gas_target: String,
    pub gas_price: String,
    pub gas_limit: String,
    pub expiration: u64,
}

impl TransactionResult {
    pub fn state_is_success(&self) -> bool {
        tx_state_is_success(&self.state)
    }

    pub fn state_is_fault(&self) -> bool {
        tx_state_is_fault(&self.state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BlockResult {
    pub hash: String,
    pub previous_hash: String,
    pub timestamp: u64,
    pub height: u64,
    pub chain_address: String,
    pub protocol: u32,
    pub txs: Vec<TransactionResult>,
    pub validator_address: String,
    pub reward: String,
    pub events: Option<Vec<EventResult>>,
    pub oracles: Option<Vec<OracleResult>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ContractResult {
    pub name: String,
    pub address: String,
    pub owner: Option<String>,
    pub script: String,
    pub methods: Option<Vec<AbiMethodResult>>,
    pub events: Option<Vec<AbiEventResult>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AbiMethodResult {
    pub name: String,
    pub return_type: String,
    pub parameters: Vec<AbiParameterResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AbiEventResult {
    pub name: String,
    pub value: u32,
    pub return_type: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AbiParameterResult {
    pub name: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct GovernanceResult {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct OrganizationResult {
    pub name: Option<String>,
    pub owner: Option<String>,
    pub carbon_owner: Option<String>,
    pub metadata: Vec<TokenPropertyResult>,
    pub member_count: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct OrganizationMemberResult {
    pub address: Option<String>,
    pub carbon_address: Option<String>,
    pub is_member: bool,
    pub member_time: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CrowdsaleResult {
    pub hash: String,
    pub name: String,
    pub creator: String,
    pub flags: String,
    pub start_date: u64,
    pub end_date: u64,
    pub sell_symbol: String,
    pub receive_symbol: String,
    pub price: u64,
    pub global_soft_cap: String,
    pub global_hard_cap: String,
    pub user_soft_cap: String,
    pub user_hard_cap: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ChainResult {
    pub name: Option<String>,
    pub address: Option<String>,
    pub parent: Option<String>,
    pub height: u64,
    pub organization: Option<String>,
    pub contracts: Option<Vec<String>>,
    pub dapps: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DappResult {
    pub name: String,
    pub address: String,
    pub chain: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NexusResult {
    pub name: Option<String>,
    pub protocol: u32,
    pub platforms: Option<Vec<PlatformResult>>,
    pub tokens: Option<Vec<TokenResult>>,
    pub chains: Option<Vec<ChainResult>>,
    pub governance: Option<Vec<GovernanceResult>>,
    pub organizations: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct LeaderboardRowResult {
    pub address: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct LeaderboardResult {
    pub name: Option<String>,
    pub rows: Option<Vec<LeaderboardRowResult>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenPropertyResult {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenExternalResult {
    pub platform: String,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenPriceResult {
    pub timestamp: u64,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct VmVariableSchemaResult {
    #[serde(rename = "type")]
    pub type_name: String,
    pub schema: Option<VmStructSchemaResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct VmNamedVariableSchemaResult {
    pub name: String,
    pub schema: VmVariableSchemaResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct VmStructSchemaResult {
    pub fields: Vec<VmNamedVariableSchemaResult>,
    pub flags: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenSchemasResult {
    pub series_metadata: VmStructSchemaResult,
    pub rom: VmStructSchemaResult,
    pub ram: VmStructSchemaResult,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenSeriesResult {
    pub series_id: String,
    pub carbon_token_id: String,
    pub carbon_series_id: String,
    pub owner_address: String,
    pub max_mint: String,
    pub mint_count: String,
    pub current_supply: String,
    pub max_supply: String,
    pub burned_supply: Option<String>,
    pub mode: Option<String>,
    pub script: Option<String>,
    pub methods: Option<Vec<AbiMethodResult>>,
    pub metadata: Vec<TokenPropertyResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenResult {
    pub symbol: String,
    pub name: String,
    pub decimals: u32,
    pub current_supply: String,
    pub max_supply: String,
    pub burned_supply: String,
    pub address: String,
    pub owner: String,
    pub flags: String,
    pub script: Option<String>,
    pub series: Vec<TokenSeriesResult>,
    pub carbon_id: String,
    pub metadata: Option<Vec<TokenPropertyResult>>,
    pub token_schemas: Option<TokenSchemasResult>,
    pub external: Option<Vec<TokenExternalResult>>,
    pub price: Option<Vec<TokenPriceResult>>,
}

impl TokenResult {
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.split(',').any(|item| item.trim() == flag)
    }

    pub fn is_fungible(&self) -> bool {
        self.has_flag("Fungible")
    }

    pub fn is_non_fungible(&self) -> bool {
        self.has_flag("NonFungible")
    }

    pub fn is_burnable(&self) -> bool {
        self.has_flag("Burnable")
    }

    pub fn is_divisible(&self) -> bool {
        self.has_flag("Divisible")
    }

    pub fn is_fiat(&self) -> bool {
        self.has_flag("Fiat")
    }

    pub fn is_finite(&self) -> bool {
        self.has_flag("Finite")
    }

    pub fn is_fuel(&self) -> bool {
        self.has_flag("Fuel")
    }

    pub fn is_mintable(&self) -> bool {
        self.has_flag("Mintable")
    }

    pub fn is_stakable(&self) -> bool {
        self.has_flag("Stakable")
    }

    pub fn is_transferable(&self) -> bool {
        self.has_flag("Transferable")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenDataResult {
    pub id: String,
    pub series: String,
    pub carbon_token_id: String,
    pub carbon_series_id: String,
    pub carbon_nft_address: String,
    pub mint: String,
    pub chain_name: String,
    pub owner_address: String,
    pub creator_address: String,
    pub ram: String,
    pub rom: String,
    pub status: String,
    pub infusion: Vec<TokenPropertyResult>,
    pub properties: Vec<TokenPropertyResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NftResult {
    pub id: String,
    pub series: String,
    pub carbon_token_id: String,
    pub carbon_series_id: String,
    pub carbon_nft_address: String,
    pub mint: String,
    pub chain_name: String,
    pub owner_address: String,
    pub creator_address: String,
    pub ram: String,
    pub rom: String,
    pub status: String,
    pub infusion: Vec<TokenPropertyResult>,
    pub properties: Vec<TokenPropertyResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AuctionResult {
    pub creator_address: String,
    pub chain_address: String,
    pub start_date: u64,
    pub end_date: u64,
    pub base_symbol: String,
    pub quote_symbol: String,
    pub token_id: String,
    pub price: String,
    pub end_price: String,
    pub extension_period: String,
    #[serde(rename = "type")]
    pub type_name: String,
    pub rom: String,
    pub ram: String,
    pub listing_fee: String,
    pub current_winner: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ChannelResult {
    pub creator_address: String,
    pub target_address: String,
    pub name: String,
    pub chain: String,
    pub creation_time: u64,
    pub symbol: String,
    pub fee: String,
    pub balance: String,
    pub active: bool,
    pub index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ReceiptResult {
    pub nexus: String,
    pub channel: String,
    pub index: String,
    pub timestamp: u64,
    pub sender: String,
    pub receiver: String,
    pub script: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PeerResult {
    pub url: String,
    pub version: String,
    pub flags: String,
    pub fee: String,
    pub pow: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ValidatorResult {
    pub address: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SwapResult {
    pub source_platform: String,
    pub source_chain: String,
    pub source_hash: String,
    pub source_address: String,
    pub destination_platform: String,
    pub destination_chain: String,
    pub destination_hash: String,
    pub destination_address: String,
    pub symbol: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PhantasmaVmConfigResult {
    pub is_stored: bool,
    pub feature_level: u32,
    pub gas_constructor: String,
    pub gas_nexus: String,
    pub gas_organization: String,
    pub gas_account: String,
    pub gas_leaderboard: String,
    pub gas_standard: String,
    pub gas_oracle: String,
    pub fuel_per_contract_deploy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ScriptResult {
    pub events: Vec<EventResult>,
    pub result: String,
    pub results: Vec<String>,
    pub oracles: Vec<OracleResult>,
    pub error: Option<String>,
    pub state: Option<String>,
    pub gas: Option<String>,
}

impl ScriptResult {
    pub fn decode_result(&self) -> Result<VMObject> {
        VMObject::from_bytes(&decode_hex(&self.result)?)
    }

    pub fn decode_results(&self, index: usize) -> Result<VMObject> {
        let Some(result) = self.results.get(index) else {
            return rpc(None, format!("script result index out of range: {index}"));
        };
        VMObject::from_bytes(&decode_hex(result)?)
    }
}

#[allow(dead_code)]
fn parse_token_create_result_from_tx(tx: &TransactionResult) -> Result<Option<u64>> {
    if tx.result.is_empty() {
        return Ok(None);
    }
    parse_create_token_result(&tx.result).map(Some)
}

#[allow(dead_code)]
fn parse_token_series_result_from_tx(tx: &TransactionResult) -> Result<Option<u32>> {
    if tx.result.is_empty() {
        return Ok(None);
    }
    parse_create_token_series_result(&tx.result).map(Some)
}

#[allow(dead_code)]
fn _keep_imports(_: Bytes32) {}
