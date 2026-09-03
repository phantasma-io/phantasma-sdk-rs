//! The one-step send: pre-flight, fee plan, signatures, broadcast - against a canned node.

mod common;

use common::{
    address_of, burn_tx, create_token_tx, keys, register_name_tx, transfer_tx, CannedNode,
};
use phantasma_sdk::{
    get_nft_address, plan_fees, FeePlanOptions, InfusedAsset, PhantasmaError, PreflightResult,
    PreflightVerdict, SendTransactionOptions, TxSigner,
};
use serde_json::json;

#[tokio::test]
async fn plans_an_unplanned_message_signs_it_and_broadcasts_the_envelope() {
    let (owner_keys, payer_keys) = keys();
    let node = CannedNode::new();
    let client = node.client();
    let msg = transfer_tx(address_of(&owner_keys), address_of(&payer_keys), None, 0);

    let hash = client
        .send_tx_msg(&msg, &[&owner_keys], &SendTransactionOptions::default())
        .await
        .unwrap();
    assert_eq!(hash, "HASH");
    let sent = node.decode_sent();
    assert_eq!(sent.msg.max_gas, 42_600_000);
    assert_eq!(sent.witnesses.len(), 1);
    assert_eq!(msg.max_gas, 0, "the input must be left unplanned");
}

#[tokio::test]
async fn sends_a_message_the_caller_already_planned_as_it_is() {
    let (owner_keys, payer_keys) = keys();
    let node = CannedNode::new();
    let msg = transfer_tx(
        address_of(&owner_keys),
        address_of(&payer_keys),
        None,
        55_000_000,
    );
    node.client()
        .send_tx_msg(&msg, &[&owner_keys], &SendTransactionOptions::default())
        .await
        .unwrap();
    assert_eq!(node.decode_sent().msg.max_gas, 55_000_000);
}

#[tokio::test]
async fn collects_every_witness_of_a_gas_payer_transfer() {
    let (owner_keys, payer_keys) = keys();
    let (owner, payer) = (address_of(&owner_keys), address_of(&payer_keys));
    let node = CannedNode::new();
    let signers: [&dyn TxSigner; 2] = [&owner_keys, &payer_keys];
    node.client()
        .send_tx_msg(
            &transfer_tx(owner, payer, Some(payer), 0),
            &signers,
            &SendTransactionOptions::default(),
        )
        .await
        .unwrap();
    let sent = node.decode_sent();
    assert_eq!(sent.witnesses[0].address, payer);
    assert_eq!(sent.witnesses[1].address, owner);
    assert_eq!(node.sent()[0].len(), (106 + 32 + 128) * 2);
}

// CreateToken consumes its policy fee before checking the symbol, so a taken symbol is refused
// here, before anything is signed or sent.
#[tokio::test]
async fn refuses_to_create_a_token_whose_symbol_is_taken() {
    let (owner_keys, _) = keys();
    let owner = address_of(&owner_keys);
    let node = CannedNode::new();
    node.state()
        .tokens
        .insert("TAKEN".into(), json!({"symbol": "TAKEN", "carbonId": "5"}));
    let client = node.client();

    let err = client
        .send_tx_msg(
            &create_token_tx(owner, "TAKEN"),
            &[&owner_keys],
            &SendTransactionOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, PhantasmaError::Preflight(_)), "{err}");
    assert!(node.sent().is_empty());

    client
        .send_tx_msg(
            &create_token_tx(owner, "FRESH"),
            &[&owner_keys],
            &SendTransactionOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(node.sent().len(), 1);
}

// The pre-flight covers token creation and nothing else. A name registration is sent without a
// lookup: the node reports a free name as an error and a taken one as an address, so there is no
// answer that means "free" to check against.
#[tokio::test]
async fn sends_a_name_registration_without_looking_anything_up() {
    let (owner_keys, _) = keys();
    let node = CannedNode::new();
    node.client()
        .send_tx_msg(
            &register_name_tx(address_of(&owner_keys), "alice"),
            &[&owner_keys],
            &SendTransactionOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(node.sent().len(), 1);
    assert_eq!(node.state().lookups, 0);
}

// A free symbol is established, not inferred from the error text: the node refuses to answer about
// FRESH, so the check asks it for a token that certainly exists. That answer proves the lookup
// works and is being truthful, which is what makes the refusal about FRESH mean "absent". Whatever
// the error says is irrelevant - including "Method not found", the JSON-RPC name of error -32601,
// which contains the words "not found" and means the question was never asked.
#[tokio::test]
async fn establishes_a_free_symbol_from_a_control_lookup_not_from_the_error_text() {
    let (owner_keys, _) = keys();
    let owner = address_of(&owner_keys);
    let node = CannedNode::new();
    let client = node.client();
    for text in [
        "Method not found",
        "backend unavailable",
        "Token symbol not found",
    ] {
        node.state().lookup_error = text.into();
        client
            .send_tx_msg(
                &create_token_tx(owner, "FRESH"),
                &[&owner_keys],
                &SendTransactionOptions::default(),
            )
            .await
            .unwrap_or_else(|err| panic!("{text}: {err}"));
    }
    assert_eq!(node.sent().len(), 3);
}

// And the case the whole check exists for: a node that cannot answer at all. Nothing is
// established, so nothing is signed - the policy fee is not spent on a guess.
#[tokio::test]
async fn refuses_when_the_lookup_cannot_answer_even_about_a_token_that_exists() {
    let (owner_keys, _) = keys();
    let owner = address_of(&owner_keys);
    let node = CannedNode::new();
    node.state().reachable = false;
    let client = node.client();
    for text in [
        "Method not found",
        "backend unavailable",
        "Token symbol not found",
    ] {
        node.state().lookup_error = text.into();
        let err = client
            .send_tx_msg(
                &create_token_tx(owner, "FRESH"),
                &[&owner_keys],
                &SendTransactionOptions::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, PhantasmaError::Preflight(_)), "{err}");
        let message = err.to_string();
        assert!(
            message.contains("could not establish whether token symbol FRESH is taken"),
            "{message}"
        );
        assert!(message.contains(text), "{message}");
    }
    assert!(node.sent().is_empty());
}

// send_tx_msg acts on one verdict only. A caller who wants to stop on the others reads the verdict
// itself and sends separately - which is the whole reason the check reports one instead of
// deciding. This is that path, and it is the one a wallet uses to warn before spending the fee.
#[tokio::test]
async fn preflight_hands_the_verdict_to_a_caller_who_wants_to_decide_for_itself() {
    let (owner_keys, _) = keys();
    let owner = address_of(&owner_keys);
    let node = CannedNode::new();
    node.state().lookup_error = "Method not found".into();
    let client = node.client();

    let check = client
        .preflight_transaction(&create_token_tx(owner, "FRESH"))
        .await
        .unwrap();
    assert_eq!(
        check,
        PreflightResult {
            verdict: PreflightVerdict::Free,
            subject: "token symbol FRESH".into(),
            reason: String::new(),
        }
    );

    // A client that cannot offer a control token - its gas config is unreadable - has nothing to
    // check the refusal against, so it says so rather than picking a side.
    let blind = CannedNode::new();
    blind.state().lookup_error = "Method not found".into();
    blind.state().gas_config_error = Some("backend unavailable".into());
    let check = blind
        .client()
        .preflight_transaction(&create_token_tx(owner, "FRESH"))
        .await
        .unwrap();
    assert_eq!(
        check,
        PreflightResult {
            verdict: PreflightVerdict::Unknown,
            subject: "token symbol FRESH".into(),
            reason: "Method not found".into(),
        }
    );

    let check = client
        .preflight_transaction(&register_name_tx(owner, "alice"))
        .await
        .unwrap();
    assert_eq!(check.verdict, PreflightVerdict::NotApplicable);

    client
        .send_tx_msg(
            &create_token_tx(owner, "FRESH"),
            &[&owner_keys],
            &SendTransactionOptions {
                skip_preflight: true,
                ..SendTransactionOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(node.sent().len(), 1);
}

// A gas-payer envelope always carries two signatures, even when one account pays for its own
// transfer: the number of witness slots comes from the message, never from the signer list.
#[tokio::test]
async fn sends_a_gas_payer_transfer_whose_payer_and_owner_are_the_same_account() {
    let (owner_keys, payer_keys) = keys();
    let (owner, payer) = (address_of(&owner_keys), address_of(&payer_keys));
    let node = CannedNode::new();
    node.client()
        .send_tx_msg(
            &transfer_tx(owner, payer, Some(owner), 0),
            &[&owner_keys],
            &SendTransactionOptions::default(),
        )
        .await
        .unwrap();
    let sent = node.decode_sent();
    assert_eq!(sent.witnesses.len(), 2);
    assert_eq!(sent.witnesses[0].signature, sent.witnesses[1].signature);
    assert_eq!(node.sent()[0].len(), (106 + 32 + 128) * 2);
    assert_eq!(sent.msg.max_gas, 66_600_000);
}

// A burn is sent for what the NFT holds: the one-step path reads the NFT address through the
// account queries and prices every returned asset, so the burn is not short by them.
#[tokio::test]
async fn reads_what_a_burned_nft_holds_and_prices_its_return() {
    let (owner_keys, _) = keys();
    let owner = address_of(&owner_keys);
    let node = CannedNode::new();
    let kcal = json!({"symbol": "KCAL", "carbonId": "1"});
    let gpx = json!({"symbol": "GPX", "carbonId": "97"});
    let art = json!({"symbol": "ART", "carbonId": "9"});
    let nft_address = get_nft_address(9, 5).to_string();
    {
        let mut state = node.state();
        for token in [&kcal, &gpx, &art] {
            state
                .tokens
                .insert(token["symbol"].as_str().unwrap().into(), token.clone());
        }
        state.fungible.insert(
            nft_address.clone(),
            vec![
                json!({"chain": "main", "symbol": "KCAL", "amount": "1", "decimals": 10}),
                json!({"chain": "main", "symbol": "GPX", "amount": "5", "decimals": 8}),
            ],
        );
        state
            .owned_nfts
            .insert(nft_address.clone(), vec![(art.clone(), "2".into())]);
    }
    let client = node.client();

    let burn = burn_tx(owner, 9, 5);
    client
        .send_tx_msg(&burn, &[&owner_keys], &SendTransactionOptions::default())
        .await
        .unwrap();
    let sent = node.decode_sent();

    let config = client.fees().config(false).await.unwrap();
    let holdings = vec![
        InfusedAsset {
            token_id: Some(1),
            ..InfusedAsset::default()
        },
        InfusedAsset {
            token_id: Some(97),
            ..InfusedAsset::default()
        },
        InfusedAsset {
            token_id: Some(9),
            non_fungible: true,
            instance_count: Some(2),
            burner_holds_token: false,
        },
    ];
    let expected = plan_fees(
        &burn,
        &config,
        &FeePlanOptions {
            infusions: Some(holdings.clone()),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(sent.msg.max_gas, expected.max_gas);
    assert_eq!(sent.msg.max_data, expected.max_data);
    // KCAL and GPX: a transfer and a query each; ART: a query, two transfers, a query - 80 units.
    let empty = plan_fees(
        &burn,
        &config,
        &FeePlanOptions {
            infusions: Some(vec![]),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(sent.msg.max_gas - empty.max_gas, 800_000);

    // The reader itself, as a caller of the planner would use it.
    assert_eq!(client.infused_assets(9, 5).await.unwrap(), holdings);

    // An NFT the node knows nothing about holds nothing: the plan is the plain burn.
    node.state().sent.clear();
    client
        .send_tx_msg(
            &burn_tx(owner, 9, 6),
            &[&owner_keys],
            &SendTransactionOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(node.decode_sent().msg.max_gas, empty.max_gas);
}

#[tokio::test]
async fn surfaces_a_broadcast_rejection_as_an_error() {
    let (owner_keys, payer_keys) = keys();
    let node = CannedNode::new();
    node.state().send_error = Some("mempool full".into());
    let err = node
        .client()
        .send_tx_msg(
            &transfer_tx(address_of(&owner_keys), address_of(&payer_keys), None, 0),
            &[&owner_keys],
            &SendTransactionOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, PhantasmaError::Rpc { message, .. } if message.contains("mempool full")),
        "{err}"
    );
}
