//! Coverage of the planner's input space. The closure is taken over what the planner ACCEPTS, not
//! over what the chain models: every message type, every fee kind and every modelled call method
//! needs a decision here. The live matrix runs against one chain and one build; these tables are what fail on a laptop the moment a branch
//! appears with no decision behind it.
//!
//! Three closures, because no two of them can see the third. A message type says nothing about which
//! call method a Call carries, and a call method that maps to an existing fee kind adds nothing to
//! the kinds a run has seen. A `Script` entry in either table is a decision that had to be written
//! down, never a default nobody chose.

mod common;

use std::collections::HashSet;

use phantasma_sdk::{
    build_token_info, build_token_metadata, plan_fees, required_witnesses, serialize,
    BurnFungibleArgs, BurnNonFungibleArgs, Bytes32, FeePlanOptions, GovernanceContractMethod, IntX,
    MintFungibleArgs, MintPhantasmaNonFungibleArgs, ModuleId, NativeFeeKind, PhantasmaNFTMintInfo,
    RegisterNameArgs, SeriesInfo, SmallString, TokenContractMethod, TransferFungibleArgs,
    TransferNonFungibleArgs, TxMsg, TxMsgBurnFungible, TxMsgBurnFungibleGasPayer,
    TxMsgBurnNonFungible, TxMsgBurnNonFungibleGasPayer, TxMsgCall, TxMsgCallMulti,
    TxMsgMintFungible, TxMsgMintNonFungible, TxMsgPhantasma, TxMsgPhantasmaRaw, TxMsgTrade,
    TxMsgTransferFungible, TxMsgTransferFungibleGasPayer, TxMsgTransferNonFungibleMulti,
    TxMsgTransferNonFungibleMultiGasPayer, TxMsgTransferNonFungibleSingle,
    TxMsgTransferNonFungibleSingleGasPayer, TxPayload, TxType,
};

use common::{address_of, base_tx, keys, v2_config};

const ICON: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==";

/// Arbitrary bytes. A method the planner does not model reads none of its arguments, so these are
/// enough for it; a modelled method would fail to read them, which is what makes the distinction
/// visible.
const PROBE_ARGS: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

fn burn_fungible_call(owner: Bytes32) -> TxMsgCall {
    TxMsgCall {
        module_id: ModuleId::Token as u32,
        method_id: TokenContractMethod::BurnFungible as u32,
        args: serialize(&BurnFungibleArgs {
            token_id: 9,
            from_address: owner,
            amount: IntX::from(1i64),
        })
        .unwrap(),
        sections: None,
    }
}

/// One row of the message-type table: a message, and the operations the planner makes of it. `None`
/// says the planner refuses the type.
fn type_cases() -> Vec<(TxType, TxMsg, Option<Vec<NativeFeeKind>>)> {
    let (owner, other) = keys();
    let from = address_of(&owner);
    let to = address_of(&other);
    let call = base_tx(
        TxType::Call,
        from,
        TxPayload::Call(burn_fungible_call(from)),
    );
    vec![
        (
            TxType::Call,
            call.clone(),
            Some(vec![NativeFeeKind::BurnFungible]),
        ),
        (
            TxType::CallMulti,
            base_tx(
                TxType::CallMulti,
                from,
                TxPayload::CallMulti(TxMsgCallMulti {
                    calls: vec![burn_fungible_call(from), burn_fungible_call(from)],
                }),
            ),
            Some(vec![
                NativeFeeKind::BurnFungible,
                NativeFeeKind::BurnFungible,
            ]),
        ),
        // A Trade packs its operations into named arrays and not into calls. Nothing reads them yet,
        // so it takes the script budget, and that is a decision rather than an omission.
        (
            TxType::Trade,
            base_tx(TxType::Trade, from, TxPayload::Trade(TxMsgTrade::default())),
            Some(vec![NativeFeeKind::Script]),
        ),
        (
            TxType::TransferFungible,
            base_tx(
                TxType::TransferFungible,
                from,
                TxPayload::TransferFungible(TxMsgTransferFungible {
                    to,
                    token_id: 9,
                    amount: 1,
                }),
            ),
            Some(vec![NativeFeeKind::TransferFungible]),
        ),
        (
            TxType::TransferFungibleGasPayer,
            base_tx(
                TxType::TransferFungibleGasPayer,
                from,
                TxPayload::TransferFungibleGasPayer(TxMsgTransferFungibleGasPayer {
                    to,
                    from_address: to,
                    token_id: 9,
                    amount: 1,
                }),
            ),
            Some(vec![NativeFeeKind::TransferFungible]),
        ),
        (
            TxType::TransferNonFungibleSingle,
            base_tx(
                TxType::TransferNonFungibleSingle,
                from,
                TxPayload::TransferNonFungibleSingle(TxMsgTransferNonFungibleSingle {
                    to,
                    token_id: 9,
                    instance_id: 1,
                }),
            ),
            Some(vec![NativeFeeKind::TransferNonFungible]),
        ),
        (
            TxType::TransferNonFungibleSingleGasPayer,
            base_tx(
                TxType::TransferNonFungibleSingleGasPayer,
                from,
                TxPayload::TransferNonFungibleSingleGasPayer(
                    TxMsgTransferNonFungibleSingleGasPayer {
                        to,
                        from_address: to,
                        token_id: 9,
                        instance_id: 1,
                    },
                ),
            ),
            Some(vec![NativeFeeKind::TransferNonFungible]),
        ),
        (
            TxType::TransferNonFungibleMulti,
            base_tx(
                TxType::TransferNonFungibleMulti,
                from,
                TxPayload::TransferNonFungibleMulti(TxMsgTransferNonFungibleMulti {
                    to,
                    token_id: 9,
                    instance_ids: vec![1, 2],
                }),
            ),
            Some(vec![NativeFeeKind::TransferNonFungible]),
        ),
        (
            TxType::TransferNonFungibleMultiGasPayer,
            base_tx(
                TxType::TransferNonFungibleMultiGasPayer,
                from,
                TxPayload::TransferNonFungibleMultiGasPayer(
                    TxMsgTransferNonFungibleMultiGasPayer {
                        to,
                        from_address: to,
                        token_id: 9,
                        instance_ids: vec![1, 2],
                    },
                ),
            ),
            Some(vec![NativeFeeKind::TransferNonFungible]),
        ),
        (
            TxType::MintFungible,
            base_tx(
                TxType::MintFungible,
                from,
                TxPayload::MintFungible(TxMsgMintFungible {
                    token_id: 9,
                    to,
                    amount: IntX::from(1i64),
                }),
            ),
            Some(vec![NativeFeeKind::MintFungible]),
        ),
        (
            TxType::BurnFungible,
            base_tx(
                TxType::BurnFungible,
                from,
                TxPayload::BurnFungible(TxMsgBurnFungible {
                    token_id: 9,
                    amount: IntX::from(1i64),
                }),
            ),
            Some(vec![NativeFeeKind::BurnFungible]),
        ),
        (
            TxType::BurnFungibleGasPayer,
            base_tx(
                TxType::BurnFungibleGasPayer,
                from,
                TxPayload::BurnFungibleGasPayer(TxMsgBurnFungibleGasPayer {
                    token_id: 9,
                    from_address: to,
                    amount: IntX::from(1i64),
                }),
            ),
            Some(vec![NativeFeeKind::BurnFungible]),
        ),
        (
            TxType::MintNonFungible,
            base_tx(
                TxType::MintNonFungible,
                from,
                TxPayload::MintNonFungible(TxMsgMintNonFungible {
                    token_id: 9,
                    to,
                    series_id: 1,
                    rom: vec![0u8; 8],
                    ram: Vec::new(),
                }),
            ),
            Some(vec![NativeFeeKind::MintNonFungible]),
        ),
        (
            TxType::BurnNonFungible,
            base_tx(
                TxType::BurnNonFungible,
                from,
                TxPayload::BurnNonFungible(TxMsgBurnNonFungible {
                    token_id: 9,
                    instance_id: 1,
                }),
            ),
            Some(vec![NativeFeeKind::BurnNonFungible]),
        ),
        (
            TxType::BurnNonFungibleGasPayer,
            base_tx(
                TxType::BurnNonFungibleGasPayer,
                from,
                TxPayload::BurnNonFungibleGasPayer(TxMsgBurnNonFungibleGasPayer {
                    token_id: 9,
                    from_address: to,
                    instance_id: 1,
                }),
            ),
            Some(vec![NativeFeeKind::BurnNonFungible]),
        ),
        (
            TxType::Phantasma,
            base_tx(
                TxType::Phantasma,
                from,
                TxPayload::Phantasma(TxMsgPhantasma {
                    nexus: SmallString::new("main").unwrap(),
                    chain: SmallString::new("main").unwrap(),
                    script: vec![1, 2, 3],
                }),
            ),
            Some(vec![NativeFeeKind::Script]),
        ),
        // A raw Phantasma transaction carries a foreign envelope the planner cannot size or read, so
        // it is refused instead of budgeted.
        (
            TxType::PhantasmaRaw,
            base_tx(
                TxType::PhantasmaRaw,
                from,
                TxPayload::PhantasmaRaw(TxMsgPhantasmaRaw {
                    transaction: vec![1, 2, 3],
                }),
            ),
            None,
        ),
    ]
}

/// Every TxType the SDK carries and what the planner makes of it. A new transaction type fails this
/// test until someone states its fee.
#[test]
fn plans_every_transaction_type_as_declared() {
    let config = v2_config();
    let mut seen = HashSet::new();
    for (tx_type, msg, expected) in type_cases() {
        assert!(
            seen.insert(tx_type as u8),
            "{tx_type:?} is in the table twice"
        );
        // The row must plan the type it declares, or the exhaustiveness check below means nothing.
        assert_eq!(msg.tx_type, tx_type, "{tx_type:?} row carries another type");
        // Only the witness-array types take a count from the caller. Every other type fixes its own
        // witness set in the message, and a count that disagrees with it is refused.
        let options = FeePlanOptions {
            witness_count: required_witnesses(&msg).is_none().then_some(1),
            infusions: Some(Vec::new()),
            ..FeePlanOptions::default()
        };
        match expected {
            None => assert!(
                plan_fees(&msg, &config, &options).is_err(),
                "{tx_type:?} must be refused"
            ),
            Some(kinds) => {
                let plan = plan_fees(&msg, &config, &options)
                    .unwrap_or_else(|err| panic!("{tx_type:?}: {err}"));
                assert_eq!(plan.kinds, kinds, "{tx_type:?}");
            }
        }
    }
    for value in 0u8..=16 {
        assert!(
            seen.contains(&value),
            "transaction type {value} has no row: state what the planner makes of it"
        );
    }
}

fn token_method_cases(owner: Bytes32, to: Bytes32) -> Vec<(u32, Vec<u8>, NativeFeeKind)> {
    let metadata = build_token_metadata(&[
        ("name", "Plan probe"),
        ("icon", ICON),
        ("url", "https://example.invalid/p"),
        ("description", "x"),
    ])
    .unwrap();
    let info = build_token_info(
        "GPX",
        IntX::from(0i64),
        false,
        2,
        owner,
        metadata,
        Vec::new(),
    )
    .unwrap();
    let mut cases = vec![
        (
            TokenContractMethod::TransferFungible as u32,
            serialize(&TransferFungibleArgs {
                to,
                from_address: owner,
                token_id: 9,
                amount: IntX::from(1i64),
            })
            .unwrap(),
            NativeFeeKind::TransferFungible,
        ),
        (
            TokenContractMethod::TransferNonFungible as u32,
            serialize(&TransferNonFungibleArgs {
                to,
                from_address: owner,
                token_id: 9,
                instance_ids: vec![1],
            })
            .unwrap(),
            NativeFeeKind::TransferNonFungible,
        ),
        (
            TokenContractMethod::CreateToken as u32,
            serialize(&info).unwrap(),
            NativeFeeKind::CreateToken,
        ),
        (
            TokenContractMethod::MintFungible as u32,
            serialize(&MintFungibleArgs {
                token_id: 9,
                to,
                amount: IntX::from(1i64),
            })
            .unwrap(),
            NativeFeeKind::MintFungible,
        ),
        (
            TokenContractMethod::BurnFungible as u32,
            serialize(&BurnFungibleArgs {
                token_id: 9,
                from_address: owner,
                amount: IntX::from(1i64),
            })
            .unwrap(),
            NativeFeeKind::BurnFungible,
        ),
        (
            TokenContractMethod::CreateTokenSeries as u32,
            {
                let mut args = 9u64.to_le_bytes().to_vec();
                args.extend(serialize(&SeriesInfo::default()).unwrap());
                args
            },
            NativeFeeKind::CreateTokenSeries,
        ),
        (
            TokenContractMethod::BurnNonFungible as u32,
            serialize(&BurnNonFungibleArgs {
                token_id: 9,
                from_address: owner,
                instance_ids: vec![1],
            })
            .unwrap(),
            NativeFeeKind::BurnNonFungible,
        ),
        (
            TokenContractMethod::MintPhantasmaNonFungible as u32,
            serialize(&MintPhantasmaNonFungibleArgs {
                token_id: 9,
                address: to,
                tokens: vec![PhantasmaNFTMintInfo {
                    phantasma_series_id: IntX::from(1i64),
                    rom: vec![0u8; 8],
                    ram: Vec::new(),
                }],
            })
            .unwrap(),
            NativeFeeKind::MintPhantasmaNonFungible,
        ),
    ];
    // Every other method of the module is budgeted. MintNonFungible is among them on purpose: the
    // chain refuses an explicit NFT mint where governance has not allowed caller-supplied ROM ids,
    // whichever way it arrives, so there is nothing to price.
    let modelled: HashSet<u32> = cases.iter().map(|(method, _, _)| *method).collect();
    for method in 0u32..=(TokenContractMethod::MintPhantasmaNonFungible as u32) {
        if !modelled.contains(&method) {
            cases.push((method, PROBE_ARGS.to_vec(), NativeFeeKind::Script));
        }
    }
    cases
}

/// Every method of the token module and what the planner makes of a call to it. A method that grows
/// a model, or loses one, fails here.
#[test]
fn plans_every_token_contract_method_as_declared() {
    let config = v2_config();
    let (owner, other) = keys();
    let from = address_of(&owner);
    let to = address_of(&other);
    let mut seen = HashSet::new();
    for (method, args, kind) in token_method_cases(from, to) {
        assert!(
            seen.insert(method),
            "token method {method} is in the table twice"
        );
        let msg = base_tx(
            TxType::Call,
            from,
            TxPayload::Call(TxMsgCall {
                module_id: ModuleId::Token as u32,
                method_id: method,
                args,
                sections: None,
            }),
        );
        let options = FeePlanOptions {
            witness_count: Some(1),
            infusions: Some(Vec::new()),
            ..FeePlanOptions::default()
        };
        let plan = plan_fees(&msg, &config, &options)
            .unwrap_or_else(|err| panic!("method {method}: {err}"));
        assert_eq!(plan.kinds, vec![kind], "token method {method}");
    }
    for method in 0u32..=(TokenContractMethod::MintPhantasmaNonFungible as u32) {
        assert!(
            seen.contains(&method),
            "token method {method} has no row: state what the planner makes of it"
        );
    }
}

/// The same for the governance module, plus a module the planner does not dispatch on at all.
#[test]
fn plans_every_governance_contract_method_as_declared() {
    let config = v2_config();
    let (owner, _) = keys();
    let from = address_of(&owner);
    let options = FeePlanOptions {
        witness_count: Some(1),
        ..FeePlanOptions::default()
    };
    let cases = vec![
        (
            GovernanceContractMethod::RegisterName as u32,
            serialize(&RegisterNameArgs {
                address: from,
                name: SmallString::new("probe").unwrap(),
            })
            .unwrap(),
            NativeFeeKind::RegisterName,
        ),
        (
            GovernanceContractMethod::SetGasConfig as u32,
            PROBE_ARGS.to_vec(),
            NativeFeeKind::Script,
        ),
    ];
    let mut seen = HashSet::new();
    for (method, args, kind) in cases {
        seen.insert(method);
        let msg = base_tx(
            TxType::Call,
            from,
            TxPayload::Call(TxMsgCall {
                module_id: ModuleId::Governance as u32,
                method_id: method,
                args,
                sections: None,
            }),
        );
        let plan = plan_fees(&msg, &config, &options).unwrap();
        assert_eq!(plan.kinds, vec![kind], "governance method {method}");
    }
    for method in [
        GovernanceContractMethod::RegisterName as u32,
        GovernanceContractMethod::SetGasConfig as u32,
    ] {
        assert!(
            seen.contains(&method),
            "governance method {method} has no row"
        );
    }
    // A module the planner does not dispatch on at all takes the script budget.
    let market = base_tx(
        TxType::Call,
        from,
        TxPayload::Call(TxMsgCall {
            module_id: ModuleId::Market as u32,
            method_id: 1,
            args: PROBE_ARGS.to_vec(),
            sections: None,
        }),
    );
    assert_eq!(
        plan_fees(&market, &config, &options).unwrap().kinds,
        vec![NativeFeeKind::Script]
    );
}
