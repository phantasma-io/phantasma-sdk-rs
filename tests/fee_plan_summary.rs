//! A fee plan rendered in the units a person reads.

use phantasma_sdk::{
    summarize_fee_plan, summarize_fee_plan_with_decimals, FeePlan, FeePlanSummary, NativeFeeKind,
};

fn plan() -> FeePlan {
    FeePlan {
        kinds: vec![NativeFeeKind::TransferFungible],
        exact: true,
        envelope_bytes: 170,
        max_gas: 42_850_000,
        max_data: 200_000,
        expected_gas_bill: 42_850_000,
        new_storage_quanta: 1,
        deleted_storage_quanta: 0,
    }
}

#[test]
fn renders_the_plan_in_kcal_and_soul() {
    assert_eq!(
        summarize_fee_plan(&plan()),
        FeePlanSummary {
            gas_bill: "0.004285".into(),
            gas_offer: "0.004285".into(),
            storage_ceiling: "0.002".into(),
        }
    );
}

#[test]
fn takes_the_decimals_of_a_chain_whose_tokens_differ() {
    assert_eq!(
        summarize_fee_plan_with_decimals(&plan(), 8, 10),
        FeePlanSummary {
            gas_bill: "0.4285".into(),
            gas_offer: "0.4285".into(),
            storage_ceiling: "0.00002".into(),
        }
    );
}
