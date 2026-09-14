//! The client-owned fee planner: one cached gas config per client, read again on request, on
//! invalidation and on expiry; burns planned for what the NFT holds.

mod common;

use std::time::Duration;

use common::{address_of, burn_tx, keys, transfer_tx, CannedNode};
use phantasma_sdk::{
    ChainFeeParams, FeePlanOptions, InfusedAsset, PhantasmaRpc, PlanRequestOptions,
};
use serde_json::json;

#[tokio::test]
async fn reads_the_gas_config_once_and_prices_messages_with_it() {
    let (owner_keys, payer_keys) = keys();
    let msg = transfer_tx(address_of(&owner_keys), address_of(&payer_keys), None, 0);
    let node = CannedNode::new();
    let client = node.client();

    let first = client
        .fees()
        .plan(&msg, &PlanRequestOptions::default())
        .await
        .unwrap();
    let second = client
        .fees()
        .plan(&msg, &PlanRequestOptions::default())
        .await
        .unwrap();
    assert_eq!(first.expected_gas_bill, 42_600_000);
    assert_eq!(second.max_gas, 42_600_000);
    assert_eq!(node.state().gas_config_reads, 1);

    let params = client.fees().chain_params(false).await.unwrap();
    assert_eq!(
        params,
        ChainFeeParams {
            expiry_window: Duration::from_secs(3600),
            block_rate_target: Duration::from_secs(2),
            gas_model_version: 2,
        }
    );
    assert_eq!(node.state().gas_config_reads, 1);
}

#[tokio::test]
async fn reads_the_config_again_when_asked_when_invalidated_and_when_the_cache_expires() {
    let (owner_keys, payer_keys) = keys();
    let msg = transfer_tx(address_of(&owner_keys), address_of(&payer_keys), None, 0);
    let node = CannedNode::new();
    let client = node.client().with_fee_config_ttl(Duration::from_millis(40));

    client.fees().config(false).await.unwrap();
    client.fees().config(false).await.unwrap();
    assert_eq!(node.state().gas_config_reads, 1);

    client
        .fees()
        .plan(
            &msg,
            &PlanRequestOptions {
                refresh_config: true,
                ..PlanRequestOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(node.state().gas_config_reads, 2);

    client.fees().invalidate();
    client.fees().config(false).await.unwrap();
    assert_eq!(node.state().gas_config_reads, 3);

    tokio::time::sleep(Duration::from_millis(80)).await;
    client.fees().config(false).await.unwrap();
    assert_eq!(node.state().gas_config_reads, 4);
}

#[tokio::test]
async fn plans_against_a_config_the_caller_holds_without_touching_the_node() {
    let (owner_keys, payer_keys) = keys();
    let msg = transfer_tx(address_of(&owner_keys), address_of(&payer_keys), None, 0);
    let node = CannedNode::new();
    let client = node.client();
    let config = client.fees().config(false).await.unwrap();
    let plan = client
        .fees()
        .plan_with(&config, &msg, &FeePlanOptions::default())
        .unwrap();
    assert_eq!(plan.expected_gas_bill, 42_600_000);
    assert_eq!(node.state().gas_config_reads, 1);
}

// A burn is planned for what the NFT holds: read from the node unless the caller stated it.
#[tokio::test]
async fn reads_what_a_burned_nft_holds_unless_the_caller_states_it() {
    let (owner_keys, _) = keys();
    let burn = burn_tx(address_of(&owner_keys), 7, 42);
    let node = CannedNode::new();
    let nft_address = phantasma_sdk::get_nft_address(7, 42).to_string();
    {
        let mut state = node.state();
        state
            .tokens
            .insert("KCAL".into(), json!({"symbol": "KCAL", "carbonId": "1"}));
        state
            .tokens
            .insert("GPX".into(), json!({"symbol": "GPX", "carbonId": "97"}));
        state.fungible.insert(
            nft_address,
            vec![
                json!({"chain": "main", "symbol": "KCAL", "amount": "1", "decimals": 10}),
                json!({"chain": "main", "symbol": "GPX", "amount": "5", "decimals": 8}),
            ],
        );
    }
    let client = node.client();
    let holdings = vec![
        InfusedAsset {
            token_id: Some(1),
            ..InfusedAsset::default()
        },
        InfusedAsset {
            token_id: Some(97),
            ..InfusedAsset::default()
        },
    ];

    let read = client
        .fees()
        .plan(&burn, &PlanRequestOptions::default())
        .await
        .unwrap();
    assert_eq!(node.state().infusion_reads, 1);
    let config = client.fees().config(false).await.unwrap();
    let stated = client
        .fees()
        .plan_with(
            &config,
            &burn,
            &FeePlanOptions {
                infusions: Some(holdings),
                ..FeePlanOptions::default()
            },
        )
        .unwrap();
    assert_eq!(stated.expected_gas_bill, read.expected_gas_bill);

    let empty = client
        .fees()
        .plan(
            &burn,
            &PlanRequestOptions {
                facts: FeePlanOptions {
                    infusions: Some(vec![]),
                    ..FeePlanOptions::default()
                },
                ..PlanRequestOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(node.state().infusion_reads, 1);
    assert_eq!(
        read.expected_gas_bill - empty.expected_gas_bill,
        40 * 10_000
    );
}

#[tokio::test]
async fn the_planner_is_owned_by_the_client_one_per_client() {
    let node = CannedNode::new();
    let client = node.client();
    client.fees().config(false).await.unwrap();
    // A clone shares the client's cache; a second client of the same node has its own.
    let copied = client.clone();
    copied.fees().config(false).await.unwrap();
    assert_eq!(node.state().gas_config_reads, 1);
    let other = PhantasmaRpc::with_transport("http://other.invalid/rpc", node.clone());
    other.fees().config(false).await.unwrap();
    assert_eq!(node.state().gas_config_reads, 2);
}

// A batch of burns is planned for what EVERY burned NFT holds. The planner reads one address per
// instance, because the fee follows each returned asset separately, and hands the union to the pure
// planner.
#[tokio::test]
async fn reads_one_address_per_burned_instance_of_a_batch() {
    let (owner_keys, _) = keys();
    let owner = address_of(&owner_keys);
    let node = CannedNode::new();
    {
        let mut state = node.state();
        state
            .tokens
            .insert("KCAL".into(), json!({"symbol": "KCAL", "carbonId": "1"}));
        for (token_id, instance_id) in [(9u64, 1u64), (9, 2), (5, 3)] {
            state.fungible.insert(
                phantasma_sdk::get_nft_address(token_id, instance_id).to_string(),
                vec![json!({"chain": "main", "symbol": "KCAL", "amount": "1", "decimals": 10})],
            );
        }
    }
    let burn_call = |token_id: u64, instance_ids: Vec<u64>| phantasma_sdk::TxMsgCall {
        module_id: phantasma_sdk::ModuleId::Token as u32,
        method_id: phantasma_sdk::TokenContractMethod::BurnNonFungible as u32,
        args: phantasma_sdk::serialize(&phantasma_sdk::BurnNonFungibleArgs {
            token_id,
            from_address: owner,
            instance_ids,
        })
        .unwrap(),
        sections: None,
    };
    let batch = phantasma_sdk::TxMsg {
        tx_type: phantasma_sdk::TxType::CallMulti,
        expiry: 1_759_711_416_000,
        max_gas: 0,
        max_data: 0,
        gas_from: owner,
        payload: phantasma_sdk::SmallString::default(),
        msg: phantasma_sdk::TxPayload::CallMulti(phantasma_sdk::TxMsgCallMulti {
            calls: vec![burn_call(9, vec![1, 2]), burn_call(5, vec![3])],
        }),
    };

    let client = node.client();
    let options = PlanRequestOptions {
        facts: FeePlanOptions {
            witness_count: Some(1),
            ..FeePlanOptions::default()
        },
        ..PlanRequestOptions::default()
    };
    let plan = client.fees().plan(&batch, &options).await.unwrap();
    assert_eq!(node.state().infusion_reads, 3);
    assert_eq!(
        plan.kinds,
        vec![
            phantasma_sdk::NativeFeeKind::BurnNonFungible,
            phantasma_sdk::NativeFeeKind::BurnNonFungible
        ]
    );

    // The three returned assets are priced once for the batch, on its first burn: one transfer plus
    // one owner-lookup query each, which is 20 work units apiece.
    let config = client.fees().config(false).await.unwrap();
    let empty = client
        .fees()
        .plan_with(
            &config,
            &batch,
            &FeePlanOptions {
                witness_count: Some(1),
                infusions: Some(vec![]),
                ..FeePlanOptions::default()
            },
        )
        .unwrap();
    assert_eq!(
        plan.expected_gas_bill - empty.expected_gas_bill,
        3 * 20 * 10_000
    );
}
