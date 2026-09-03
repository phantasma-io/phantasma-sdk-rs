//! Gas-model-v2 GasConfig wire format and getGasConfig conversion tests.
//!
//! The chain serializes the 10 v2 config fields only for version >= 1; the version-0 image is
//! frozen forever (historical replay). The calculator itself is tested in
//! `tests/native_fee_estimator.rs`.

use phantasma_sdk::rpc::{GasConfigDataResult, GasConfigResult};
use phantasma_sdk::{deserialize, serialize, GasConfig};

/// The mainnet v1 values (feeMultiplier 10000, transfer 10 units, byte fee 250000, escrow 2).
fn live_v1_config() -> GasConfig {
    GasConfig {
        version: 0,
        max_name_length: 32,
        max_token_symbol_length: 10,
        fee_shift: 0,
        max_structure_size: 65536,
        fee_multiplier: 10_000,
        gas_token_id: 2,
        data_token_id: 1,
        minimum_gas_offer: 10,
        data_escrow_per_row: 2,
        gas_fee_transfer: 10,
        gas_fee_query: 2,
        gas_fee_create_token_base: 10_000_000_000,
        gas_fee_create_token_symbol: 10_000_000_000,
        gas_fee_create_token_series: 2_500_000_000,
        gas_fee_per_byte: 250_000,
        gas_fee_register_name: 10_000_000_000_000,
        gas_burn_ratio_mul: 1,
        ..GasConfig::default()
    }
}

/// The spec activation-package values for the v2 tail.
fn v2_config() -> GasConfig {
    GasConfig {
        version: 1,
        data_escrow_per_row: 200_000,
        minimum_gas_bill: 10_000_000,
        policy_fee_create_token_base: 100_000_000_000_000,
        policy_fee_create_token_symbol: 100_000_000_000_000,
        policy_fee_create_token_series: 25_000_000_000_000,
        policy_fee_register_name: 100_000_000_000_000_000,
        legacy_data_escrow_per_row: 2,
        ..live_v1_config()
    }
}

#[test]
fn v0_keeps_legacy_113_byte_layout() {
    // Any growth of the version-0 image would corrupt every historical block image.
    assert_eq!(serialize(&live_v1_config()).unwrap().len(), 113);
}

#[test]
fn v2_appends_66_byte_tail_after_unchanged_head() {
    let v2_bytes = serialize(&v2_config()).unwrap();
    assert_eq!(v2_bytes.len(), 179);

    let v0_twin = GasConfig {
        version: 0, // same head values, version-0 layout
        ..v2_config()
    };
    let v0_bytes = serialize(&v0_twin).unwrap();
    assert_eq!(v0_bytes.len(), 113);

    assert_eq!(v2_bytes[0], 1);
    assert_eq!(v0_bytes[0], 0);
    // The tail is a pure wire extension: the head encoding must be untouched.
    assert_eq!(&v2_bytes[1..113], &v0_bytes[1..113]);
}

#[test]
fn v2_roundtrip_preserves_all_fields() {
    let original = v2_config();
    let decoded: GasConfig = deserialize(serialize(&original).unwrap()).unwrap();
    assert_eq!(decoded, original);
    assert!(decoded.has_gas_model_v2());
}

#[test]
fn v0_read_zeroes_v2_fields() {
    // The deserializer starts from defaults, so a version-0 image must leave every v2 field at
    // zero - consumers must never see stale tail values on a v1 chain.
    let decoded: GasConfig = deserialize(serialize(&live_v1_config()).unwrap()).unwrap();
    assert!(!decoded.has_gas_model_v2());
    assert_eq!(decoded.minimum_gas_bill, 0);
    assert_eq!(decoded.policy_fee_create_token_base, 0);
    assert_eq!(decoded.legacy_data_escrow_per_row, 0);
}

#[test]
fn truncated_v2_image_fails_to_parse() {
    // Never silently produce a config with zeroed v2 prices (free product actions).
    let truncated = serialize(&v2_config()).unwrap()[..113].to_vec();
    assert!(deserialize::<GasConfig>(truncated).is_err());
}

fn v2_result() -> GasConfigResult {
    GasConfigResult {
        gas_model_version: 2,
        block_rate_target: 2000,
        expiry_window: 90_000,
        units_per_block_data_byte: Some(25),
        gas_config: Some(GasConfigDataResult {
            version: 1,
            max_name_length: 32,
            max_token_symbol_length: 10,
            fee_shift: 0,
            max_structure_size: 65536,
            fee_multiplier: Some("10000".into()),
            gas_token_id: Some("2".into()),
            data_token_id: Some("1".into()),
            minimum_gas_offer: Some("10".into()),
            data_escrow_per_row: Some("200000".into()),
            gas_fee_transfer: Some("10".into()),
            gas_fee_query: Some("2".into()),
            gas_fee_create_token_base: Some("10000000000".into()),
            gas_fee_create_token_symbol: Some("10000000000".into()),
            gas_fee_create_token_series: Some("2500000000".into()),
            gas_fee_per_byte: Some("250000".into()),
            gas_fee_register_name: Some("10000000000000".into()),
            gas_burn_ratio_mul: Some("1".into()),
            gas_burn_ratio_shift: 0,
            minimum_gas_bill: Some("10000000".into()),
            gas_producer_ratio_mul: Some("0".into()),
            gas_producer_ratio_shift: Some(0),
            gas_dapp_ratio_mul: Some("0".into()),
            gas_dapp_ratio_shift: Some(0),
            policy_fee_create_token_base: Some("100000000000000".into()),
            policy_fee_create_token_symbol: Some("100000000000000".into()),
            policy_fee_create_token_series: Some("25000000000000".into()),
            // > 2^53: must survive exactly because it rides a string.
            policy_fee_register_name: Some("100000000000000000".into()),
            legacy_data_escrow_per_row: Some("2".into()),
        }),
    }
}

#[test]
fn v2_response_maps_to_gas_config() {
    let config = v2_result().to_gas_config().unwrap();

    assert!(config.has_gas_model_v2());
    assert_eq!(config.data_escrow_per_row, 200_000);
    assert_eq!(config.minimum_gas_bill, 10_000_000);
    assert_eq!(config.policy_fee_register_name, 100_000_000_000_000_000);
    assert_eq!(config.legacy_data_escrow_per_row, 2);
}

#[test]
fn v1_response_zeroes_absent_v2_fields() {
    let mut result = v2_result();
    // A version-0 config must ignore any tail strings: v1 semantics never read the tail.
    result.gas_config.as_mut().unwrap().version = 0;

    let config = result.to_gas_config().unwrap();

    assert!(!config.has_gas_model_v2());
    assert_eq!(config.minimum_gas_bill, 0);
    assert_eq!(config.policy_fee_register_name, 0);
}

#[test]
fn v2_response_with_missing_tail_field_fails() {
    // Estimating fees from silently zeroed v2 prices would produce rejected transactions.
    let mut result = v2_result();
    result.gas_config.as_mut().unwrap().minimum_gas_bill = None;

    let err = result.to_gas_config().unwrap_err();
    assert!(err.to_string().contains("minimumGasBill"));
}

#[test]
fn missing_gas_config_section_fails() {
    let result = GasConfigResult {
        gas_model_version: 1,
        ..GasConfigResult::default()
    };
    assert!(result.to_gas_config().is_err());
}
