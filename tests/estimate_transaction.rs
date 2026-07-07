//! estimateTransaction response decoding + Tier-2 fee-estimate conversion.
//!
//! The node serializes 64-bit amounts as decimal strings (JSON-number precision). A completed
//! estimate must convert into the same NativeFeeEstimate the Tier-1 estimator produces, so wallet
//! code consumes both tiers identically. The same fixtures exist in every SDK (parity suite).

use phantasma_sdk::rpc::EstimateTransactionResult;

// Completed dry run: recommendations present, no abort. recommendedMaxGas deliberately exceeds
// 2^53 to pin the strings-not-numbers decision.
const COMPLETED_JSON: &str = r#"{
  "wouldAbort": false,
  "abortReason": "",
  "gasBillKcalBase": "10000000",
  "dataRows": "1",
  "dataEscrowAtoms": "200000",
  "dataRefundAtoms": "0",
  "recommendedMaxGas": "100000000000000000",
  "recommendedMaxData": "400000"
}"#;

// Aborted dry run: the settled abort bill is still reported (aborts pay), recommendations are 0.
const ABORTED_JSON: &str = r#"{
  "wouldAbort": true,
  "abortReason": "gas fees [gas=3125 max=40]",
  "gasBillKcalBase": "40",
  "dataRows": "0",
  "dataEscrowAtoms": "0",
  "dataRefundAtoms": "0",
  "recommendedMaxGas": "0",
  "recommendedMaxData": "0"
}"#;

#[test]
fn completed_estimate_converts() {
    let result: EstimateTransactionResult = serde_json::from_str(COMPLETED_JSON).unwrap();
    assert!(!result.would_abort);
    let estimate = result.to_fee_estimate().unwrap();
    // Above-2^53 value survives exactly because it rides a string.
    assert_eq!(estimate.max_gas, 100_000_000_000_000_000);
    assert_eq!(estimate.max_data, 400_000);
    assert_eq!(estimate.expected_gas_bill, 10_000_000);
}

// An aborted simulation has no recommendations; converting must fail rather than yield zero
// ceilings a wallet could sign with.
#[test]
fn aborted_estimate_refuses_conversion() {
    let result: EstimateTransactionResult = serde_json::from_str(ABORTED_JSON).unwrap();
    assert!(result.would_abort);
    let err = result.to_fee_estimate().unwrap_err();
    assert!(err.to_string().contains("gas fees"));
}

// A malformed server response (lost field) must not silently become a zero ceiling.
#[test]
fn missing_field_fails() {
    let result: EstimateTransactionResult =
        serde_json::from_str(r#"{"wouldAbort": false}"#).unwrap();
    assert!(result.to_fee_estimate().is_err());
}
