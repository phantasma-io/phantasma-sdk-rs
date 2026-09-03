//! Fee calculator tests. Every v2 expectation below is a bill a real transaction was actually
//! charged on a gas-model-v2 network, read back from its settlement: a failure here means the SDK
//! disagrees with the chain, not that a number was chosen badly. The same fixtures and
//! expectations exist in every SDK, so a divergence between SDKs shows up as a failure in one of
//! them.

use phantasma_sdk::{
    envelope_bytes_for, estimate_native_fee, phantasma_canonical_rom_bytes, storage_quanta_for,
    GasConfig, InfusedAsset, NativeFeeEstimate, NativeFeeKind, NativeFeeParams,
};

/// The live mainnet v1 configuration: multiplier 10000, shift 0, transfer 10 units, byte fee
/// 250000 kcal-base, minimum offer 10, escrow 2 atoms per row.
fn v1_config() -> GasConfig {
    GasConfig {
        version: 0,
        max_name_length: 32,
        max_token_symbol_length: 10,
        fee_shift: 0,
        fee_multiplier: 10_000,
        gas_token_id: 1,
        data_token_id: 2,
        minimum_gas_offer: 10,
        data_escrow_per_row: 2,
        gas_fee_transfer: 10,
        gas_fee_query: 10,
        gas_fee_create_token_base: 10_000_000_000,
        gas_fee_create_token_symbol: 10_000_000_000,
        gas_fee_create_token_series: 2_500_000_000,
        gas_fee_per_byte: 250_000,
        gas_fee_register_name: 10_000_000_000_000,
        gas_burn_ratio_mul: 1,
        ..GasConfig::default()
    }
}

/// The mainnet / testnet gas-model-v2 configuration (special resolutions #79 and #68 carry the
/// same 29 fields).
fn v2_config() -> GasConfig {
    GasConfig {
        version: 1,
        max_name_length: 255,
        max_token_symbol_length: 255,
        data_escrow_per_row: 200_000,
        legacy_data_escrow_per_row: 2,
        minimum_gas_bill: 10_000_000,
        policy_fee_create_token_base: 100_000_000_000_000,
        policy_fee_create_token_symbol: 100_000_000_000_000,
        policy_fee_create_token_series: 25_000_000_000_000,
        policy_fee_register_name: 100_000_000_000_000_000,
        ..v1_config()
    }
}

/// The localnet configuration of the 2026-08-28 live run: policy fees at the 10 KCAL scale, escrow
/// 50,000 atoms per row, everything else as mainnet.
fn localnet_config() -> GasConfig {
    GasConfig {
        data_escrow_per_row: 50_000,
        policy_fee_create_token_base: 100_000_000_000,
        policy_fee_create_token_symbol: 100_000_000_000,
        policy_fee_create_token_series: 25_000_000_000,
        policy_fee_register_name: 1_000_000_000_000,
        ..v2_config()
    }
}

fn estimate(kind: NativeFeeKind, config: &GasConfig, params: NativeFeeParams) -> NativeFeeEstimate {
    estimate_native_fee(kind, config, &params).unwrap()
}

fn params(envelope_bytes: u32, token_id: u64) -> NativeFeeParams {
    NativeFeeParams {
        envelope_bytes,
        token_id: Some(token_id),
        ..NativeFeeParams::default()
    }
}

// A native SOUL or KCAL transfer is 170 signed bytes and escrows nothing, even into an address that
// has never held the token, because the gas and data token balance rows are free.
#[test]
fn bills_a_native_gas_or_data_token_transfer_for_its_envelope_alone() {
    for token_id in [1, 2] {
        let got = estimate(
            NativeFeeKind::TransferFungible,
            &v2_config(),
            params(170, token_id),
        );
        assert_eq!(got.expected_gas_bill, 42_600_000);
        assert_eq!(got.max_gas, 42_600_000);
        assert_eq!(got.max_data, 0);
        assert_eq!(got.new_storage_quanta, 0);
    }
}

// The same envelope moving an ordinary token into a fresh holder creates one paid balance row,
// which the chain adds to the byte count and escrows at the row price.
#[test]
fn bills_and_escrows_the_fresh_balance_row_of_an_ordinary_token_transfer() {
    let fresh = estimate(
        NativeFeeKind::TransferFungible,
        &v2_config(),
        params(170, 97),
    );
    assert_eq!(fresh.expected_gas_bill, 42_850_000);
    assert_eq!(fresh.max_data, 200_000);
    let held = estimate(
        NativeFeeKind::TransferFungible,
        &v2_config(),
        NativeFeeParams {
            recipient_holds_token: true,
            ..params(170, 97)
        },
    );
    assert_eq!(held.expected_gas_bill, 42_600_000);
    assert_eq!(held.max_data, 0);
}

// Mainnet tx 6C1A6412... (block 9,075,820): a PoltergeistLite transfer of 178 bytes was billed
// 44,600,000 - the figure the node printed in its "gas fees" abort.
#[test]
fn reproduces_the_first_live_mainnet_bill() {
    let got = estimate(
        NativeFeeKind::TransferFungible,
        &v2_config(),
        params(178, 1),
    );
    assert_eq!(got.expected_gas_bill, 44_600_000);
}

// A native MintFungible of 171 bytes into a fresh holder of a token whose supply row is in place.
// The call returns the new balance, and a call result is block data exactly like the envelope. The
// result's size depends on the balance it reports: 9 bytes while it fits int64, up to 33 beyond -
// so stating `big_fungible: Some(false)` prices the 9-byte result exactly, and leaving it unstated
// prices the 33-byte maximum, 24 bytes of block data more.
#[test]
fn bills_a_fungible_mint_with_its_nine_byte_result_when_the_token_is_declared_small() {
    let exact = estimate(
        NativeFeeKind::MintFungible,
        &v2_config(),
        NativeFeeParams {
            big_fungible: Some(false),
            supply_row_exists: true,
            ..params(171, 97)
        },
    );
    assert_eq!(exact.expected_gas_bill, 45_350_000);
    assert_eq!(exact.max_data, 200_000);
    let defaulted = estimate(
        NativeFeeKind::MintFungible,
        &v2_config(),
        NativeFeeParams {
            supply_row_exists: true,
            ..params(171, 97)
        },
    );
    assert_eq!(
        defaulted.expected_gas_bill - exact.expected_gas_bill,
        24 * 25 * 10_000
    );
}

// An NFT transfer of 170 bytes into a fresh holder. The owner's lookup row is deleted and the
// recipient's created (net zero), the fresh balance row is the one net quantum; both new rows are
// escrowed, the deleted row's deposit comes back at its own price.
#[test]
fn bills_an_nft_transfer_for_its_net_rows_and_escrows_its_new_ones() {
    let got = estimate(
        NativeFeeKind::TransferNonFungible,
        &v2_config(),
        params(170, 7),
    );
    assert_eq!(got.expected_gas_bill, 42_850_000);
    assert_eq!(got.new_storage_quanta, 2);
    assert_eq!(got.deleted_storage_quanta, 1);
    assert_eq!(got.max_data, 400_000);
}

// The same transfer into an NFT-derived address (an infusion) pays one query fee more for the
// owner check of the target; every mint kind pays the same lookup once per call.
#[test]
fn adds_the_query_fee_of_an_infusion_target() {
    let got = estimate(
        NativeFeeKind::TransferNonFungible,
        &v2_config(),
        NativeFeeParams {
            to_is_nft_address: true,
            ..params(170, 7)
        },
    );
    assert_eq!(got.expected_gas_bill, 42_950_000);
    for kind in [
        NativeFeeKind::MintFungible,
        NativeFeeKind::MintNonFungible,
        NativeFeeKind::MintPhantasmaNonFungible,
    ] {
        let plain = NativeFeeParams {
            rom_bytes: vec![10],
            ..params(200, 7)
        };
        let infused = NativeFeeParams {
            to_is_nft_address: true,
            ..plain.clone()
        };
        assert_eq!(
            estimate(kind, &v2_config(), infused).expected_gas_bill
                - estimate(kind, &v2_config(), plain).expected_gas_bill,
            100_000,
            "{kind:?}"
        );
    }
}

// A native NFT burn of 138 bytes; the mint's ten quanta are deleted and refunded, no net block
// data, the token had been burned before and its supply row was in place.
#[test]
fn bills_an_nft_burn_for_its_work_and_envelope_only() {
    let got = estimate(
        NativeFeeKind::BurnNonFungible,
        &v2_config(),
        NativeFeeParams {
            rom_bytes: vec![3067 * 2 + 36],
            rom_has_meta_id: Some(true),
            token_burned_before: true,
            supply_row_exists: true,
            ..params(138, 7)
        },
    );
    assert_eq!(got.expected_gas_bill, 34_800_000);
    assert_eq!(got.deleted_storage_quanta, 10);
    assert_eq!(got.new_storage_quanta, 0);
    assert_eq!(got.max_data, 0);
}

// Burning an NFT returns whatever its own address holds, and the chain charges for each returned
// asset as the transfers it performs: a transfer fee plus the owner lookup of the NFT-address
// source per fungible token; an instance query, a transfer per instance and that lookup per NFT
// token. The returned rows never add block data - a burn refunds more than the returns create - so
// the bill moves by the work alone, while the escrow ceiling covers a balance row the burner lacks
// and the moved lookup rows. Measured live 2026-09-02: +200,000 for one infused KCAL atom, +700,000
// for KCAL, a custom token and an NFT together.
#[test]
fn prices_the_assets_a_burned_nft_returns() {
    let burn = |infusions: Vec<InfusedAsset>| {
        estimate(
            NativeFeeKind::BurnNonFungible,
            &v2_config(),
            NativeFeeParams {
                rom_bytes: vec![3067 * 2 + 36],
                token_burned_before: true,
                supply_row_exists: true,
                infusions: Some(infusions),
                ..params(138, 7)
            },
        )
    };
    let empty = burn(vec![]);
    assert_eq!(empty.expected_gas_bill, 34_800_000);

    // One fungible token: a transfer and a query. The gas token's rows are free, so nothing else.
    let kcal = burn(vec![InfusedAsset {
        token_id: Some(1),
        ..InfusedAsset::default()
    }]);
    assert_eq!(
        kcal.expected_gas_bill - empty.expected_gas_bill,
        20 * 10_000
    );
    assert_eq!(kcal.new_storage_quanta, empty.new_storage_quanta);
    assert_eq!(kcal.deleted_storage_quanta, empty.deleted_storage_quanta);

    // A custom token the burner does not hold: the same work, plus the balance row the return
    // creates - and the NFT address's own row, which the return deletes.
    let custom = burn(vec![InfusedAsset {
        token_id: Some(97),
        ..InfusedAsset::default()
    }]);
    assert_eq!(
        custom.expected_gas_bill - empty.expected_gas_bill,
        20 * 10_000
    );
    assert_eq!(custom.new_storage_quanta, empty.new_storage_quanta + 1);
    assert_eq!(
        custom.deleted_storage_quanta,
        empty.deleted_storage_quanta + 1
    );
    assert_eq!(
        custom.max_data,
        empty.max_data + v2_config().data_escrow_per_row
    );
    let held = burn(vec![InfusedAsset {
        token_id: Some(97),
        burner_holds_token: true,
        ..InfusedAsset::default()
    }]);
    assert_eq!(held.new_storage_quanta, empty.new_storage_quanta);
    // An id the reader could not resolve is priced as a paid row: over-covering, never short.
    assert_eq!(
        burn(vec![InfusedAsset::default()]).new_storage_quanta,
        empty.new_storage_quanta + 1
    );

    // Two instances of an NFT token: the instance query, two transfers, the lookup; a balance row
    // plus two moved lookup rows created, the NFT address's balance row and two lookups deleted.
    let nft = burn(vec![InfusedAsset {
        token_id: Some(9),
        non_fungible: true,
        instance_count: Some(2),
        ..InfusedAsset::default()
    }]);
    assert_eq!(nft.expected_gas_bill - empty.expected_gas_bill, 40 * 10_000);
    assert_eq!(nft.new_storage_quanta, empty.new_storage_quanta + 3);
    assert_eq!(nft.deleted_storage_quanta, empty.deleted_storage_quanta + 3);

    // The live combination: KCAL, a held custom token and one NFT - seventy units.
    let all = burn(vec![
        InfusedAsset {
            token_id: Some(1),
            ..InfusedAsset::default()
        },
        InfusedAsset {
            token_id: Some(97),
            burner_holds_token: true,
            ..InfusedAsset::default()
        },
        InfusedAsset {
            token_id: Some(9),
            non_fungible: true,
            instance_count: Some(1),
            burner_holds_token: true,
        },
    ]);
    assert_eq!(all.expected_gas_bill - empty.expected_gas_bill, 70 * 10_000);

    let zero_instances = estimate_native_fee(
        NativeFeeKind::BurnNonFungible,
        &v2_config(),
        &NativeFeeParams {
            infusions: Some(vec![InfusedAsset {
                token_id: Some(9),
                non_fungible: true,
                instance_count: Some(0),
                ..InfusedAsset::default()
            }]),
            ..params(138, 7)
        },
    );
    assert!(zero_instances
        .unwrap_err()
        .to_string()
        .contains("instance_count"));
}

// Two settled CreateToken bills with 7-character symbols. The fungible one is 374 signed bytes and
// writes the symbol, token-info and null-balance rows and returns the u64 token id; the
// NFT-capable one (466 bytes) adds the series counter. Policy fee 10 KCAL + 10 KCAL >> 6.
#[test]
fn reproduces_the_two_localnet_create_token_bills_to_the_atom() {
    // The Call arguments ARE the serialized TokenInfo, so their length is the envelope minus the
    // 100-byte transaction header and the 58 bytes the Call header and witness array add.
    let fungible = estimate(
        NativeFeeKind::CreateToken,
        &localnet_config(),
        NativeFeeParams {
            envelope_bytes: 374,
            symbol_length: 7,
            token_info_bytes: 374 - 100 - 58,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(fungible.expected_gas_bill, 101_658_750_000);
    assert_eq!(fungible.new_storage_quanta, 3);
    assert_eq!(fungible.max_data, 150_000);

    let nft = estimate(
        NativeFeeKind::CreateToken,
        &localnet_config(),
        NativeFeeParams {
            envelope_bytes: 466,
            symbol_length: 7,
            token_info_bytes: 466 - 100 - 58,
            non_fungible: true,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(nft.expected_gas_bill, 101_682_000_000);
    assert_eq!(nft.new_storage_quanta, 4);
    assert_eq!(nft.max_data, 200_000);
}

// Validating token metadata that names a staking organisation looks the organisation up, and
// metadata that names a reward token reads that token: one query fee each, on top of the policy
// fee and the rows.
#[test]
fn charges_the_lookups_that_staking_metadata_costs_a_token_creation() {
    let create = |org: bool, reward: bool| {
        estimate(
            NativeFeeKind::CreateToken,
            &localnet_config(),
            NativeFeeParams {
                envelope_bytes: 374,
                symbol_length: 7,
                token_info_bytes: 374 - 100 - 58,
                has_staking_organisation: org,
                has_staking_reward_token: reward,
                ..NativeFeeParams::default()
            },
        )
    };
    let plain = create(false, false);
    assert_eq!(
        create(true, false).expected_gas_bill - plain.expected_gas_bill,
        100_000
    );
    assert_eq!(
        create(false, true).expected_gas_bill - plain.expected_gas_bill,
        100_000
    );
    let both = create(true, true);
    assert_eq!(both.expected_gas_bill - plain.expected_gas_bill, 200_000);
    assert_eq!(both.new_storage_quanta, plain.new_storage_quanta);
}

// A settled CreateTokenSeries of 246 bytes with a `_i` in its metadata writes the series info, its
// supply and the meta-id lookup and returns the u32 series id.
#[test]
fn reproduces_the_localnet_create_token_series_bill() {
    let got = estimate(
        NativeFeeKind::CreateTokenSeries,
        &localnet_config(),
        NativeFeeParams {
            envelope_bytes: 246,
            // As above, less the 8-byte token id that precedes the SeriesInfo in the Call arguments.
            series_info_bytes: 246 - 100 - 58 - 8,
            series_has_meta_id: Some(true),
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(got.expected_gas_bill, 25_063_250_000);
    assert_eq!(got.new_storage_quanta, 3);
}

// Two settled deterministic Phantasma mints. The 182-byte public ROM was the token's first mint to
// that owner (fresh balance row, 5 quanta); the 3,083-byte one found the balance row in place and
// its canonical ROM alone took seven quanta (10 in total). Each instance pays the mint plus two
// query fees and returns 40 bytes after the 4-byte count.
//
// Both were minted into a UNIQUE series of a token whose supply row was in place, and every
// settled-bill case says both explicitly: the series mode and the supply row are chain state the
// message does not carry, so the calculator assumes the costlier reading of each when nobody tells
// it. Leaving either out here would compare the chain's receipt against a deliberate over-estimate.
#[test]
fn reproduces_the_two_localnet_phantasma_nft_mint_bills() {
    let small = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            rom_bytes: vec![182],
            duplicated_series: Some(false),
            supply_row_exists: true,
            ..params(413, 9)
        },
    );
    assert_eq!(small.expected_gas_bill, 115_800_000);
    assert_eq!(small.new_storage_quanta, 5);
    assert_eq!(small.max_data, 250_000);

    let large = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            rom_bytes: vec![3083],
            recipient_holds_token: true,
            duplicated_series: Some(false),
            supply_row_exists: true,
            ..params(3314, 9)
        },
    );
    assert_eq!(large.expected_gas_bill, 842_300_000);
    assert_eq!(large.new_storage_quanta, 10);
    assert_eq!(large.max_data, 500_000);
}

// The same model at a different network's prices, including two 166-byte mints of the same size
// where only the first paid for the owner's balance row.
#[test]
fn reproduces_the_four_testnet_phantasma_nft_mint_bills() {
    for (envelope, rom, held, bill, quanta) in [
        (398, 167, false, 112_050_000, 5),
        (3298, 3067, true, 838_300_000, 10),
        (397, 166, false, 111_800_000, 5),
        (397, 166, true, 111_550_000, 4),
    ] {
        let got = estimate(
            NativeFeeKind::MintPhantasmaNonFungible,
            &v2_config(),
            NativeFeeParams {
                rom_bytes: vec![rom],
                recipient_holds_token: held,
                duplicated_series: Some(false),
                supply_row_exists: true,
                ..params(envelope, 9)
            },
        );
        assert_eq!(got.expected_gas_bill, bill);
        assert_eq!(got.new_storage_quanta, quanta);
    }
}

// A sweep of settled mints across the quantum boundary (public ROM bytes -> quanta, the first mint
// also paying for the balance row): 268 -> 5, 968 -> 5, 1168 -> 6, 3068 -> 10, 5068 -> 13. 968 and
// 1168 straddle the 1024-byte boundary, which is where a wrong ROM model shows up.
#[test]
fn reproduces_the_rom_size_sweep_quanta() {
    for (rom, held, quanta) in [
        (268, false, 5),
        (968, true, 5),
        (1168, true, 6),
        (3068, true, 10),
        (5068, true, 13),
    ] {
        let got = estimate(
            NativeFeeKind::MintPhantasmaNonFungible,
            &v2_config(),
            NativeFeeParams {
                envelope_bytes: 1000,
                rom_bytes: vec![rom],
                recipient_holds_token: held,
                supply_row_exists: true,
                ..NativeFeeParams::default()
            },
        );
        assert_eq!(got.new_storage_quanta, quanta, "ROM {rom}");
    }
}

// Several instances in one transaction: the work units, the per-instance query fees, the call
// result bytes and the per-instance storage all scale with the count, while the recipient's balance
// row is paid for exactly once. A settled three-instance Phantasma mint of 75-byte public ROMs, 490
// signed bytes: 13 quanta are one balance row plus, per instance, one ROM row and the three fixed
// rows a deterministic mint writes.
#[test]
fn scales_a_multi_instance_phantasma_mint_by_its_instance_count() {
    let three = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            envelope_bytes: 490,
            count: Some(3),
            rom_bytes: vec![75],
            duplicated_series: Some(false),
            supply_row_exists: true,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(three.expected_gas_bill, 157_650_000);
    assert_eq!(three.new_storage_quanta, 13);
    assert_eq!(three.max_data, 650_000);
    let single = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            envelope_bytes: 490,
            count: Some(1),
            rom_bytes: vec![75],
            duplicated_series: Some(false),
            supply_row_exists: true,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(single.new_storage_quanta, 5);
}

// A duplicated series is read twice per instance rather than once - the mode check and the shared
// ROM each read the token info - and its supply is read once per series for the whole transaction,
// because the chain reuses the number it already read. The counts therefore scale differently and
// the model has to keep them apart.
#[test]
fn charges_a_duplicated_series_three_queries_an_instance_and_one_for_the_series() {
    let shared = NativeFeeParams {
        envelope_bytes: 490,
        count: Some(3),
        rom_bytes: vec![75],
        duplicated_series: Some(true),
        ..NativeFeeParams::default()
    };
    let one_series = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            distinct_series_count: Some(1),
            ..shared.clone()
        },
    );
    let unique = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            duplicated_series: Some(false),
            ..shared.clone()
        },
    );
    // Three extra instance queries and one supply query over the unique-series bill.
    assert_eq!(
        one_series.expected_gas_bill - unique.expected_gas_bill,
        40 * 10_000
    );
    let three_series = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            distinct_series_count: Some(3),
            ..shared.clone()
        },
    );
    assert_eq!(
        three_series.expected_gas_bill - one_series.expected_gas_bill,
        20 * 10_000
    );
    // A series count nobody could have minted is a bookkeeping error, not a price.
    let impossible = estimate_native_fee(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        &NativeFeeParams {
            distinct_series_count: Some(4),
            ..shared
        },
    );
    assert!(impossible
        .unwrap_err()
        .to_string()
        .contains("distinct_series_count"));
}

// A settled two-instance NFT transfer of 182 signed bytes: each instance deletes the owner's lookup
// row and creates the recipient's, so only the recipient's new balance row is billed, while the
// escrow ceiling still covers all three rows the transaction may create.
#[test]
fn scales_a_multi_instance_nft_transfer_by_its_instance_count() {
    let got = estimate(
        NativeFeeKind::TransferNonFungible,
        &localnet_config(),
        NativeFeeParams {
            count: Some(2),
            ..params(182, 181)
        },
    );
    assert_eq!(got.expected_gas_bill, 45_950_000);
    assert_eq!(got.new_storage_quanta, 3);
    assert_eq!(got.deleted_storage_quanta, 2);
    assert_eq!(got.max_data, 150_000);
}

// A settled RegisterName of a 10-character name, 213 signed bytes. Governance rows are free data,
// so the bill is the policy fee (10,000,000 KCAL >> 9) plus the envelope and nothing else.
#[test]
fn reproduces_the_testnet_register_name_bill() {
    let got = estimate(
        NativeFeeKind::RegisterName,
        &v2_config(),
        NativeFeeParams {
            envelope_bytes: 213,
            name_length: 10,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(got.expected_gas_bill, 195_312_553_250_000);
    assert_eq!(got.max_data, 0);
}

// A tiny v2 tx can never bill below the consensus floor; the offer must also respect the admission
// check max_gas >= minimum_gas_bill.
#[test]
fn applies_the_minimum_bill_floor() {
    let config = GasConfig {
        minimum_gas_bill: 10_000_000_000,
        ..v2_config()
    };
    let got = estimate(NativeFeeKind::TransferFungible, &config, params(170, 1));
    assert_eq!(got.expected_gas_bill, 10_000_000_000);
    assert_eq!(got.max_gas, 10_000_000_000);
}

// Facts about chain state that the message cannot carry are defaulted to the case that COSTS MORE,
// because the offer is spent against the real bill and a short one aborts the transaction while an
// over-offer is refunded. These pin that direction; each case fails if a default flips back.
#[test]
fn assumes_a_phantasma_series_is_duplicated_until_told_otherwise() {
    let shared = NativeFeeParams {
        envelope_bytes: 490,
        count: Some(3),
        rom_bytes: vec![75],
        ..NativeFeeParams::default()
    };
    let assumed = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        shared.clone(),
    );
    let duplicated = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            duplicated_series: Some(true),
            ..shared.clone()
        },
    );
    let unique = estimate(
        NativeFeeKind::MintPhantasmaNonFungible,
        &localnet_config(),
        NativeFeeParams {
            duplicated_series: Some(false),
            ..shared
        },
    );
    // Three extra instance queries plus one supply query: 40 units at this config's multiplier.
    assert_eq!(
        duplicated.expected_gas_bill - unique.expected_gas_bill,
        40 * 10_000
    );
    assert_eq!(assumed.expected_gas_bill, duplicated.expected_gas_bill);
}

#[test]
fn assumes_a_minted_rom_carries_a_meta_id_which_is_a_row_it_must_escrow_for() {
    let shared = NativeFeeParams {
        rom_bytes: vec![64],
        ..params(300, 97)
    };
    let assumed = estimate(NativeFeeKind::MintNonFungible, &v2_config(), shared.clone());
    let with_meta_id = estimate(
        NativeFeeKind::MintNonFungible,
        &v2_config(),
        NativeFeeParams {
            rom_has_meta_id: Some(true),
            ..shared.clone()
        },
    );
    let without = estimate(
        NativeFeeKind::MintNonFungible,
        &v2_config(),
        NativeFeeParams {
            rom_has_meta_id: Some(false),
            ..shared
        },
    );
    assert_eq!(
        with_meta_id.new_storage_quanta,
        without.new_storage_quanta + 1
    );
    assert_eq!(assumed.new_storage_quanta, with_meta_id.new_storage_quanta);
    // max_data is the one that has to be right: the chain aborts a transaction whose escrow exceeds
    // it, so a row assumed away is not an under-offer but a failed transaction.
    assert_eq!(assumed.max_data, with_meta_id.max_data);
    assert_eq!(
        assumed.max_data - without.max_data,
        v2_config().data_escrow_per_row
    );
}

// Same flag, same default on a burn - it keeps the deleted rows mirroring what the mint wrote - but
// there it moves a reported number and nothing else. Deleted rows are refunded, max_data covers
// only the rows an operation CREATES, and a burn always deletes more than it creates, so the
// block-data term floors at zero whichever way the flag goes. This pins both halves: the count
// follows the flag, the price does not.
#[test]
fn counts_a_burned_meta_id_row_but_does_not_price_on_it() {
    let shared = NativeFeeParams {
        rom_bytes: vec![64],
        ..params(300, 97)
    };
    let assumed = estimate(NativeFeeKind::BurnNonFungible, &v2_config(), shared.clone());
    let without = estimate(
        NativeFeeKind::BurnNonFungible,
        &v2_config(),
        NativeFeeParams {
            rom_has_meta_id: Some(false),
            ..shared
        },
    );
    assert_eq!(
        assumed.deleted_storage_quanta,
        without.deleted_storage_quanta + 1
    );
    assert_eq!(assumed.quote(), without.quote());
}

#[test]
fn assumes_a_created_series_carries_a_meta_id_which_is_one_more_row() {
    let shared = NativeFeeParams {
        envelope_bytes: 300,
        series_info_bytes: 100,
        ..NativeFeeParams::default()
    };
    let assumed = estimate(
        NativeFeeKind::CreateTokenSeries,
        &v2_config(),
        shared.clone(),
    );
    let without = estimate(
        NativeFeeKind::CreateTokenSeries,
        &v2_config(),
        NativeFeeParams {
            series_has_meta_id: Some(false),
            ..shared
        },
    );
    assert_eq!(assumed.new_storage_quanta, without.new_storage_quanta + 1);
}

// The supply row disappears when its balance reaches exactly zero - a limited token fully in
// circulation, an unlimited one with nothing outstanding - and the next mint or burn recreates it.
// Unstated, every mint and burn prices that recreation: one quantum in the bill and the escrow
// ceiling. Transfers never touch the row, and the chain's own gas and data tokens are free rows, so
// neither moves with the flag.
#[test]
fn assumes_the_supply_row_must_be_recreated_on_mints_and_burns() {
    for kind in [
        NativeFeeKind::MintFungible,
        NativeFeeKind::BurnFungible,
        NativeFeeKind::MintNonFungible,
        NativeFeeKind::MintPhantasmaNonFungible,
        NativeFeeKind::BurnNonFungible,
    ] {
        let shared = NativeFeeParams {
            rom_bytes: vec![64],
            ..params(300, 97)
        };
        let assumed = estimate(kind, &v2_config(), shared.clone());
        let in_place = estimate(
            kind,
            &v2_config(),
            NativeFeeParams {
                supply_row_exists: true,
                ..shared
            },
        );
        assert_eq!(
            assumed.new_storage_quanta,
            in_place.new_storage_quanta + 1,
            "{kind:?}"
        );
        assert_eq!(
            assumed.max_data - in_place.max_data,
            v2_config().data_escrow_per_row,
            "{kind:?}"
        );
        // An NFT burn deletes more quanta than it creates, so its block-data term floors at zero
        // either way and only the escrow ceiling moves; everywhere else the bill moves too.
        let want = if kind == NativeFeeKind::BurnNonFungible {
            0
        } else {
            25 * 10_000
        };
        assert_eq!(
            assumed.expected_gas_bill - in_place.expected_gas_bill,
            want,
            "{kind:?}"
        );
    }

    let transfer = params(300, 97);
    assert_eq!(
        estimate(
            NativeFeeKind::TransferFungible,
            &v2_config(),
            transfer.clone()
        ),
        estimate(
            NativeFeeKind::TransferFungible,
            &v2_config(),
            NativeFeeParams {
                supply_row_exists: true,
                ..transfer
            }
        )
    );
    let gas_token = params(300, 1);
    assert_eq!(
        estimate(NativeFeeKind::BurnFungible, &v2_config(), gas_token.clone()),
        estimate(
            NativeFeeKind::BurnFungible,
            &v2_config(),
            NativeFeeParams {
                supply_row_exists: true,
                ..gas_token
            }
        )
    );
}

#[test]
fn requires_the_envelope_size_under_gas_model_v2() {
    let err = estimate_native_fee(
        NativeFeeKind::TransferFungible,
        &v2_config(),
        &NativeFeeParams::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("envelope_bytes is required"));
}

// v1 transfer with an existing recipient row: bill is the pure work term 10 * 10000.
#[test]
fn v1_bills_work_only_for_a_transfer_to_an_existing_recipient() {
    let got = estimate(
        NativeFeeKind::TransferFungible,
        &v1_config(),
        NativeFeeParams {
            recipient_holds_token: true,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(got.expected_gas_bill, 100_000);
    // stdFee shape: 2x min offer + work + flat 1 KiB byte allowance.
    assert_eq!(got.max_gas, 10 * 2 + 100_000 + 1024 * 250_000);
    assert_eq!(got.max_data, 0);
}

// v1 transfer default (worst case: 1 fresh row): the row quantum joins the byte fee and the escrow
// shows up in max_data at the v1 price.
#[test]
fn v1_includes_one_fresh_row_by_default() {
    let got = estimate(
        NativeFeeKind::TransferFungible,
        &v1_config(),
        NativeFeeParams::default(),
    );
    assert_eq!(got.expected_gas_bill, 100_000 + 250_000);
    assert_eq!(got.max_data, 2);
}

// CreateToken under v1 charges unit-priced product fees through the multiplier; the 8-byte result
// and the rows are block data at the v1 byte price.
#[test]
fn v1_prices_token_creation_through_the_multiplier() {
    let got = estimate(
        NativeFeeKind::CreateToken,
        &v1_config(),
        NativeFeeParams {
            symbol_length: 4,
            token_info_bytes: 200,
            ..NativeFeeParams::default()
        },
    );
    let work = (10_000_000_000u64 + 1_250_000_000) * 10_000;
    assert_eq!(got.expected_gas_bill, work + (8 + 3) * 250_000);
}

// RegisterName halves the price per character after the first, under both models.
#[test]
fn halves_the_name_price_per_character_under_both_models() {
    let v1 = estimate(
        NativeFeeKind::RegisterName,
        &v1_config(),
        NativeFeeParams {
            name_length: 8,
            ..NativeFeeParams::default()
        },
    );
    let v2 = estimate(
        NativeFeeKind::RegisterName,
        &v2_config(),
        NativeFeeParams {
            name_length: 8,
            envelope_bytes: 300,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(v1.expected_gas_bill, (10_000_000_000_000u64 >> 7) * 10_000);
    assert_eq!(
        v2.expected_gas_bill,
        (100_000_000_000_000_000u64 >> 7) + 300 * 25 * 10_000
    );
}

// fee_shift semantics: the chain clamps shifts >= 64 to a zero work delta; the estimator must match
// rather than undercharge/overcharge.
#[test]
fn zeroes_scaled_terms_on_an_oversized_fee_shift() {
    let config = GasConfig {
        fee_shift: 64,
        ..v1_config()
    };
    let got = estimate(
        NativeFeeKind::TransferFungible,
        &config,
        NativeFeeParams {
            recipient_holds_token: true,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(got.expected_gas_bill, 0);
}

// The Script kind budgets a generous VM unit allowance (default 5000 exceeds every script in
// mainnet history), event bytes and storage rows instead of pretending opcode costs are closed-form.
#[test]
fn budgets_a_vm_allowance_for_scripts() {
    let got = estimate(
        NativeFeeKind::Script,
        &v2_config(),
        NativeFeeParams {
            envelope_bytes: 568,
            script_storage_quanta: Some(0),
            ..NativeFeeParams::default()
        },
    );
    // (5000 vm units + (568 + 512 events) * 25) * 10000
    assert_eq!(got.expected_gas_bill, (5000 + 1080 * 25) * 10_000);
    let defaulted = estimate(
        NativeFeeKind::Script,
        &v2_config(),
        NativeFeeParams {
            envelope_bytes: 568,
            ..NativeFeeParams::default()
        },
    );
    assert_eq!(defaulted.new_storage_quanta, 4);
}

// Envelope arithmetic mirrors SignedTxMsg: native kinds append bare 64-byte signatures, call/script
// kinds append a length-prefixed 96-byte witness array. The witness count is always stated: a fee
// kind cannot tell a two-signature gas-payer transfer from its one-signature form.
#[test]
fn envelope_bytes_for_follows_the_witness_layout() {
    assert_eq!(
        envelope_bytes_for(NativeFeeKind::TransferFungible, 150, 1),
        150 + 64
    );
    assert_eq!(
        envelope_bytes_for(NativeFeeKind::TransferFungible, 150, 2),
        150 + 128
    );
    assert_eq!(
        envelope_bytes_for(NativeFeeKind::CreateToken, 900, 1),
        900 + 4 + 96
    );
    assert_eq!(
        envelope_bytes_for(NativeFeeKind::Script, 500, 2),
        500 + 4 + 192
    );
}

#[test]
fn measures_rows_in_quanta_and_the_canonical_rom_at_twice_the_public_one() {
    assert_eq!(storage_quanta_for(0), 0);
    assert_eq!(storage_quanta_for(1024), 1);
    assert_eq!(storage_quanta_for(1025), 2);
    assert_eq!(phantasma_canonical_rom_bytes(182), 400);
}

// The chain's length-halved fee is defined up to 64 characters, and the calculator refuses to price
// anything longer. A 64-character name is the longest priceable one and must still price.
#[test]
fn refuses_to_price_a_name_or_symbol_past_the_length_the_chain_can_shift() {
    let config = v2_config();
    let longest = estimate(
        NativeFeeKind::RegisterName,
        &config,
        NativeFeeParams {
            envelope_bytes: 300,
            name_length: 64,
            ..NativeFeeParams::default()
        },
    );
    assert!(longest.expected_gas_bill > 0);
    let name = estimate_native_fee(
        NativeFeeKind::RegisterName,
        &config,
        &NativeFeeParams {
            envelope_bytes: 300,
            name_length: 65,
            ..NativeFeeParams::default()
        },
    )
    .unwrap_err();
    assert!(name.to_string().contains("cannot be priced offline"));
    let symbol = estimate_native_fee(
        NativeFeeKind::CreateToken,
        &config,
        &NativeFeeParams {
            envelope_bytes: 300,
            symbol_length: 65,
            token_info_bytes: 100,
            ..NativeFeeParams::default()
        },
    )
    .unwrap_err();
    assert!(symbol.to_string().contains("cannot be priced offline"));
}

// Impossible inputs are rejected instead of quoting fees for txs the chain would never admit.
#[test]
fn rejects_invalid_inputs() {
    assert!(estimate_native_fee(
        NativeFeeKind::TransferFungible,
        &v1_config(),
        &NativeFeeParams {
            count: Some(0),
            ..NativeFeeParams::default()
        },
    )
    .is_err());
    assert!(estimate_native_fee(
        NativeFeeKind::RegisterName,
        &v1_config(),
        &NativeFeeParams::default(),
    )
    .is_err());
    // max_token_symbol_length is 10 on the v1 config.
    assert!(estimate_native_fee(
        NativeFeeKind::CreateToken,
        &v1_config(),
        &NativeFeeParams {
            symbol_length: 11,
            ..NativeFeeParams::default()
        },
    )
    .is_err());
    let mismatch = estimate_native_fee(
        NativeFeeKind::MintNonFungible,
        &v2_config(),
        &NativeFeeParams {
            envelope_bytes: 300,
            count: Some(2),
            rom_bytes: vec![10, 20, 30],
            ..NativeFeeParams::default()
        },
    )
    .unwrap_err();
    assert!(mismatch.to_string().contains("one entry per instance"));
}
