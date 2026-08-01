//! Typed extended-event decoding: kind dispatch, per-method argument dispatch, raw fallbacks.
//!
//! Live fixtures were captured from https://devnet.phantasma.info/rpc on 2026-08-01 via
//! getBlockByHeight("main", <height>); the height is stated on each test. Long hex payloads
//! (contract scripts, ABIs, ROMs) are truncated to keep fixtures readable - the field set, field
//! types and every other value are verbatim. Shape-only fixtures for the event kinds without a
//! capturable live sample (token and market events) mirror the node's emission sites in
//! RpcEventBuilder.TokenEvents.cs / RpcEventBuilder.MarketEvents.cs, whose serializer settings
//! (camelCase, enum names as strings, nulls omitted) are the same ones verified live on the
//! special-resolution family.

use std::mem::discriminant;

use phantasma_sdk::{EventExResult, SpecialResolutionArguments, SpecialResolutionCall, VmValue};
use serde_json::{json, Value};

/// Devnet block 8,736,259: one SpecialResolution event whose single call is
/// phantasma_vm.RepairSeries with 3,649 supplements and 8,370 repairs. The fixture keeps the
/// first supplement and the first repair.
fn repair_series_event() -> Value {
    json!({
        "address": "P2KJPTC82NAFEzXg3X4eA83JvyWQ8PJVaBop2fUUsKPBcou",
        "contract": "governance",
        "kind": "SpecialResolution",
        "data": {
            "resolutionId": 37,
            "description": "Special Resolution",
            "calls": [{
                "moduleId": 2,
                "module": "phantasma_vm",
                "methodId": 6,
                "method": "RepairSeries",
                "arguments": {
                    "supplementsCount": "3649",
                    "supplements": [{
                        "token": "BRC",
                        "tokenId": "23",
                        "phantasmaSeriesId": "6472",
                        "maxSupply": "1000",
                        "mintCount": "30",
                        "mode": "1",
                        "script": "0004010D000403524F4D0300",
                        "abi": "080A67657443726561746564",
                        "rom": "010804076372656174656405"
                    }],
                    "repairsCount": "8370",
                    "repairs": [{
                        "token": "CROWN",
                        "tokenId": "4",
                        "phantasmaSeriesId": "0",
                        "importedLiveCount": "10998",
                        "script": "0004000E0000040D01040743",
                        "abi": "04076765744E616D65040100"
                    }]
                }
            }]
        }
    })
}

#[test]
fn special_resolution_repair_series_decodes_from_the_devnet_answer() {
    let event: EventExResult = serde_json::from_value(repair_series_event()).unwrap();

    assert_eq!(event.contract, "governance");
    assert_eq!(event.kind, "SpecialResolution");
    let resolution = event
        .data
        .as_special_resolution()
        .expect("typed resolution");
    assert_eq!(resolution.resolution_id, 37);
    assert_eq!(
        resolution.description.as_deref(),
        Some("Special Resolution")
    );
    assert_eq!(resolution.calls.len(), 1);

    let call = &resolution.calls[0];
    assert_eq!(call.module_id, 2);
    assert_eq!(call.module, "phantasma_vm");
    assert_eq!(call.method_id, 6);
    assert_eq!(call.method, "RepairSeries");
    assert!(call.calls.is_none());

    let Some(SpecialResolutionArguments::RepairSeries(arguments)) = &call.arguments else {
        panic!("RepairSeries arguments should decode to their typed shape");
    };
    assert_eq!(arguments.supplements_count, "3649");
    assert_eq!(arguments.repairs_count, "8370");
    let supplement = &arguments.supplements[0];
    assert_eq!(supplement.token, "BRC");
    assert_eq!(supplement.token_id, "23");
    assert_eq!(supplement.phantasma_series_id, "6472");
    assert_eq!(supplement.max_supply, "1000");
    assert_eq!(supplement.mint_count, "30");
    assert_eq!(supplement.mode, "1");
    let repair = &arguments.repairs[0];
    assert_eq!(repair.token, "CROWN");
    assert_eq!(repair.token_id, "4");
    assert_eq!(repair.phantasma_series_id, "0");
    assert_eq!(repair.imported_live_count, "10998");
}

#[test]
fn special_resolution_round_trips_to_the_wire_shape() {
    // Serializing the decoded event must reproduce the wire object exactly: camelCase names,
    // numeric ids as numbers, string counts as strings, no null keys for the absent nested calls.
    let wire = repair_series_event();
    let event: EventExResult = serde_json::from_value(wire.clone()).unwrap();
    let round_tripped = serde_json::to_value(&event).unwrap();
    assert_eq!(round_tripped, wire);
}

#[test]
fn special_resolution_transfer_fungible_decodes_from_the_devnet_answer() {
    // Devnet block 8,736,266: resolution 44 "Repair imported NFT fungible infusions" carries
    // 9,600 token.TransferFungible calls; this is its first call verbatim.
    let event: EventExResult = serde_json::from_value(json!({
        "address": "P2KJPTC82NAFEzXg3X4eA83JvyWQ8PJVaBop2fUUsKPBcou",
        "contract": "governance",
        "kind": "SpecialResolution",
        "data": {
            "resolutionId": 44,
            "description": "Repair imported NFT fungible infusions",
            "calls": [{
                "moduleId": 1,
                "module": "token",
                "methodId": 0,
                "method": "TransferFungible",
                "arguments": {
                    "from": "S3dPnV8dfdkHDHDcJiHY255FEUZCM7oAmDW78LpYZ4jveGW",
                    "to": "S3dPnV8dfdkHDHDcJiHY255FEUZCM7oAmDW78LpYZ4jveGW",
                    "amount": "10000000000",
                    "token": "KCAL",
                    "tokenId": "1"
                }
            }]
        }
    }))
    .unwrap();

    let resolution = event
        .data
        .as_special_resolution()
        .expect("typed resolution");
    assert_eq!(resolution.resolution_id, 44);
    let Some(SpecialResolutionArguments::TransferFungible(transfer)) =
        &resolution.calls[0].arguments
    else {
        panic!("TransferFungible arguments should decode to their typed shape");
    };
    assert_eq!(
        transfer.from,
        "S3dPnV8dfdkHDHDcJiHY255FEUZCM7oAmDW78LpYZ4jveGW"
    );
    assert_eq!(transfer.to, transfer.from);
    assert_eq!(transfer.amount, "10000000000");
    assert_eq!(transfer.token, "KCAL");
    assert_eq!(transfer.token_id, "1");
}

#[test]
fn import_contracts_decodes_from_the_devnet_answer() {
    // Devnet block 8,736,257: phantasma_vm.ImportContracts restoring 70 contracts. The fixture
    // keeps the "mail" contract (empty storage) and the "pharming" contract's first root
    // variable and first table row.
    let event: EventExResult = serde_json::from_value(json!({
        "address": "P2KJPTC82NAFEzXg3X4eA83JvyWQ8PJVaBop2fUUsKPBcou",
        "contract": "governance",
        "kind": "SpecialResolution",
        "data": {
            "resolutionId": 36,
            "description": "Special Resolution",
            "calls": [{
                "moduleId": 2,
                "module": "phantasma_vm",
                "methodId": 5,
                "method": "ImportContracts",
                "arguments": {
                    "contractsCount": "70",
                    "contracts": [
                        {
                            "name": "mail",
                            "address": "S3d6cUXRwJbudV4ADbRtMz3P9527ts7D2Lh9h2J96m48FPW",
                            "owner": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
                            "script": "0B",
                            "abi": "090B507573684D657373616765",
                            "rootVariables": [],
                            "tables": []
                        },
                        {
                            "name": "pharming",
                            "address": "S3d6cUXRwJbudV4ADbRtMz3P9527ts7D2Lh9h2J96m48FPW",
                            "owner": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
                            "script": "0B",
                            "abi": "0906676574546F6B656E",
                            "rootVariables": [{
                                "key": "6D616E61676572",
                                "value": "0100E9F4F69F677473684D2E201672A6AC30CA8F2A238C68"
                            }],
                            "tables": [{
                                "name": "addrs_kcal_bnb",
                                "rows": [{
                                    "key": "3C003E",
                                    "value": "0104040B534F554C41646472657373"
                                }]
                            }]
                        }
                    ]
                }
            }]
        }
    }))
    .unwrap();

    let resolution = event
        .data
        .as_special_resolution()
        .expect("typed resolution");
    let Some(SpecialResolutionArguments::ImportContracts(imported)) =
        &resolution.calls[0].arguments
    else {
        panic!("ImportContracts arguments should decode to their typed shape");
    };
    assert_eq!(imported.contracts_count, "70");
    assert_eq!(imported.contracts.len(), 2);
    assert_eq!(imported.contracts[0].name, "mail");
    assert!(imported.contracts[0].root_variables.is_empty());
    assert!(imported.contracts[0].tables.is_empty());
    let pharming = &imported.contracts[1];
    assert_eq!(pharming.name, "pharming");
    assert_eq!(pharming.root_variables[0].key, "6D616E61676572");
    assert_eq!(pharming.tables[0].name, "addrs_kcal_bnb");
    assert_eq!(pharming.tables[0].rows[0].key, "3C003E");
}

#[test]
fn token_create_data_decodes_and_round_trips() {
    // Shape fixture per RpcEventBuilder.TokenEvents.cs:153: carbonTokenId is a JSON number,
    // isNonFungible a boolean, metadata a string-to-string object with keys kept verbatim.
    let wire = json!({
        "address": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
        "contract": "token",
        "kind": "TokenCreate",
        "data": {
            "symbol": "BRC",
            "maxSupply": "0",
            "decimals": 0,
            "isNonFungible": true,
            "carbonTokenId": 23,
            "metadata": {"_iib": "1", "name": "Bricks"}
        }
    });
    let event: EventExResult = serde_json::from_value(wire.clone()).unwrap();

    let created = event.data.as_token_create().expect("typed token creation");
    assert_eq!(created.symbol, "BRC");
    assert_eq!(created.max_supply, "0");
    assert_eq!(created.decimals, 0);
    assert!(created.is_non_fungible);
    assert_eq!(created.carbon_token_id, 23);
    assert_eq!(created.metadata["name"], "Bricks");
    assert_eq!(created.metadata["_iib"], "1");

    assert_eq!(serde_json::to_value(&event).unwrap(), wire);
}

#[test]
fn token_series_create_data_decodes() {
    // Shape fixture per RpcEventBuilder.TokenEvents.cs:360. seriesId is the big-integer
    // Phantasma id as a string; carbonSeriesId is the numeric Carbon id.
    let event: EventExResult = serde_json::from_value(json!({
        "address": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
        "contract": "token",
        "kind": "TokenSeriesCreate",
        "data": {
            "symbol": "BRC",
            "seriesId": "6472",
            "maxMint": 30,
            "maxSupply": 1000,
            "owner": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
            "carbonTokenId": 23,
            "carbonSeriesId": 7,
            "metadata": {"seriesId": "6472", "name": "Bricks S1"}
        }
    }))
    .unwrap();

    let series = event
        .data
        .as_token_series_create()
        .expect("typed series creation");
    assert_eq!(series.series_id, "6472");
    assert_eq!(series.max_mint, 30);
    assert_eq!(series.max_supply, 1000);
    assert_eq!(series.carbon_token_id, 23);
    assert_eq!(series.carbon_series_id, 7);
    assert_eq!(series.metadata["seriesId"], "6472");
}

#[test]
fn market_order_data_decodes_for_each_order_kind() {
    // Shape fixture per RpcEventBuilder.MarketEvents.cs:387: one payload shape for all three
    // order kinds. On a cancel the node reports the seller as buyer to keep the shape stable.
    let data = json!({
        "baseSymbol": "CROWN",
        "quoteSymbol": "SOUL",
        "tokenId": "108166370197722688972979742015215678103",
        "carbonBaseTokenId": 4,
        "carbonQuoteTokenId": 0,
        "carbonInstanceId": 12,
        "seller": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
        "buyer": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
        "price": "1000000000",
        "endPrice": "0",
        "startDate": 1753900000,
        "endDate": 1754000000,
        "type": "Fixed"
    });

    for kind in ["OrderCreated", "OrderCancelled", "OrderFilled"] {
        let event: EventExResult = serde_json::from_value(json!({
            "address": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
            "contract": "market",
            "kind": kind,
            "data": data.clone()
        }))
        .unwrap();

        let order = event
            .data
            .as_market_order()
            .unwrap_or_else(|| panic!("{kind} should decode to the market-order shape"));
        assert_eq!(order.token_id, "108166370197722688972979742015215678103");
        assert_eq!(order.carbon_base_token_id, 4);
        assert_eq!(order.carbon_instance_id, 12);
        assert_eq!(order.buyer, order.seller);
        assert_eq!(order.start_date, 1753900000);
        assert_eq!(order.type_name, "Fixed");
    }
}

#[test]
fn arguments_dispatch_covers_every_module_method_pair() {
    use SpecialResolutionArguments as Args;

    // One row per module/method pair the node decodes, mirroring the reference converter's map.
    // An empty object is enough to pick the variant: every argument struct fills missing fields
    // with defaults, so this table pins the dispatch itself.
    let expected: &[(&str, &str, Args)] = &[
        (
            "governance",
            "SetGasConfig",
            Args::GasConfig(Default::default()),
        ),
        (
            "governance",
            "SetChainConfig",
            Args::ChainConfig(Default::default()),
        ),
        (
            "governance",
            "SpecialResolution",
            Args::NestedResolution(Default::default()),
        ),
        (
            "governance",
            "SetMetadata",
            Args::Metadata(Default::default()),
        ),
        (
            "governance",
            "SetNodeConfig",
            Args::NodeConfig(Default::default()),
        ),
        (
            "governance",
            "RegisterName",
            Args::RegisterName(Default::default()),
        ),
        (
            "governance",
            "LookupName",
            Args::Address(Default::default()),
        ),
        (
            "governance",
            "LookupAddress",
            Args::Name(Default::default()),
        ),
        (
            "phantasma_vm",
            "ExecuteScript",
            Args::ExecuteScript(Default::default()),
        ),
        (
            "phantasma_vm",
            "RegisterTokenContract",
            Args::RegisterTokenContract(Default::default()),
        ),
        (
            "phantasma_vm",
            "DeployContract",
            Args::DeployContract(Default::default()),
        ),
        (
            "phantasma_vm",
            "IsContractDeployed",
            Args::Name(Default::default()),
        ),
        (
            "phantasma_vm",
            "SetConfig",
            Args::PhantasmaVmConfig(Default::default()),
        ),
        (
            "phantasma_vm",
            "ImportContracts",
            Args::ImportContracts(Default::default()),
        ),
        (
            "phantasma_vm",
            "RepairSeries",
            Args::RepairSeries(Default::default()),
        ),
        (
            "phantasma_vm",
            "RepairToken",
            Args::RepairToken(Default::default()),
        ),
        (
            "token",
            "TransferFungible",
            Args::TransferFungible(Default::default()),
        ),
        (
            "token",
            "TransferNonFungible",
            Args::TransferNonFungible(Default::default()),
        ),
        (
            "token",
            "CreateToken",
            Args::CreateToken(Default::default()),
        ),
        (
            "token",
            "MintFungible",
            Args::MintFungible(Default::default()),
        ),
        (
            "token",
            "BurnFungible",
            Args::BurnFungible(Default::default()),
        ),
        ("token", "GetBalance", Args::Balance(Default::default())),
        (
            "token",
            "CreateTokenSeries",
            Args::TokenSeries(Default::default()),
        ),
        (
            "token",
            "DeleteTokenSeries",
            Args::TokenSeriesReference(Default::default()),
        ),
        (
            "token",
            "MintNonFungible",
            Args::MintNonFungible(Default::default()),
        ),
        (
            "token",
            "BurnNonFungible",
            Args::BurnNonFungible(Default::default()),
        ),
        (
            "token",
            "GetNonFungibleInfo",
            Args::NonFungibleInfo(Default::default()),
        ),
        (
            "token",
            "GetNonFungibleInfoByRomId",
            Args::NonFungibleInfoByRomId(Default::default()),
        ),
        (
            "token",
            "GetSeriesInfo",
            Args::TokenSeriesReference(Default::default()),
        ),
        (
            "token",
            "GetSeriesInfoByMetaId",
            Args::SeriesInfoByMetaId(Default::default()),
        ),
        (
            "token",
            "GetTokenInfo",
            Args::TokenReference(Default::default()),
        ),
        (
            "token",
            "GetTokenInfoBySymbol",
            Args::Symbol(Default::default()),
        ),
        (
            "token",
            "GetTokenSupply",
            Args::TokenReference(Default::default()),
        ),
        (
            "token",
            "GetSeriesSupply",
            Args::TokenSeriesReference(Default::default()),
        ),
        (
            "token",
            "GetTokenIdBySymbol",
            Args::Symbol(Default::default()),
        ),
        ("token", "GetBalances", Args::Address(Default::default())),
        (
            "token",
            "CreateMintedTokenSeries",
            Args::CreateMintedTokenSeries(Default::default()),
        ),
        (
            "token",
            "ApplyInflation",
            Args::TokenReference(Default::default()),
        ),
        (
            "token",
            "UpdateTokenMetadata",
            Args::UpdateTokenMetadata(Default::default()),
        ),
        (
            "token",
            "GetNextTokenInflation",
            Args::TokenReference(Default::default()),
        ),
        (
            "token",
            "SetTokensConfig",
            Args::TokensConfig(Default::default()),
        ),
        (
            "token",
            "UpdateSeriesMetadata",
            Args::UpdateSeriesMetadata(Default::default()),
        ),
        (
            "token",
            "MintPhantasmaNonFungible",
            Args::MintPhantasmaNonFungible(Default::default()),
        ),
    ];
    assert_eq!(
        expected.len(),
        43,
        "the node decodes 43 module/method pairs"
    );

    for (module, method, expected_arguments) in expected {
        let call: SpecialResolutionCall = serde_json::from_value(json!({
            "moduleId": 0,
            "module": module,
            "methodId": 0,
            "method": method,
            "arguments": {}
        }))
        .unwrap();
        let arguments = call
            .arguments
            .unwrap_or_else(|| panic!("{module}.{method} should produce typed arguments"));
        assert_eq!(
            discriminant(&arguments),
            discriminant(expected_arguments),
            "{module}.{method} decoded to {arguments:?}"
        );
    }
}

#[test]
fn raw_args_win_over_a_known_method() {
    // The undecoded case is recognised by content, not by method name: an older node can answer
    // a raw dump for a method this build models, and typing it would produce an object with
    // every field empty.
    let call: SpecialResolutionCall = serde_json::from_value(json!({
        "moduleId": 1,
        "module": "token",
        "methodId": 0,
        "method": "TransferFungible",
        "arguments": {"rawArgs": "AABBCC"}
    }))
    .unwrap();

    let raw = call
        .arguments
        .as_ref()
        .and_then(SpecialResolutionArguments::as_raw)
        .expect("rawArgs content wins over the method map");
    assert_eq!(raw.raw_args, "AABBCC");
}

#[test]
fn unknown_method_and_kind_payloads_stay_verbatim() {
    // A module/method pair outside the map keeps its JSON: dropping it would lose answered data
    // whenever the node is newer than this SDK.
    let arguments = json!({"futureField": ["x", {"y": 1}]});
    let call: SpecialResolutionCall = serde_json::from_value(json!({
        "moduleId": 9,
        "module": "governance",
        "methodId": 99,
        "method": "MethodOfANewerNode",
        "arguments": arguments.clone()
    }))
    .unwrap();
    assert_eq!(
        call.arguments
            .as_ref()
            .and_then(SpecialResolutionArguments::as_unrecognized),
        Some(&arguments)
    );

    // Same rule one level up: an event kind outside the modeled set keeps its payload.
    let data = json!({"anything": true});
    let event: EventExResult = serde_json::from_value(json!({
        "address": "PADDR",
        "contract": "future",
        "kind": "KindOfANewerNode",
        "data": data.clone()
    }))
    .unwrap();
    assert_eq!(event.data.as_unknown(), Some(&data));
}

#[test]
fn mismatched_known_shapes_fall_back_to_raw_variants() {
    // A modeled kind whose payload no longer matches the modeled shape must not fail the whole
    // answer; it degrades to the raw variant, where the kind/variant mismatch is detectable.
    let drifted = json!({"symbol": "SOUL", "carbonTokenId": "not-a-number"});
    let event: EventExResult = serde_json::from_value(json!({
        "address": "PADDR",
        "contract": "token",
        "kind": "TokenCreate",
        "data": drifted.clone()
    }))
    .unwrap();
    assert_eq!(event.data.as_unknown(), Some(&drifted));

    // Same for arguments that are not even an object.
    let call: SpecialResolutionCall = serde_json::from_value(json!({
        "moduleId": 1,
        "module": "token",
        "methodId": 0,
        "method": "TransferFungible",
        "arguments": ["not", "an", "object"]
    }))
    .unwrap();
    assert_eq!(
        call.arguments
            .as_ref()
            .and_then(SpecialResolutionArguments::as_unrecognized),
        Some(&json!(["not", "an", "object"]))
    );
}

#[test]
fn nested_resolution_calls_decode_recursively() {
    // governance.SpecialResolution nests another resolution: its id arrives as a string in the
    // arguments (unlike the numeric envelope id) and its calls arrive in the carrying call's
    // "calls", per the reference converter.
    let call: SpecialResolutionCall = serde_json::from_value(json!({
        "moduleId": 0,
        "module": "governance",
        "methodId": 7,
        "method": "SpecialResolution",
        "arguments": {"resolutionId": "7"},
        "calls": [{
            "moduleId": 1,
            "module": "token",
            "methodId": 0,
            "method": "TransferFungible",
            "arguments": {
                "from": "S3dA",
                "to": "S3dB",
                "amount": "5",
                "token": "SOUL",
                "tokenId": "0"
            }
        }]
    }))
    .unwrap();

    let Some(SpecialResolutionArguments::NestedResolution(nested)) = &call.arguments else {
        panic!("nested resolution arguments should decode to their typed shape");
    };
    assert_eq!(nested.resolution_id, "7");

    let nested_calls = call.calls.as_ref().expect("nested calls are present");
    let Some(SpecialResolutionArguments::TransferFungible(transfer)) = &nested_calls[0].arguments
    else {
        panic!("nested call arguments should dispatch like top-level ones");
    };
    assert_eq!(transfer.amount, "5");
}

#[test]
fn create_token_arguments_carry_vm_metadata() {
    // token.CreateToken metadata values are VM values, not plain strings: the interest array of
    // getToken("SOUL", true) is the shape that motivated VmValue (devnet, 2026-08-01).
    let call: SpecialResolutionCall = serde_json::from_value(json!({
        "moduleId": 1,
        "module": "token",
        "methodId": 2,
        "method": "CreateToken",
        "arguments": {
            "symbol": "SOUL",
            "owner": "P2KFNXEbt65rQiWqogAzqkVGMqFirPmqPw8mQyxvRKsrXV8",
            "maxSupply": "0",
            "decimals": "8",
            "flags": "199",
            "metadata": {"_ia": [{"mul": "25", "div": "10000"}], "name": "Phantasma Stake"}
        }
    }))
    .unwrap();

    let Some(SpecialResolutionArguments::CreateToken(created)) = &call.arguments else {
        panic!("CreateToken arguments should decode to their typed shape");
    };
    assert_eq!(created.decimals, "8");
    let metadata = created.metadata.as_ref().expect("metadata is present");
    assert_eq!(metadata["name"].as_text(), Some("Phantasma Stake"));
    let interest = metadata["_ia"].as_items().expect("_ia stays an array");
    assert_eq!(
        interest[0].field("mul").and_then(VmValue::as_text),
        Some("25")
    );
}

#[test]
fn gas_config_v2_tail_is_optional() {
    let base = json!({
        "version": "0",
        "maxNameLength": "255",
        "maxTokenSymbolLength": "10",
        "feeShift": "10",
        "maxStructureSize": "65535",
        "feeMultiplier": "16",
        "gasTokenId": "1",
        "dataTokenId": "0",
        "minimumGasOffer": "10000",
        "dataEscrowPerRow": "1000000",
        "gasFeeTransfer": "1000",
        "gasFeeQuery": "100",
        "gasFeeCreateTokenBase": "100000000",
        "gasFeeCreateTokenSymbol": "10000000",
        "gasFeeCreateTokenSeries": "1000000",
        "gasFeePerByte": "10",
        "gasFeeRegisterName": "100000",
        "gasBurnRatioMul": "1",
        "gasBurnRatioShift": "1"
    });

    let call = |arguments: Value| -> SpecialResolutionCall {
        serde_json::from_value(json!({
            "moduleId": 0,
            "module": "governance",
            "methodId": 0,
            "method": "SetGasConfig",
            "arguments": arguments
        }))
        .unwrap()
    };

    // A version 0 config has no v2 tail on the wire; the options stay None and serializing the
    // call must not invent null keys for them.
    let v1_call = call(base.clone());
    let Some(SpecialResolutionArguments::GasConfig(v1)) = &v1_call.arguments else {
        panic!("SetGasConfig arguments should decode to their typed shape");
    };
    assert_eq!(v1.fee_multiplier, "16");
    assert_eq!(v1.minimum_gas_bill, None);
    let serialized = serde_json::to_value(&v1_call).unwrap();
    assert_eq!(serialized["arguments"], base);

    let mut with_tail = base;
    with_tail["minimumGasBill"] = json!("21000");
    with_tail["gasProducerRatioMul"] = json!("45");
    let v2_call = call(with_tail);
    let Some(SpecialResolutionArguments::GasConfig(v2)) = &v2_call.arguments else {
        panic!("SetGasConfig arguments should decode to their typed shape");
    };
    assert_eq!(v2.minimum_gas_bill.as_deref(), Some("21000"));
    assert_eq!(v2.gas_producer_ratio_mul.as_deref(), Some("45"));
}

#[test]
fn absent_and_null_arguments_stay_none() {
    // The node omits absent arguments (nulls are dropped by its serializer); both spellings
    // must land on None rather than a fabricated empty shape.
    let without: SpecialResolutionCall = serde_json::from_value(json!({
        "moduleId": 1,
        "module": "token",
        "methodId": 0,
        "method": "TransferFungible"
    }))
    .unwrap();
    assert!(without.arguments.is_none());
    assert!(without.calls.is_none());

    let with_null: SpecialResolutionCall = serde_json::from_value(json!({
        "moduleId": 1,
        "module": "token",
        "methodId": 0,
        "method": "TransferFungible",
        "arguments": null
    }))
    .unwrap();
    assert!(with_null.arguments.is_none());
}

#[test]
fn special_resolution_envelope_tolerates_missing_fields() {
    // Response DTOs are default-tolerant across this crate; an empty data object decodes to the
    // empty resolution instead of failing.
    let event: EventExResult = serde_json::from_value(json!({
        "address": "PADDR",
        "contract": "governance",
        "kind": "SpecialResolution",
        "data": {}
    }))
    .unwrap();

    let resolution = event
        .data
        .as_special_resolution()
        .expect("typed resolution");
    assert_eq!(resolution.resolution_id, 0);
    assert_eq!(resolution.description, None);
    assert!(resolution.calls.is_empty());
}
