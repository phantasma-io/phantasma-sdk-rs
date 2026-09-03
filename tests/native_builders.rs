//! Native builders assemble the message only: the fee is planned afterwards from the message, the
//! form (plain or gas-payer) follows from whether a gas payer was named, and the limits are the
//! caller's. The `build_*_tx_and_sign` conveniences plan a fresh message before signing it.

use std::time::Duration;

use phantasma_sdk::{
    build_burn_fungible_tx, build_burn_non_fungible_tx, build_create_token_series_tx,
    build_create_token_series_tx_and_sign, build_create_token_tx, build_create_token_tx_and_sign,
    build_create_token_tx_and_sign_hex, build_mint_fungible_tx,
    build_mint_phantasma_non_fungible_single_tx,
    build_mint_phantasma_non_fungible_single_tx_and_sign, build_mint_phantasma_non_fungible_tx,
    build_mint_phantasma_non_fungible_tx_and_sign, build_phantasma_nft_rom, build_series_info,
    build_token_info, build_token_metadata, build_transfer_fungible_tx,
    build_transfer_non_fungible_tx, bytes32_from_public_key, default_expiry, deserialize,
    expiry_within, now_unix_millis, plan_and_sign_with_keys, plan_fees,
    prepare_standard_token_schemas, sign_tx_msg, sign_tx_msg_with_keys, BurnFungibleParams,
    BurnNonFungibleParams, Bytes32, FeePlanOptions, GasConfig, IntX, MintFungibleParams,
    PhantasmaKeys, PhantasmaNFTMintInfo, PlanAndSignOptions, SignedTxMsg, TransferFungibleParams,
    TransferNonFungibleParams, TxLimits, TxPayload, TxType, VMValue, DEFAULT_TX_EXPIRY,
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

fn repeated(byte: u8) -> Bytes32 {
    Bytes32([byte; 32])
}

fn token_info(owner: Bytes32) -> phantasma_sdk::TokenInfo {
    let metadata = build_token_metadata(&[
        ("name", "Planned"),
        ("icon", ICON),
        ("url", "https://example.com"),
        ("description", "a planned token"),
    ])
    .unwrap();
    build_token_info(
        "PLANNED",
        IntX::from(0i64),
        false,
        8,
        owner,
        metadata,
        Vec::new(),
    )
    .unwrap()
}

fn one_witness() -> FeePlanOptions {
    FeePlanOptions {
        witness_count: Some(1),
        ..FeePlanOptions::default()
    }
}

#[test]
fn builds_a_plain_transfer_paid_by_the_sender_unplanned_until_fees_are_set() {
    let (owner_keys, receiver_keys) = keys();
    let (owner, receiver) = (address_of(&owner_keys), address_of(&receiver_keys));
    let msg = build_transfer_fungible_tx(TransferFungibleParams {
        from: owner,
        to: receiver,
        token_id: 1,
        amount: 5,
        ..TransferFungibleParams::default()
    });
    assert_eq!(msg.tx_type, TxType::TransferFungible);
    assert_eq!(msg.gas_from, owner);
    assert_eq!((msg.max_gas, msg.max_data), (0, 0));
    assert!(
        msg.expiry > now_unix_millis(),
        "the default expiry is in the future"
    );
    let TxPayload::TransferFungible(inner) = &msg.msg else {
        panic!("payload = {:?}", msg.msg);
    };
    assert_eq!((inner.to, inner.amount), (receiver, 5));
    // A zero offer can never be admitted, so signing an unplanned message is refused.
    let err = sign_tx_msg(&msg, &owner_keys).unwrap_err();
    assert!(err.to_string().contains("no gas offer"), "{err}");
    let planned = plan_fees(&msg, &config(), &FeePlanOptions::default())
        .unwrap()
        .apply(&msg);
    assert_eq!(planned.max_gas, 42_600_000);
    let signed = sign_tx_msg(&planned, &owner_keys).unwrap();
    assert_eq!(signed.witnesses.len(), 1);
}

#[test]
fn switches_to_the_gas_payer_form_when_another_account_pays() {
    let (owner_keys, payer_keys) = keys();
    let (owner, payer) = (address_of(&owner_keys), address_of(&payer_keys));
    let msg = build_transfer_fungible_tx(TransferFungibleParams {
        limits: TxLimits {
            max_gas: 60_000_000,
            ..TxLimits::default()
        },
        from: owner,
        gas_payer: Some(payer),
        to: repeated(0x33),
        token_id: 1,
        amount: 5,
    });
    assert_eq!(msg.tx_type, TxType::TransferFungibleGasPayer);
    assert_eq!(msg.gas_from, payer);
    let TxPayload::TransferFungibleGasPayer(inner) = &msg.msg else {
        panic!("payload = {:?}", msg.msg);
    };
    assert_eq!(inner.from_address, owner);
    // Both sign: the payer first, the owner second, as the node reads them.
    let signed = sign_tx_msg_with_keys(&msg, &[&owner_keys, &payer_keys]).unwrap();
    assert_eq!(signed.witnesses.len(), 2);
    assert_eq!(signed.witnesses[0].address, payer);
    assert_eq!(signed.witnesses[1].address, owner);
}

#[test]
fn picks_the_single_or_multi_instance_type_by_the_instance_count() {
    let (owner, payer, receiver) = (repeated(0x11), repeated(0x22), repeated(0x33));
    let build = |gas_payer: Option<Bytes32>, instance_ids: Vec<u64>| {
        build_transfer_non_fungible_tx(TransferNonFungibleParams {
            from: owner,
            gas_payer,
            to: receiver,
            token_id: 7,
            instance_ids,
            ..TransferNonFungibleParams::default()
        })
    };
    let single = build(None, vec![42]).unwrap();
    assert_eq!(single.tx_type, TxType::TransferNonFungibleSingle);
    let TxPayload::TransferNonFungibleSingle(inner) = &single.msg else {
        panic!("payload = {:?}", single.msg);
    };
    assert_eq!(inner.instance_id, 42);
    let multi = build(None, vec![42, 43]).unwrap();
    assert_eq!(multi.tx_type, TxType::TransferNonFungibleMulti);
    let TxPayload::TransferNonFungibleMulti(inner) = &multi.msg else {
        panic!("payload = {:?}", multi.msg);
    };
    assert_eq!(inner.instance_ids, vec![42, 43]);
    let paid_single = build(Some(payer), vec![42]).unwrap();
    assert_eq!(
        paid_single.tx_type,
        TxType::TransferNonFungibleSingleGasPayer
    );
    assert_eq!(paid_single.gas_from, payer);
    let paid_multi = build(Some(payer), vec![42, 43]).unwrap();
    assert_eq!(paid_multi.tx_type, TxType::TransferNonFungibleMultiGasPayer);
    let TxPayload::TransferNonFungibleMultiGasPayer(inner) = &paid_multi.msg else {
        panic!("payload = {:?}", paid_multi.msg);
    };
    assert_eq!(inner.from_address, owner);
    let err = build(None, vec![]).unwrap_err();
    assert!(err.to_string().contains("instance_ids"), "{err}");
}

#[test]
fn builds_mints_and_burns_with_the_limits_given() {
    let (owner, payer, receiver) = (repeated(0x11), repeated(0x22), repeated(0x33));
    let limits = TxLimits {
        max_gas: 1,
        max_data: 2,
        expiry: 3,
    };
    let mint = build_mint_fungible_tx(MintFungibleParams {
        limits,
        owner,
        to: receiver,
        token_id: 9,
        amount: IntX::from(100i64),
    });
    assert_eq!(mint.tx_type, TxType::MintFungible);
    assert_eq!(mint.gas_from, owner);
    assert_eq!((mint.max_gas, mint.max_data, mint.expiry), (1, 2, 3));
    let TxPayload::MintFungible(inner) = &mint.msg else {
        panic!("payload = {:?}", mint.msg);
    };
    assert_eq!(inner.amount, IntX::from(100i64));

    let burn = build_burn_fungible_tx(BurnFungibleParams {
        from: owner,
        token_id: 9,
        amount: IntX::from(1i64),
        ..BurnFungibleParams::default()
    });
    assert_eq!(burn.tx_type, TxType::BurnFungible);
    assert_eq!(burn.gas_from, owner);
    let paid_burn = build_burn_fungible_tx(BurnFungibleParams {
        from: owner,
        gas_payer: Some(payer),
        token_id: 9,
        amount: IntX::from(1i64),
        ..BurnFungibleParams::default()
    });
    assert_eq!(paid_burn.tx_type, TxType::BurnFungibleGasPayer);
    assert_eq!(paid_burn.gas_from, payer);

    let nft_burn = build_burn_non_fungible_tx(BurnNonFungibleParams {
        from: owner,
        token_id: 7,
        instance_id: 42,
        ..BurnNonFungibleParams::default()
    });
    assert_eq!(nft_burn.tx_type, TxType::BurnNonFungible);
    let TxPayload::BurnNonFungible(inner) = &nft_burn.msg else {
        panic!("payload = {:?}", nft_burn.msg);
    };
    assert_eq!(inner.instance_id, 42);
    let paid_nft_burn = build_burn_non_fungible_tx(BurnNonFungibleParams {
        from: owner,
        gas_payer: Some(payer),
        token_id: 7,
        instance_id: 42,
        ..BurnNonFungibleParams::default()
    });
    assert_eq!(paid_nft_burn.tx_type, TxType::BurnNonFungibleGasPayer);
    let TxPayload::BurnNonFungibleGasPayer(inner) = &paid_nft_burn.msg else {
        panic!("payload = {:?}", paid_nft_burn.msg);
    };
    assert_eq!(inner.from_address, owner);
}

#[test]
fn stamps_the_default_lifetime_from_now_unless_an_expiry_is_given() {
    let before = now_unix_millis();
    let expiry = default_expiry();
    let after = now_unix_millis();
    let lifetime = DEFAULT_TX_EXPIRY.as_millis() as i64;
    assert!(expiry >= before + lifetime && expiry <= after + lifetime);
    let msg = build_transfer_fungible_tx(TransferFungibleParams::default());
    assert!(msg.expiry >= before + lifetime);
    let explicit = build_transfer_fungible_tx(TransferFungibleParams {
        limits: TxLimits {
            max_gas: 1,
            max_data: 2,
            expiry: 3,
        },
        ..TransferFungibleParams::default()
    });
    assert_eq!(
        (explicit.max_gas, explicit.max_data, explicit.expiry),
        (1, 2, 3)
    );
}

#[test]
fn expiry_within_uses_the_chains_window_less_a_margin() {
    let before = now_unix_millis();
    let expiry = expiry_within(Duration::from_secs(3600), Duration::ZERO).unwrap();
    let lifetime = (Duration::from_secs(3600) - Duration::from_secs(5)).as_millis() as i64;
    assert!(expiry >= before + lifetime && expiry <= now_unix_millis() + lifetime);
    let custom = expiry_within(Duration::from_secs(60), Duration::from_secs(10)).unwrap();
    assert!(custom - before >= 50_000 && custom - before <= 50_000 + 1_000);
    assert!(expiry_within(Duration::from_secs(10), Duration::from_secs(10)).is_err());
    assert!(expiry_within(Duration::ZERO, Duration::ZERO).is_err());
}

#[test]
fn create_token_and_sign_plans_the_message_before_signing() {
    let (payer_keys, _) = keys();
    let payer = address_of(&payer_keys);
    let info = token_info(payer);
    let expected = plan_fees(
        &build_create_token_tx(info.clone(), payer, TxLimits::default()).unwrap(),
        &config(),
        &one_witness(),
    )
    .unwrap();
    let signed = build_create_token_tx_and_sign(
        info.clone(),
        &payer_keys,
        Some(&config()),
        &PlanAndSignOptions::default(),
    )
    .unwrap();
    assert_eq!(signed.len() as u32, expected.envelope_bytes);
    let decoded: SignedTxMsg = deserialize(&signed).unwrap();
    assert_eq!(
        (decoded.msg.max_gas, decoded.msg.max_data),
        (expected.max_gas, expected.max_data)
    );
    let as_hex = build_create_token_tx_and_sign_hex(
        info,
        &payer_keys,
        Some(&config()),
        &PlanAndSignOptions {
            limits: TxLimits {
                expiry: decoded.msg.expiry,
                ..TxLimits::default()
            },
            ..PlanAndSignOptions::default()
        },
    )
    .unwrap();
    assert_eq!(as_hex, hex::encode(&signed));
}

#[test]
fn keeps_an_offer_the_caller_fixed_and_needs_no_config_for_it() {
    let (payer_keys, _) = keys();
    let info = token_info(address_of(&payer_keys));
    let signed = build_create_token_tx_and_sign(
        info.clone(),
        &payer_keys,
        None,
        &PlanAndSignOptions {
            limits: TxLimits {
                max_gas: 55_000_000,
                max_data: 7,
                ..TxLimits::default()
            },
            ..PlanAndSignOptions::default()
        },
    )
    .unwrap();
    let decoded: SignedTxMsg = deserialize(&signed).unwrap();
    assert_eq!((decoded.msg.max_gas, decoded.msg.max_data), (55_000_000, 7));
    let err =
        build_create_token_tx_and_sign(info, &payer_keys, None, &PlanAndSignOptions::default())
            .unwrap_err();
    assert!(err.to_string().contains("gas config"), "{err}");
}

#[test]
fn series_and_phantasma_mint_conveniences_plan_their_calls() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let series = build_series_info(7, 0, 0, payer).unwrap();
    let series_signed = build_create_token_series_tx_and_sign(
        9,
        series.clone(),
        &payer_keys,
        Some(&config()),
        &PlanAndSignOptions::default(),
    )
    .unwrap();
    let series_plan = plan_fees(
        &build_create_token_series_tx(9, series, payer, TxLimits::default()).unwrap(),
        &config(),
        &one_witness(),
    )
    .unwrap();
    let decoded: SignedTxMsg = deserialize(&series_signed).unwrap();
    assert_eq!(decoded.msg.max_gas, series_plan.max_gas);

    let schemas = prepare_standard_token_schemas(false);
    let rom = build_phantasma_nft_rom(
        &schemas.rom,
        &[
            ("name", VMValue::String("Planned".into())),
            ("description", VMValue::String("planned".into())),
            (
                "imageURL",
                VMValue::String("https://example.com/i.png".into()),
            ),
            ("infoURL", VMValue::String("https://example.com".into())),
            ("royalties", VMValue::Int(10_000_000)),
        ],
    )
    .unwrap();
    let facts = PlanAndSignOptions {
        facts: FeePlanOptions {
            duplicated_series: Some(false),
            supply_row_exists: true,
            ..FeePlanOptions::default()
        },
        ..PlanAndSignOptions::default()
    };
    let mint_signed = build_mint_phantasma_non_fungible_single_tx_and_sign(
        9,
        7,
        &payer_keys,
        owner,
        rom.clone(),
        vec![],
        Some(&config()),
        &facts,
    )
    .unwrap();
    let unsigned = build_mint_phantasma_non_fungible_single_tx(
        9,
        7,
        payer,
        owner,
        rom.clone(),
        vec![],
        TxLimits::default(),
    )
    .unwrap();
    let expected = plan_fees(
        &unsigned,
        &config(),
        &FeePlanOptions {
            witness_count: Some(1),
            ..facts.facts.clone()
        },
    )
    .unwrap();
    let decoded_mint: SignedTxMsg = deserialize(&mint_signed).unwrap();
    assert_eq!(decoded_mint.msg.max_gas, expected.max_gas);
    assert_eq!(mint_signed.len() as u32, expected.envelope_bytes);
    let multi_signed = build_mint_phantasma_non_fungible_tx_and_sign(
        9,
        &payer_keys,
        owner,
        vec![PhantasmaNFTMintInfo {
            phantasma_series_id: IntX::from(7i64),
            rom,
            ram: vec![],
        }],
        Some(&config()),
        &facts,
    )
    .unwrap();
    assert_eq!(multi_signed.len(), mint_signed.len());
    assert!(
        build_mint_phantasma_non_fungible_tx(9, payer, owner, vec![], TxLimits::default()).is_err()
    );
}

#[test]
fn plan_and_sign_sizes_the_witnesses_from_the_message_or_the_keys() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let msg = build_transfer_fungible_tx(TransferFungibleParams {
        from: owner,
        gas_payer: Some(payer),
        to: repeated(0x33),
        token_id: 1,
        amount: 5,
        ..TransferFungibleParams::default()
    });
    let signed = plan_and_sign_with_keys(
        &msg,
        &[&owner_keys, &payer_keys],
        Some(&config()),
        &PlanAndSignOptions::default(),
    )
    .unwrap();
    assert_eq!(signed.len(), 170 + 32 + 64);

    let call = build_create_token_tx(token_info(payer), payer, TxLimits::default()).unwrap();
    let two = plan_and_sign_with_keys(
        &call,
        &[&payer_keys, &owner_keys],
        Some(&config()),
        &PlanAndSignOptions::default(),
    )
    .unwrap();
    let one = plan_and_sign_with_keys(
        &call,
        &[&payer_keys],
        Some(&config()),
        &PlanAndSignOptions::default(),
    )
    .unwrap();
    assert_eq!(two.len() - one.len(), 96, "two keys size two witnesses");
}
