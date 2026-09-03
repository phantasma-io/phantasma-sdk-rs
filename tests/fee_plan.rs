//! Planner tests: `plan_fees` reads every message fact out of the message, demands the facts it
//! cannot bound, and prices with the calculator the estimator tests pin to live bills.

use phantasma_sdk::{
    build_and_serialize_token_schemas, build_create_token_series_tx, build_create_token_tx,
    build_mint_phantasma_non_fungible_tx, build_series_info, build_token_info,
    build_token_metadata, bytes32_from_public_key, deserialize, envelope_bytes,
    estimate_native_fee, get_nft_address, is_nft_address, plan_fees, serialize, unpack_nft_address,
    Bytes32, FeePlanOptions, GasConfig, GovernanceContractMethod, InfusedAsset, IntX, ModuleId,
    NativeFeeKind, NativeFeeParams, PhantasmaKeys, PhantasmaNFTMintInfo, RegisterNameArgs,
    SmallString, TxLimits, TxMsg, TxMsgBurnNonFungible, TxMsgCall, TxMsgCallMulti,
    TxMsgMintNonFungible, TxMsgTransferFungible, TxMsgTransferFungibleGasPayer,
    TxMsgTransferNonFungibleMulti, TxPayload, TxType,
};

const ICON: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==";

fn config() -> GasConfig {
    GasConfig {
        version: 1,
        max_name_length: 255,
        max_token_symbol_length: 255,
        fee_multiplier: 10_000,
        gas_token_id: 1,
        data_token_id: 2,
        minimum_gas_offer: 10,
        data_escrow_per_row: 200_000,
        legacy_data_escrow_per_row: 2,
        minimum_gas_bill: 10_000_000,
        gas_fee_transfer: 10,
        gas_fee_query: 10,
        gas_fee_create_token_base: 10_000_000_000,
        gas_fee_create_token_symbol: 10_000_000_000,
        gas_fee_create_token_series: 2_500_000_000,
        gas_fee_per_byte: 250_000,
        gas_fee_register_name: 10_000_000_000_000,
        gas_burn_ratio_mul: 1,
        policy_fee_create_token_base: 100_000_000_000_000,
        policy_fee_create_token_symbol: 100_000_000_000_000,
        policy_fee_create_token_series: 25_000_000_000_000,
        policy_fee_register_name: 100_000_000_000_000_000,
        ..GasConfig::default()
    }
}

fn keys() -> (PhantasmaKeys, PhantasmaKeys) {
    (
        PhantasmaKeys::from_wif("KwPpBSByydVKqStGHAnZzQofCqhDmD2bfRgc9BmZqM3ZmsdWJw4d").unwrap(),
        PhantasmaKeys::from_wif("KwVG94yjfVg1YKFyRxAGtug93wdRbmLnqqrFV6Yd2CiA9KZDAp4H").unwrap(),
    )
}

fn address_of(keys: &PhantasmaKeys) -> Bytes32 {
    bytes32_from_public_key(&keys.public_key()).unwrap()
}

fn base_tx(tx_type: TxType, gas_from: Bytes32, msg: TxPayload) -> TxMsg {
    TxMsg {
        tx_type,
        expiry: 1_759_711_416_000,
        max_gas: 0,
        max_data: 0,
        gas_from,
        payload: SmallString::default(),
        msg,
    }
}

fn fixed_expiry() -> TxLimits {
    TxLimits {
        expiry: 1_759_711_416_000,
        ..TxLimits::default()
    }
}

fn transfer(to: Bytes32, token_id: u64) -> TxMsg {
    let (payer, _) = keys();
    base_tx(
        TxType::TransferFungible,
        address_of(&payer),
        TxPayload::TransferFungible(TxMsgTransferFungible {
            to,
            token_id,
            amount: 100_000_000,
        }),
    )
}

fn create_token(symbol: &str, is_nft: bool, extra: &[(&str, &str)]) -> TxMsg {
    let (creator, _) = keys();
    let mut fields = vec![
        ("name", "Planned"),
        ("icon", ICON),
        ("url", "https://example.com"),
        ("description", "a planned token"),
    ];
    fields.extend_from_slice(extra);
    let metadata = build_token_metadata(&fields).unwrap();
    let info = build_token_info(
        symbol,
        IntX::from(0i64),
        is_nft,
        8,
        address_of(&creator),
        metadata,
        if is_nft {
            build_and_serialize_token_schemas(None).unwrap()
        } else {
            Vec::new()
        },
    )
    .unwrap();
    build_create_token_tx(info, address_of(&creator), fixed_expiry()).unwrap()
}

fn phantasma_mint(series_ids: &[i64], rom_bytes: usize, to: Bytes32) -> TxMsg {
    let (owner, _) = keys();
    let tokens = series_ids
        .iter()
        .map(|series| PhantasmaNFTMintInfo {
            phantasma_series_id: IntX::from(*series),
            rom: vec![7u8; rom_bytes],
            ram: Vec::new(),
        })
        .collect();
    build_mint_phantasma_non_fungible_tx(9, address_of(&owner), to, tokens, fixed_expiry()).unwrap()
}

// A native transfer is planned from the message alone: its signed size with one signature, the
// recipient read from the message, the bill the calculator gives for those facts.
#[test]
fn plans_a_native_transfer_from_the_message() {
    let (_, receiver) = keys();
    let msg = transfer(address_of(&receiver), 1);
    let plan = plan_fees(&msg, &config(), &FeePlanOptions::default()).unwrap();
    assert_eq!(plan.kind, NativeFeeKind::TransferFungible);
    assert_eq!(plan.envelope_bytes, envelope_bytes(&msg, None).unwrap());
    assert_eq!(plan.envelope_bytes, 170);
    assert_eq!(plan.expected_gas_bill, 42_600_000);
    assert_eq!(plan.max_gas, 42_600_000);
    assert_eq!(plan.max_data, 0);

    let applied = plan.apply(&msg);
    assert_eq!(applied.max_gas, plan.max_gas);
    assert_eq!(applied.max_data, plan.max_data);
    assert_eq!(msg.max_gas, 0, "apply leaves the input untouched");
    assert_eq!(plan.quote().expected_gas_bill, plan.expected_gas_bill);
}

// The plan equals the calculator's answer for the facts the message states, so a change in either
// shows up here as a disagreement rather than as a silently different number.
#[test]
fn agrees_with_the_calculator_for_the_facts_it_reads() {
    let (_, receiver) = keys();
    let msg = transfer(address_of(&receiver), 97);
    let options = FeePlanOptions {
        recipient_holds_token: true,
        ..FeePlanOptions::default()
    };
    let plan = plan_fees(&msg, &config(), &options).unwrap();
    let direct = estimate_native_fee(
        NativeFeeKind::TransferFungible,
        &config(),
        &NativeFeeParams {
            envelope_bytes: 170,
            token_id: Some(97),
            recipient_holds_token: true,
            ..NativeFeeParams::default()
        },
    )
    .unwrap();
    assert_eq!(plan.quote(), direct.quote());
    assert_eq!(plan.new_storage_quanta, direct.new_storage_quanta);
}

// A gas-payer transfer fixes its own two witnesses: the plan sizes both signatures without being
// told, and a stated count that disagrees is refused rather than priced.
#[test]
fn sizes_a_gas_payer_transfer_for_both_of_its_signatures() {
    let (payer, owner) = keys();
    let msg = base_tx(
        TxType::TransferFungibleGasPayer,
        address_of(&payer),
        TxPayload::TransferFungibleGasPayer(TxMsgTransferFungibleGasPayer {
            to: address_of(&owner),
            from_address: address_of(&owner),
            token_id: 1,
            amount: 5,
        }),
    );
    let plan = plan_fees(&msg, &config(), &FeePlanOptions::default()).unwrap();
    assert_eq!(plan.envelope_bytes, 170 + 32 + 64);
    let agreeing = plan_fees(
        &msg,
        &config(),
        &FeePlanOptions {
            witness_count: Some(2),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(agreeing, plan);
    let err = plan_fees(
        &msg,
        &config(),
        &FeePlanOptions {
            witness_count: Some(1),
            ..FeePlanOptions::default()
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("witness"), "{err}");
}

// A Call chooses its own witnesses, so the count is demanded; each witness is 96 billed bytes.
#[test]
fn demands_the_witness_count_of_a_call_and_bills_each_witness() {
    let msg = create_token("PLAN", false, &[]);
    let err = plan_fees(&msg, &config(), &FeePlanOptions::default()).unwrap_err();
    assert!(err.to_string().contains("witness_count"), "{err}");
    let one = plan_fees(
        &msg,
        &config(),
        &FeePlanOptions {
            witness_count: Some(1),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    let two = plan_fees(
        &msg,
        &config(),
        &FeePlanOptions {
            witness_count: Some(2),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(two.envelope_bytes - one.envelope_bytes, 96);
    assert_eq!(
        two.expected_gas_bill - one.expected_gas_bill,
        96 * 25 * 10_000
    );
}

// CreateToken: the symbol length, the NFT flag, the token-info row (the Call arguments as
// submitted) and the metadata keys that add rows or lookups are all read from the call.
#[test]
fn reads_a_token_creation_out_of_its_call() {
    let options = FeePlanOptions {
        witness_count: Some(1),
        ..FeePlanOptions::default()
    };
    let fungible = create_token("PLANNED", false, &[]);
    let TxPayload::Call(call) = &fungible.msg else {
        panic!("expected a call");
    };
    let plan = plan_fees(&fungible, &config(), &options).unwrap();
    assert_eq!(plan.kind, NativeFeeKind::CreateToken);
    let direct = estimate_native_fee(
        NativeFeeKind::CreateToken,
        &config(),
        &NativeFeeParams {
            envelope_bytes: plan.envelope_bytes,
            symbol_length: 7,
            token_info_bytes: call.args.len() as u32,
            ..NativeFeeParams::default()
        },
    )
    .unwrap();
    assert_eq!(plan.quote(), direct.quote());
    assert_eq!(plan.new_storage_quanta, 3);

    let nft = plan_fees(&create_token("PLANNED", true, &[]), &config(), &options).unwrap();
    assert_eq!(
        nft.new_storage_quanta, 4,
        "an NFT token adds its series counter row"
    );

    // The staking keys cost a lookup each; the pre-burn and inflation keys write a row each.
    let plain = plan_fees(&create_token("PLANNED", false, &[]), &config(), &options).unwrap();
    let org = plan_fees(
        &create_token("PLANNED", false, &[("_soi", "1")]),
        &config(),
        &options,
    )
    .unwrap();
    let reward = plan_fees(
        &create_token("PLANNED", false, &[("_srt", "1")]),
        &config(),
        &options,
    )
    .unwrap();
    // The metadata grows by the key it carries, which the token-info row and the envelope both
    // carry; take that growth out to see the lookup alone.
    let block_data = |plan: &phantasma_sdk::FeePlan| {
        u64::from(plan.envelope_bytes + plan.new_storage_quanta) * 25 * 10_000
    };
    assert_eq!(
        (org.expected_gas_bill - block_data(&org)) - (plain.expected_gas_bill - block_data(&plain)),
        100_000
    );
    assert_eq!(
        (reward.expected_gas_bill - block_data(&reward))
            - (plain.expected_gas_bill - block_data(&plain)),
        100_000
    );
    let pre_burn = plan_fees(
        &create_token("PLANNED", false, &[("_brn", "1")]),
        &config(),
        &options,
    )
    .unwrap();
    assert_eq!(pre_burn.new_storage_quanta, plain.new_storage_quanta + 1);
    let inflation = plan_fees(
        &create_token("PLANNED", false, &[("_ip", "1")]),
        &config(),
        &options,
    )
    .unwrap();
    assert_eq!(inflation.new_storage_quanta, plain.new_storage_quanta + 1);
}

// CreateTokenSeries: the row is the SeriesInfo after the 8-byte token id; the meta-id flag is chain
// state passed through.
#[test]
fn reads_a_series_creation_out_of_its_call() {
    let (creator, _) = keys();
    let info = build_series_info(5, 0, 0, address_of(&creator)).unwrap();
    let msg = build_create_token_series_tx(9, info, address_of(&creator), fixed_expiry()).unwrap();
    let TxPayload::Call(call) = &msg.msg else {
        panic!("expected a call");
    };
    let options = FeePlanOptions {
        witness_count: Some(1),
        series_has_meta_id: Some(true),
        ..FeePlanOptions::default()
    };
    let plan = plan_fees(&msg, &config(), &options).unwrap();
    assert_eq!(plan.kind, NativeFeeKind::CreateTokenSeries);
    let direct = estimate_native_fee(
        NativeFeeKind::CreateTokenSeries,
        &config(),
        &NativeFeeParams {
            envelope_bytes: plan.envelope_bytes,
            series_info_bytes: call.args.len() as u32 - 8,
            series_has_meta_id: Some(true),
            ..NativeFeeParams::default()
        },
    )
    .unwrap();
    assert_eq!(plan.quote(), direct.quote());
    assert_eq!(plan.new_storage_quanta, 3);
}

// A Phantasma mint: the count, every instance's ROM size, the distinct series the instances name
// and the NFT-address recipient are read from the call; the series mode is passed through.
#[test]
fn reads_a_phantasma_mint_out_of_its_call() {
    let (_, receiver) = keys();
    let options = FeePlanOptions {
        witness_count: Some(1),
        duplicated_series: Some(true),
        supply_row_exists: true,
        ..FeePlanOptions::default()
    };
    let one_series = plan_fees(
        &phantasma_mint(&[5, 5, 5], 75, address_of(&receiver)),
        &config(),
        &options,
    )
    .unwrap();
    assert_eq!(one_series.kind, NativeFeeKind::MintPhantasmaNonFungible);
    let three_series = plan_fees(
        &phantasma_mint(&[5, 6, 7], 75, address_of(&receiver)),
        &config(),
        &options,
    )
    .unwrap();
    assert_eq!(three_series.envelope_bytes, one_series.envelope_bytes);
    assert_eq!(
        three_series.expected_gas_bill - one_series.expected_gas_bill,
        2 * 10 * 10_000,
        "one supply read per distinct series"
    );
    let direct = estimate_native_fee(
        NativeFeeKind::MintPhantasmaNonFungible,
        &config(),
        &NativeFeeParams {
            envelope_bytes: one_series.envelope_bytes,
            token_id: Some(9),
            count: Some(3),
            rom_bytes: vec![75, 75, 75],
            ram_bytes: vec![0, 0, 0],
            duplicated_series: Some(true),
            distinct_series_count: Some(1),
            supply_row_exists: true,
            ..NativeFeeParams::default()
        },
    )
    .unwrap();
    assert_eq!(one_series.quote(), direct.quote());

    let infused = plan_fees(
        &phantasma_mint(&[5, 5, 5], 75, get_nft_address(9, 1)),
        &config(),
        &options,
    )
    .unwrap();
    assert_eq!(
        infused.expected_gas_bill - one_series.expected_gas_bill,
        100_000,
        "an NFT-address recipient costs one owner lookup"
    );
}

// A native NFT mint carries its ROM and RAM; both are rows, both are read from the message.
#[test]
fn reads_a_native_nft_mint_out_of_the_message() {
    let (owner, receiver) = keys();
    let msg = base_tx(
        TxType::MintNonFungible,
        address_of(&owner),
        TxPayload::MintNonFungible(TxMsgMintNonFungible {
            token_id: 7,
            to: address_of(&receiver),
            series_id: 1,
            rom: vec![1u8; 1100],
            ram: vec![2u8; 30],
        }),
    );
    let plan = plan_fees(&msg, &config(), &FeePlanOptions::default()).unwrap();
    assert_eq!(plan.kind, NativeFeeKind::MintNonFungible);
    let direct = estimate_native_fee(
        NativeFeeKind::MintNonFungible,
        &config(),
        &NativeFeeParams {
            envelope_bytes: plan.envelope_bytes,
            token_id: Some(7),
            rom_bytes: vec![1100],
            ram_bytes: vec![30],
            ..NativeFeeParams::default()
        },
    )
    .unwrap();
    assert_eq!(plan.quote(), direct.quote());
    assert_eq!(plan.new_storage_quanta, direct.new_storage_quanta);
}

// A multi-instance transfer counts its instances from the message; an empty list is refused.
#[test]
fn counts_the_instances_of_a_multi_transfer() {
    let (owner, receiver) = keys();
    let multi = |instance_ids: Vec<u64>| {
        base_tx(
            TxType::TransferNonFungibleMulti,
            address_of(&owner),
            TxPayload::TransferNonFungibleMulti(TxMsgTransferNonFungibleMulti {
                to: address_of(&receiver),
                token_id: 7,
                instance_ids,
            }),
        )
    };
    let plan = plan_fees(&multi(vec![1, 2]), &config(), &FeePlanOptions::default()).unwrap();
    assert_eq!(plan.kind, NativeFeeKind::TransferNonFungible);
    assert_eq!(plan.deleted_storage_quanta, 2);
    assert_eq!(plan.new_storage_quanta, 3);
    let err = plan_fees(&multi(vec![]), &config(), &FeePlanOptions::default()).unwrap_err();
    assert!(err.to_string().contains("at least one instance"), "{err}");
}

// A burn returns whatever the NFT holds, which no default can bound: the plan demands the list, an
// empty list states that the address holds nothing, and each asset is priced as its return.
#[test]
fn demands_what_a_burned_nft_holds() {
    let (owner, _) = keys();
    let msg = base_tx(
        TxType::BurnNonFungible,
        address_of(&owner),
        TxPayload::BurnNonFungible(TxMsgBurnNonFungible {
            token_id: 7,
            instance_id: 42,
        }),
    );
    let err = plan_fees(&msg, &config(), &FeePlanOptions::default()).unwrap_err();
    assert!(err.to_string().contains("infusions"), "{err}");
    let empty = plan_fees(
        &msg,
        &config(),
        &FeePlanOptions {
            infusions: Some(vec![]),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(empty.kind, NativeFeeKind::BurnNonFungible);
    let infused = plan_fees(
        &msg,
        &config(),
        &FeePlanOptions {
            infusions: Some(vec![InfusedAsset {
                token_id: Some(1),
                ..InfusedAsset::default()
            }]),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        infused.expected_gas_bill - empty.expected_gas_bill,
        20 * 10_000
    );
}

// Transfers into an NFT-derived address pay the owner lookup of the target; the address form is
// read from the message, never stated by the caller.
#[test]
fn prices_the_owner_lookup_of_an_nft_address_recipient() {
    let (_, receiver) = keys();
    let plain = plan_fees(
        &transfer(address_of(&receiver), 1),
        &config(),
        &FeePlanOptions::default(),
    )
    .unwrap();
    let infused = plan_fees(
        &transfer(get_nft_address(9, 1), 1),
        &config(),
        &FeePlanOptions::default(),
    )
    .unwrap();
    assert_eq!(infused.expected_gas_bill - plain.expected_gas_bill, 100_000);
}

// Calls the model does not price are budgeted as scripts; a raw Phantasma transaction cannot be
// planned at all.
#[test]
fn budgets_unmodelled_calls_as_scripts_and_refuses_raw_transactions() {
    let (owner, _) = keys();
    let unknown = base_tx(
        TxType::Call,
        address_of(&owner),
        TxPayload::Call(TxMsgCall {
            module_id: ModuleId::Token as u32,
            method_id: 999,
            args: vec![0u8; 40],
            sections: None,
        }),
    );
    let options = FeePlanOptions {
        witness_count: Some(1),
        script_storage_quanta: Some(0),
        ..FeePlanOptions::default()
    };
    let plan = plan_fees(&unknown, &config(), &options).unwrap();
    assert_eq!(plan.kind, NativeFeeKind::Script);
    assert_eq!(
        plan.expected_gas_bill,
        (5000 + u64::from(plan.envelope_bytes + 512) * 25) * 10_000
    );
    let multi = base_tx(
        TxType::CallMulti,
        address_of(&owner),
        TxPayload::CallMulti(TxMsgCallMulti { calls: vec![] }),
    );
    assert_eq!(
        plan_fees(&multi, &config(), &options).unwrap().kind,
        NativeFeeKind::Script
    );
}

// RegisterName is a governance call whose arguments the plan reads for the name length.
#[test]
fn reads_a_name_registration_out_of_its_call() {
    let (owner, _) = keys();
    let args = RegisterNameArgs {
        address: address_of(&owner),
        name: SmallString::new("planned-name").unwrap(),
    };
    let bytes = serialize(&args).unwrap();
    assert_eq!(deserialize::<RegisterNameArgs>(&bytes).unwrap(), args);
    let msg = base_tx(
        TxType::Call,
        address_of(&owner),
        TxPayload::Call(TxMsgCall {
            module_id: ModuleId::Governance as u32,
            method_id: GovernanceContractMethod::RegisterName as u32,
            args: bytes,
            sections: None,
        }),
    );
    let plan = plan_fees(
        &msg,
        &config(),
        &FeePlanOptions {
            witness_count: Some(2),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(plan.kind, NativeFeeKind::RegisterName);
    let direct = estimate_native_fee(
        NativeFeeKind::RegisterName,
        &config(),
        &NativeFeeParams {
            envelope_bytes: plan.envelope_bytes,
            name_length: 12,
            ..NativeFeeParams::default()
        },
    )
    .unwrap();
    assert_eq!(plan.quote(), direct.quote());
    assert_eq!(plan.max_data, 0);
}

#[test]
fn recognises_nft_addresses_by_their_form() {
    let address = get_nft_address(9, 42);
    assert!(is_nft_address(&address));
    assert_eq!(unpack_nft_address(&address), (9, 42));
    assert!(!is_nft_address(&get_nft_address(0, 42)));
    assert!(!is_nft_address(&get_nft_address(9, 0)));
    let (owner, _) = keys();
    assert!(!is_nft_address(&address_of(&owner)));
    assert!(!is_nft_address(&Bytes32::default()));
}
