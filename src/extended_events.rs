//! Typed extended-event data carried by `extendedEvents[]` of a transaction answer.
//!
//! The node reconstructs these payloads from chain state and serializes them with camelCase
//! names, enum names as strings, and nulls omitted. The `kind` of the carrying event decides the
//! shape of `data`; inside a special resolution, the `module` and `method` of each call decide the
//! shape of its `arguments`. Both dispatches happen during deserialization, so a consumer pattern
//! matches instead of walking untyped JSON.
//!
//! Two rules keep decoding total, so one unexpected event can never fail a whole block answer:
//! - a payload whose kind, module, or method this build does not model arrives verbatim in
//!   [`EventData::Unknown`] or [`SpecialResolutionArguments::Unrecognized`];
//! - a payload that names a modeled shape but does not match it falls back to the same raw
//!   variants instead of erroring, so a newer node's field changes degrade to raw JSON, not to a
//!   dead client. A `kind` that names a modeled shape while the variant is `Unknown` is how a
//!   consumer detects that drift.
//!
//! Numeric fields follow the wire exactly: chain amounts, counts and big-integer ids travel as
//! strings (JSON numbers lose precision above 2^53 in JavaScript consumers), while Carbon-side
//! ids (`carbonTokenId`, `moduleId`, `resolutionId`, timestamps) are plain JSON numbers.

use std::collections::BTreeMap;

use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

use crate::rpc::VmValue;

/// Data of one extended event, typed by the `kind` of the event that carries it.
///
/// The market order kinds (`OrderCreated`, `OrderCancelled`, `OrderFilled`) share
/// [`MarketOrderData`]; the carrying event's `kind` string tells them apart.
///
/// ```
/// use phantasma_sdk::{EventData, EventExResult};
///
/// let event: EventExResult = serde_json::from_str(
///     r#"{"address":"P2K...","contract":"governance","kind":"SpecialResolution",
///         "data":{"resolutionId":44,"description":"Repair","calls":[]}}"#,
/// )
/// .unwrap();
/// match &event.data {
///     EventData::SpecialResolution(resolution) => assert_eq!(resolution.resolution_id, 44),
///     other => panic!("wrong shape: {other:?}"),
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventData {
    TokenCreate(TokenCreateData),
    TokenSeriesCreate(TokenSeriesCreateData),
    MarketOrder(MarketOrderData),
    SpecialResolution(SpecialResolutionData),
    /// Payload of an event kind this build does not model, or a modeled kind whose payload did
    /// not match the modeled shape; the JSON is preserved verbatim either way.
    Unknown(Value),
}

impl EventData {
    /// Types a raw `data` payload by the `kind` of the event that carries it.
    ///
    /// A kind outside the set the node emits with extended data, and a payload that fails its
    /// modeled shape, both keep the raw JSON (see the module documentation for why decoding
    /// must be total).
    pub(crate) fn from_kind_and_json(kind: &str, data: Value) -> Self {
        fn typed<T: for<'de> Deserialize<'de>>(
            data: Value,
            wrap: impl FnOnce(T) -> EventData,
        ) -> EventData {
            // Deserializing from a reference keeps ownership of the payload for the fallback.
            match T::deserialize(&data) {
                Ok(parsed) => wrap(parsed),
                Err(_) => EventData::Unknown(data),
            }
        }

        match kind {
            "TokenCreate" => typed(data, EventData::TokenCreate),
            "TokenSeriesCreate" => typed(data, EventData::TokenSeriesCreate),
            "OrderCreated" | "OrderCancelled" | "OrderFilled" => {
                typed(data, EventData::MarketOrder)
            }
            "SpecialResolution" => typed(data, EventData::SpecialResolution),
            _ => EventData::Unknown(data),
        }
    }

    /// Returns the token-creation data, or `None` for any other shape.
    pub fn as_token_create(&self) -> Option<&TokenCreateData> {
        match self {
            EventData::TokenCreate(data) => Some(data),
            _ => None,
        }
    }

    /// Returns the series-creation data, or `None` for any other shape.
    pub fn as_token_series_create(&self) -> Option<&TokenSeriesCreateData> {
        match self {
            EventData::TokenSeriesCreate(data) => Some(data),
            _ => None,
        }
    }

    /// Returns the market-order data, or `None` for any other shape.
    pub fn as_market_order(&self) -> Option<&MarketOrderData> {
        match self {
            EventData::MarketOrder(data) => Some(data),
            _ => None,
        }
    }

    /// Returns the special-resolution data, or `None` for any other shape.
    pub fn as_special_resolution(&self) -> Option<&SpecialResolutionData> {
        match self {
            EventData::SpecialResolution(data) => Some(data),
            _ => None,
        }
    }

    /// Returns the raw payload of an unmodeled or mismatched event, or `None` for a typed shape.
    pub fn as_unknown(&self) -> Option<&Value> {
        match self {
            EventData::Unknown(value) => Some(value),
            _ => None,
        }
    }
}

impl Default for EventData {
    fn default() -> Self {
        EventData::Unknown(Value::Null)
    }
}

impl Serialize for EventData {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        // The wire has no wrapper around the payload: the carrying event's kind is the only
        // discriminator, so each variant serializes as its bare shape.
        match self {
            EventData::TokenCreate(data) => data.serialize(serializer),
            EventData::TokenSeriesCreate(data) => data.serialize(serializer),
            EventData::MarketOrder(data) => data.serialize(serializer),
            EventData::SpecialResolution(data) => data.serialize(serializer),
            EventData::Unknown(value) => value.serialize(serializer),
        }
    }
}

/// Data of a `TokenCreate` extended event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenCreateData {
    pub symbol: String,
    pub max_supply: String,
    pub decimals: u32,
    pub is_non_fungible: bool,
    pub carbon_token_id: u64,
    /// Metadata rendered to strings by the node; keys arrive exactly as the chain stores them.
    pub metadata: BTreeMap<String, String>,
}

/// Data of a `TokenSeriesCreate` extended event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenSeriesCreateData {
    pub symbol: String,
    /// Phantasma series id, a big integer rendered as a string.
    pub series_id: String,
    pub max_mint: u32,
    pub max_supply: u32,
    pub owner: String,
    pub carbon_token_id: u64,
    pub carbon_series_id: u32,
    /// Metadata rendered to strings by the node; keys arrive exactly as the chain stores them.
    pub metadata: BTreeMap<String, String>,
}

/// Data of an `OrderCreated`, `OrderCancelled` or `OrderFilled` extended event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MarketOrderData {
    pub base_symbol: String,
    pub quote_symbol: String,
    /// Phantasma NFT id, a big integer rendered as a string.
    pub token_id: String,
    pub carbon_base_token_id: u64,
    pub carbon_quote_token_id: u64,
    pub carbon_instance_id: u64,
    pub seller: String,
    /// On a cancel the node reports the seller here as well: the cancel path has no buyer by
    /// definition and the payload shape stays stable.
    pub buyer: String,
    pub price: String,
    pub end_price: String,
    pub start_date: i64,
    pub end_date: i64,
    /// Auction type name, for example `Fixed`.
    #[serde(rename = "type")]
    pub type_name: String,
}

/// Data of a `SpecialResolution` extended event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpecialResolutionData {
    pub resolution_id: u64,
    pub description: Option<String>,
    pub calls: Vec<SpecialResolutionCall>,
}

impl Serialize for SpecialResolutionData {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        // Hand-written to omit an absent description, matching the node's null-omitting wire.
        let fields = 2 + usize::from(self.description.is_some());
        let mut map = serializer.serialize_map(Some(fields))?;
        map.serialize_entry("resolutionId", &self.resolution_id)?;
        if let Some(description) = &self.description {
            map.serialize_entry("description", description)?;
        }
        map.serialize_entry("calls", &self.calls)?;
        map.end()
    }
}

impl<'de> Deserialize<'de> for SpecialResolutionData {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let Value::Object(mut object) = value else {
            return Ok(Self::default());
        };
        let calls = match object.remove("calls") {
            Some(Value::Array(items)) => items
                .into_iter()
                .map(SpecialResolutionCall::from_json)
                .collect(),
            _ => Vec::new(),
        };
        Ok(Self {
            resolution_id: object
                .get("resolutionId")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            description: object.remove("description").and_then(|value| match value {
                Value::String(text) => Some(text),
                _ => None,
            }),
            calls,
        })
    }
}

/// One call carried by a special resolution.
///
/// `arguments` is typed per method: the deserializer picks the concrete
/// [`SpecialResolutionArguments`] from `module` and `method`. `calls` carries the calls of a
/// nested resolution and is absent everywhere else.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpecialResolutionCall {
    pub module_id: u32,
    pub module: String,
    pub method_id: u32,
    pub method: String,
    pub arguments: Option<SpecialResolutionArguments>,
    pub calls: Option<Vec<SpecialResolutionCall>>,
}

impl SpecialResolutionCall {
    /// Builds one call from its JSON object; anything else becomes the empty default, mirroring
    /// the reference SDK's null handling.
    ///
    /// Recursion over nested calls is bounded by the JSON parser itself: serde_json rejects
    /// input nested deeper than its own recursion limit before a `Value` ever reaches this.
    fn from_json(value: Value) -> Self {
        let Value::Object(mut object) = value else {
            return Self::default();
        };
        let module = match object.get("module").and_then(Value::as_str) {
            Some(module) => module.to_string(),
            None => String::new(),
        };
        let method = match object.get("method").and_then(Value::as_str) {
            Some(method) => method.to_string(),
            None => String::new(),
        };
        let calls = match object.remove("calls") {
            Some(Value::Array(items)) => Some(items.into_iter().map(Self::from_json).collect()),
            _ => None,
        };
        let arguments = object
            .remove("arguments")
            .and_then(|value| SpecialResolutionArguments::from_call_json(&module, &method, value));
        Self {
            module_id: id_field(&object, "moduleId"),
            module,
            method_id: id_field(&object, "methodId"),
            method,
            arguments,
            calls,
        }
    }
}

/// Reads a numeric id field; a missing or non-numeric value is 0, as in the reference SDK.
fn id_field(object: &serde_json::Map<String, Value>, name: &str) -> u32 {
    object
        .get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0)
}

impl Serialize for SpecialResolutionCall {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        // Hand-written to omit absent arguments and nested calls, matching the node's wire.
        let fields = 4 + usize::from(self.arguments.is_some()) + usize::from(self.calls.is_some());
        let mut map = serializer.serialize_map(Some(fields))?;
        map.serialize_entry("moduleId", &self.module_id)?;
        map.serialize_entry("module", &self.module)?;
        map.serialize_entry("methodId", &self.method_id)?;
        map.serialize_entry("method", &self.method)?;
        if let Some(arguments) = &self.arguments {
            map.serialize_entry("arguments", arguments)?;
        }
        if let Some(calls) = &self.calls {
            map.serialize_entry("calls", calls)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for SpecialResolutionCall {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        Ok(Self::from_json(Value::deserialize(deserializer)?))
    }
}

/// Decoded arguments of one call inside a special resolution, typed per module and method.
///
/// Shapes that repeat across methods share one variant on purpose: a query by token id looks the
/// same whichever query it is. [`SpecialResolutionArguments::Raw`] carries the argument buffer of
/// a call the answering node itself could not decode; it is recognised by its content (a
/// `rawArgs` field), not by the method name, because an older node can answer a raw dump for a
/// method this build models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpecialResolutionArguments {
    /// The answering node did not decode this call: the raw argument buffer as hex.
    Raw(RawArguments),
    /// Boxed because the gas config is by far the widest shape, and one enum size is paid by
    /// every call of a resolution - including the thousands of small transfers a repair carries.
    GasConfig(Box<GasConfigArguments>),
    ChainConfig(ChainConfigArguments),
    NestedResolution(NestedResolutionArguments),
    Metadata(MetadataArguments),
    NodeConfig(NodeConfigArguments),
    RegisterName(RegisterNameArguments),
    Address(AddressArguments),
    Name(NameArguments),
    ExecuteScript(ExecuteScriptArguments),
    RegisterTokenContract(RegisterTokenContractArguments),
    DeployContract(DeployContractArguments),
    PhantasmaVmConfig(PhantasmaVmConfigArguments),
    ImportContracts(ImportContractsArguments),
    RepairSeries(RepairSeriesArguments),
    RepairToken(RepairTokenArguments),
    TokenReference(TokenReferenceArguments),
    TokenSeriesReference(TokenSeriesReferenceArguments),
    Symbol(SymbolArguments),
    TransferFungible(TransferFungibleArguments),
    TransferNonFungible(TransferNonFungibleArguments),
    MintFungible(MintFungibleArguments),
    BurnFungible(BurnFungibleArguments),
    Balance(BalanceArguments),
    CreateToken(CreateTokenArguments),
    TokenSeries(TokenSeriesArguments),
    CreateMintedTokenSeries(CreateMintedTokenSeriesArguments),
    MintNonFungible(MintNonFungibleArguments),
    MintPhantasmaNonFungible(MintPhantasmaNonFungibleArguments),
    BurnNonFungible(BurnNonFungibleArguments),
    NonFungibleInfo(NonFungibleInfoArguments),
    NonFungibleInfoByRomId(NonFungibleInfoByRomIdArguments),
    SeriesInfoByMetaId(SeriesInfoByMetaIdArguments),
    TokensConfig(TokensConfigArguments),
    UpdateTokenMetadata(UpdateTokenMetadataArguments),
    UpdateSeriesMetadata(UpdateSeriesMetadataArguments),
    /// Arguments of a module/method pair this build does not model, or a modeled pair whose
    /// payload did not match its shape; the JSON is preserved verbatim either way. The reference
    /// SDK drops these, this crate keeps them so no answered data is ever lost.
    Unrecognized(Value),
}

impl SpecialResolutionArguments {
    /// Types the `arguments` object of one call from the call's module and method.
    ///
    /// `None` only for an explicit JSON null, which the node never writes (it omits absent
    /// arguments, and the caller maps an absent field to `None` before this runs).
    fn from_call_json(module: &str, method: &str, value: Value) -> Option<Self> {
        if value.is_null() {
            return None;
        }
        let Some(object) = value.as_object() else {
            return Some(SpecialResolutionArguments::Unrecognized(value));
        };
        // Content check before method dispatch; see the enum documentation.
        if object.contains_key("rawArgs") {
            return Some(match RawArguments::deserialize(&value) {
                Ok(raw) => SpecialResolutionArguments::Raw(raw),
                Err(_) => SpecialResolutionArguments::Unrecognized(value),
            });
        }

        fn typed<T: for<'de> Deserialize<'de>>(
            value: Value,
            wrap: impl FnOnce(T) -> SpecialResolutionArguments,
        ) -> SpecialResolutionArguments {
            // Deserializing from a reference keeps ownership of the payload for the fallback.
            match T::deserialize(&value) {
                Ok(parsed) => wrap(parsed),
                Err(_) => SpecialResolutionArguments::Unrecognized(value),
            }
        }

        use SpecialResolutionArguments as Args;
        // One arm per module/method pair the node decodes; the map mirrors the reference SDK's
        // converter and the node's SpecialResolutionHelper.
        Some(match (module, method) {
            ("governance", "SetGasConfig") => {
                typed(value, |arguments| Args::GasConfig(Box::new(arguments)))
            }
            ("governance", "SetChainConfig") => typed(value, Args::ChainConfig),
            ("governance", "SpecialResolution") => typed(value, Args::NestedResolution),
            ("governance", "SetMetadata") => typed(value, Args::Metadata),
            ("governance", "SetNodeConfig") => typed(value, Args::NodeConfig),
            ("governance", "RegisterName") => typed(value, Args::RegisterName),
            ("governance", "LookupName") => typed(value, Args::Address),
            ("governance", "LookupAddress") => typed(value, Args::Name),
            ("phantasma_vm", "ExecuteScript") => typed(value, Args::ExecuteScript),
            ("phantasma_vm", "RegisterTokenContract") => typed(value, Args::RegisterTokenContract),
            ("phantasma_vm", "DeployContract") => typed(value, Args::DeployContract),
            ("phantasma_vm", "IsContractDeployed") => typed(value, Args::Name),
            ("phantasma_vm", "SetConfig") => typed(value, Args::PhantasmaVmConfig),
            ("phantasma_vm", "ImportContracts") => typed(value, Args::ImportContracts),
            ("phantasma_vm", "RepairSeries") => typed(value, Args::RepairSeries),
            ("phantasma_vm", "RepairToken") => typed(value, Args::RepairToken),
            ("token", "TransferFungible") => typed(value, Args::TransferFungible),
            ("token", "TransferNonFungible") => typed(value, Args::TransferNonFungible),
            ("token", "CreateToken") => typed(value, Args::CreateToken),
            ("token", "MintFungible") => typed(value, Args::MintFungible),
            ("token", "BurnFungible") => typed(value, Args::BurnFungible),
            ("token", "GetBalance") => typed(value, Args::Balance),
            ("token", "CreateTokenSeries") => typed(value, Args::TokenSeries),
            ("token", "DeleteTokenSeries") => typed(value, Args::TokenSeriesReference),
            ("token", "MintNonFungible") => typed(value, Args::MintNonFungible),
            ("token", "BurnNonFungible") => typed(value, Args::BurnNonFungible),
            ("token", "GetNonFungibleInfo") => typed(value, Args::NonFungibleInfo),
            ("token", "GetNonFungibleInfoByRomId") => typed(value, Args::NonFungibleInfoByRomId),
            ("token", "GetSeriesInfo") => typed(value, Args::TokenSeriesReference),
            ("token", "GetSeriesInfoByMetaId") => typed(value, Args::SeriesInfoByMetaId),
            ("token", "GetTokenInfo") => typed(value, Args::TokenReference),
            ("token", "GetTokenInfoBySymbol") => typed(value, Args::Symbol),
            ("token", "GetTokenSupply") => typed(value, Args::TokenReference),
            ("token", "GetSeriesSupply") => typed(value, Args::TokenSeriesReference),
            ("token", "GetTokenIdBySymbol") => typed(value, Args::Symbol),
            ("token", "GetBalances") => typed(value, Args::Address),
            ("token", "CreateMintedTokenSeries") => typed(value, Args::CreateMintedTokenSeries),
            ("token", "ApplyInflation") => typed(value, Args::TokenReference),
            ("token", "UpdateTokenMetadata") => typed(value, Args::UpdateTokenMetadata),
            ("token", "GetNextTokenInflation") => typed(value, Args::TokenReference),
            ("token", "SetTokensConfig") => typed(value, Args::TokensConfig),
            ("token", "UpdateSeriesMetadata") => typed(value, Args::UpdateSeriesMetadata),
            ("token", "MintPhantasmaNonFungible") => typed(value, Args::MintPhantasmaNonFungible),
            _ => SpecialResolutionArguments::Unrecognized(value),
        })
    }

    /// Returns the raw argument buffer of an undecoded call, or `None` for a typed shape.
    pub fn as_raw(&self) -> Option<&RawArguments> {
        match self {
            SpecialResolutionArguments::Raw(raw) => Some(raw),
            _ => None,
        }
    }

    /// Returns the preserved JSON of an unmodeled or mismatched call, or `None` for a typed shape.
    pub fn as_unrecognized(&self) -> Option<&Value> {
        match self {
            SpecialResolutionArguments::Unrecognized(value) => Some(value),
            _ => None,
        }
    }
}

impl Serialize for SpecialResolutionArguments {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        // The wire has no wrapper around the arguments: module and method of the carrying call
        // are the only discriminator, so each variant serializes as its bare shape.
        use SpecialResolutionArguments as Args;
        match self {
            Args::Raw(arguments) => arguments.serialize(serializer),
            Args::GasConfig(arguments) => arguments.serialize(serializer),
            Args::ChainConfig(arguments) => arguments.serialize(serializer),
            Args::NestedResolution(arguments) => arguments.serialize(serializer),
            Args::Metadata(arguments) => arguments.serialize(serializer),
            Args::NodeConfig(arguments) => arguments.serialize(serializer),
            Args::RegisterName(arguments) => arguments.serialize(serializer),
            Args::Address(arguments) => arguments.serialize(serializer),
            Args::Name(arguments) => arguments.serialize(serializer),
            Args::ExecuteScript(arguments) => arguments.serialize(serializer),
            Args::RegisterTokenContract(arguments) => arguments.serialize(serializer),
            Args::DeployContract(arguments) => arguments.serialize(serializer),
            Args::PhantasmaVmConfig(arguments) => arguments.serialize(serializer),
            Args::ImportContracts(arguments) => arguments.serialize(serializer),
            Args::RepairSeries(arguments) => arguments.serialize(serializer),
            Args::RepairToken(arguments) => arguments.serialize(serializer),
            Args::TokenReference(arguments) => arguments.serialize(serializer),
            Args::TokenSeriesReference(arguments) => arguments.serialize(serializer),
            Args::Symbol(arguments) => arguments.serialize(serializer),
            Args::TransferFungible(arguments) => arguments.serialize(serializer),
            Args::TransferNonFungible(arguments) => arguments.serialize(serializer),
            Args::MintFungible(arguments) => arguments.serialize(serializer),
            Args::BurnFungible(arguments) => arguments.serialize(serializer),
            Args::Balance(arguments) => arguments.serialize(serializer),
            Args::CreateToken(arguments) => arguments.serialize(serializer),
            Args::TokenSeries(arguments) => arguments.serialize(serializer),
            Args::CreateMintedTokenSeries(arguments) => arguments.serialize(serializer),
            Args::MintNonFungible(arguments) => arguments.serialize(serializer),
            Args::MintPhantasmaNonFungible(arguments) => arguments.serialize(serializer),
            Args::BurnNonFungible(arguments) => arguments.serialize(serializer),
            Args::NonFungibleInfo(arguments) => arguments.serialize(serializer),
            Args::NonFungibleInfoByRomId(arguments) => arguments.serialize(serializer),
            Args::SeriesInfoByMetaId(arguments) => arguments.serialize(serializer),
            Args::TokensConfig(arguments) => arguments.serialize(serializer),
            Args::UpdateTokenMetadata(arguments) => arguments.serialize(serializer),
            Args::UpdateSeriesMetadata(arguments) => arguments.serialize(serializer),
            Args::Unrecognized(value) => value.serialize(serializer),
        }
    }
}

/// Arguments of a call the answering node could not decode: the raw argument buffer as hex.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RawArguments {
    pub raw_args: String,
}

// Governance module arguments.

/// Arguments of `governance.SetGasConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct GasConfigArguments {
    pub version: String,
    pub max_name_length: String,
    pub max_token_symbol_length: String,
    pub fee_shift: String,
    pub max_structure_size: String,
    pub fee_multiplier: String,
    pub gas_token_id: String,
    pub data_token_id: String,
    pub minimum_gas_offer: String,
    pub data_escrow_per_row: String,
    pub gas_fee_transfer: String,
    pub gas_fee_query: String,
    pub gas_fee_create_token_base: String,
    pub gas_fee_create_token_symbol: String,
    pub gas_fee_create_token_series: String,
    pub gas_fee_per_byte: String,
    pub gas_fee_register_name: String,
    pub gas_burn_ratio_mul: String,
    pub gas_burn_ratio_shift: String,
    // Gas-model-v2 tail: present only when the packaged config declares version >= 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_gas_bill: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_producer_ratio_mul: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_producer_ratio_shift: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_dapp_ratio_mul: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_dapp_ratio_shift: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_fee_create_token_base: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_fee_create_token_symbol: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_fee_create_token_series: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy_fee_register_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_data_escrow_per_row: Option<String>,
}

/// Arguments of `governance.SetChainConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ChainConfigArguments {
    pub version: String,
    pub reserved1: String,
    pub reserved2: String,
    pub reserved3: String,
    pub allowed_tx_types: String,
    pub expiry_window: String,
    pub block_rate_target: String,
}

/// Arguments of `governance.SpecialResolution`: a resolution nested inside another one. Its own
/// calls are reported in the carrying call's `calls`, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NestedResolutionArguments {
    /// Rendered as a string here, unlike the numeric `resolutionId` of the resolution envelope.
    pub resolution_id: String,
}

/// Arguments of `governance.SetMetadata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MetadataArguments {
    pub metadata: BTreeMap<String, VmValue>,
}

/// One consensus node of a `governance.SetNodeConfig` call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ConsensusNode {
    pub id: String,
    #[serde(rename = "type")]
    pub type_name: String,
}

/// Arguments of `governance.SetNodeConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NodeConfigArguments {
    pub nodes: Vec<ConsensusNode>,
}

/// Arguments of `governance.RegisterName`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RegisterNameArguments {
    pub address: String,
    pub name: String,
}

/// A single address argument, shared by `governance.LookupName` and `token.GetBalances`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AddressArguments {
    pub address: String,
}

/// A single name argument, shared by `governance.LookupAddress` and
/// `phantasma_vm.IsContractDeployed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NameArguments {
    pub name: String,
}

// Phantasma VM module arguments.

/// Arguments of `phantasma_vm.ExecuteScript`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ExecuteScriptArguments {
    pub max_gas: String,
    pub gas_from: String,
    pub script: String,
}

/// Arguments of `phantasma_vm.RegisterTokenContract`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RegisterTokenContractArguments {
    pub token_id: String,
    pub symbol: String,
    pub script: String,
    pub abi: String,
    /// Resolved token symbol; absent when the token could not be resolved at answer time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Arguments of `phantasma_vm.DeployContract`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct DeployContractArguments {
    pub from: String,
    pub contract_name: String,
    pub script: String,
    pub abi: String,
}

/// Arguments of `phantasma_vm.SetConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PhantasmaVmConfigArguments {
    pub feature_level: String,
    pub gas_constructor: String,
    pub gas_nexus: String,
    pub gas_organization: String,
    pub gas_account: String,
    pub gas_leaderboard: String,
    pub gas_standard: String,
    pub gas_oracle: String,
    pub fuel_per_contract_deploy: String,
}

/// A key/value row of contract storage, both sides hex-encoded because they hold arbitrary bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ContractStorageRow {
    pub key: String,
    pub value: String,
}

/// One map or list table of a contract, with every row it carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ContractStorageTable {
    pub name: String,
    pub rows: Vec<ContractStorageRow>,
}

/// One contract restored by a migration: identity, code and the whole of its stored state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportedContract {
    pub name: String,
    pub address: String,
    pub owner: String,
    pub script: String,
    pub abi: String,
    /// Root-level contract variables.
    pub root_variables: Vec<ContractStorageRow>,
    /// Map and list tables, including their backing rows.
    pub tables: Vec<ContractStorageTable>,
}

/// Arguments of `phantasma_vm.ImportContracts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportContractsArguments {
    pub contracts_count: String,
    pub contracts: Vec<ImportedContract>,
}

/// Definition needed to rebuild one Phantasma series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SeriesSupplement {
    pub token: String,
    pub token_id: String,
    pub phantasma_series_id: String,
    pub max_supply: String,
    pub mint_count: String,
    pub mode: String,
    pub script: String,
    pub abi: String,
    pub rom: String,
}

/// Mint-count repair of one Phantasma series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SeriesMintCountRepair {
    pub token: String,
    pub token_id: String,
    pub phantasma_series_id: String,
    pub imported_live_count: String,
    pub script: String,
    pub abi: String,
}

/// Arguments of `phantasma_vm.RepairSeries`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RepairSeriesArguments {
    pub supplements_count: String,
    pub supplements: Vec<SeriesSupplement>,
    pub repairs_count: String,
    pub repairs: Vec<SeriesMintCountRepair>,
}

/// Repair of one token definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenRepair {
    pub token: String,
    pub token_id: String,
    pub symbol: String,
    pub script: String,
    pub abi: String,
    pub token_flags: String,
    /// Bitmask of the repair operations the chain was asked to perform. Kept numeric on purpose:
    /// a new chain-side operation must not silently render as an unrelated name here.
    pub repair_mask: String,
}

/// Arguments of `phantasma_vm.RepairToken`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct RepairTokenArguments {
    pub repairs_count: String,
    pub repairs: Vec<TokenRepair>,
}

// Token module arguments.

/// Token identity, shared by every query that addresses a token: the resolved symbol plus the
/// numeric id it was resolved from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenReferenceArguments {
    pub token: String,
    pub token_id: String,
}

/// Token and series identity, shared by every query that addresses a series.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenSeriesReferenceArguments {
    pub series_id: String,
    pub token: String,
    pub token_id: String,
}

/// A single symbol argument, shared by `token.GetTokenInfoBySymbol` and
/// `token.GetTokenIdBySymbol`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SymbolArguments {
    pub symbol: String,
}

/// Arguments of `token.TransferFungible`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TransferFungibleArguments {
    pub from: String,
    pub to: String,
    pub amount: String,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.TransferNonFungible`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TransferNonFungibleArguments {
    pub from: String,
    pub to: String,
    pub instance_ids: Vec<String>,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.MintFungible`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MintFungibleArguments {
    pub to: String,
    pub amount: String,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.BurnFungible`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BurnFungibleArguments {
    pub from: String,
    pub amount: String,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.GetBalance`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BalanceArguments {
    pub address: String,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.CreateToken`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CreateTokenArguments {
    pub symbol: String,
    pub owner: String,
    pub max_supply: String,
    pub decimals: String,
    pub flags: String,
    /// Decoded metadata fields; absent when the token carries none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, VmValue>>,
    /// NFT schema blob as hex; absent for fungible tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_schemas: Option<String>,
}

/// Series definition, as carried by `token.CreateTokenSeries`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenSeriesArguments {
    pub owner: String,
    pub max_mint: String,
    pub max_supply: String,
    /// Decoded series metadata; absent when the token declares no schema for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, VmValue>>,
    /// Phantasma series id taken from the decoded metadata, when the schema carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_id: Option<String>,
    /// Metadata blob as hex, reported instead of `metadata` when it cannot be decoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_raw: Option<String>,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.CreateMintedTokenSeries`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct CreateMintedTokenSeriesArguments {
    pub recipient: String,
    pub roms: Vec<String>,
    pub rams: Vec<String>,
    pub owner: String,
    pub max_mint: String,
    pub max_supply: String,
    /// Decoded series metadata; absent when the token declares no schema for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, VmValue>>,
    /// Phantasma series id taken from the decoded metadata, when the schema carries one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series_id: Option<String>,
    /// Metadata blob as hex, reported instead of `metadata` when it cannot be decoded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_raw: Option<String>,
    pub token: String,
    pub token_id: String,
}

/// One NFT to mint, addressed by the carbon series id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NftMint {
    pub series_id: String,
    pub rom: String,
    pub ram: String,
}

/// One NFT to mint, addressed by the 32-byte Phantasma series id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct PhantasmaNftMint {
    pub phantasma_series_id: String,
    pub rom: String,
    pub ram: String,
}

/// Arguments of `token.MintNonFungible`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MintNonFungibleArguments {
    pub owner: String,
    pub tokens: Vec<NftMint>,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.MintPhantasmaNonFungible`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct MintPhantasmaNonFungibleArguments {
    pub owner: String,
    pub tokens: Vec<PhantasmaNftMint>,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.BurnNonFungible`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct BurnNonFungibleArguments {
    pub address: String,
    pub instance_ids: Vec<String>,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.GetNonFungibleInfo`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NonFungibleInfoArguments {
    pub instance_id: String,
    pub get_schemas: String,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.GetNonFungibleInfoByRomId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct NonFungibleInfoByRomIdArguments {
    pub rom_id: String,
    pub get_schemas: String,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.GetSeriesInfoByMetaId`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct SeriesInfoByMetaIdArguments {
    pub rom_id: String,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.SetTokensConfig`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct TokensConfigArguments {
    pub flags: String,
    /// Names of the flags that are set, including a Reserved0xNN entry for unknown bits.
    pub flags_names: Vec<String>,
}

/// Arguments of `token.UpdateTokenMetadata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdateTokenMetadataArguments {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, VmValue>>,
    pub token: String,
    pub token_id: String,
}

/// Arguments of `token.UpdateSeriesMetadata`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdateSeriesMetadataArguments {
    pub series_id: String,
    /// Metadata blob as hex: this call carries it unschematized.
    pub metadata: String,
    pub token: String,
    pub token_id: String,
}
