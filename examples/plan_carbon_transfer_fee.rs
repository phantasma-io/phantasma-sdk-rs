//! Builds a one-atom transfer of the chain's gas token to the given address and prices it from
//! the message, without signing or sending anything: the plan is what a wallet shows before asking
//! for confirmation. Read-only - it costs nothing to run.
//!
//! Usage: plan_carbon_transfer_fee [RPC_URL] [RECIPIENT_ADDRESS]

use phantasma_sdk::{
    build_transfer_fungible_tx, bytes32_from_phantasma_address_text, bytes32_from_public_key,
    summarize_fee_plan, PhantasmaKeys, PhantasmaRpc, PlanRequestOptions, Result,
    TransferFungibleParams,
};

#[tokio::main]
async fn main() -> Result<()> {
    let endpoint = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("PHANTASMA_RPC_URL").ok())
        .unwrap_or_else(|| "http://localhost:5172/rpc".to_string());
    // The sender needs no funds to be planned for: only its address goes into the message.
    let keys = PhantasmaKeys::try_from_slice(&[7u8; 32])?;
    let sender = bytes32_from_public_key(&keys.public_key())?;
    let recipient = match std::env::args().nth(2) {
        Some(text) => bytes32_from_phantasma_address_text(&text)?,
        None => sender,
    };

    let client = PhantasmaRpc::new(endpoint);
    let config = client.fees().config(false).await?;
    let msg = build_transfer_fungible_tx(TransferFungibleParams {
        from: sender,
        to: recipient,
        token_id: config.gas_token_id,
        amount: 1,
        ..TransferFungibleParams::default()
    });
    // Every fact this plan needs is in the message: a gas-token transfer escrows no rows, so the
    // options stay empty. A plan of an ordinary token would state recipient_holds_token when known.
    let plan = client
        .fees()
        .plan(&msg, &PlanRequestOptions::default())
        .await?;
    let summary = summarize_fee_plan(&plan);
    let operations: Vec<String> = plan.kinds.iter().map(|kind| format!("{kind:?}")).collect();
    println!(
        "Operations: {}, signed size {} bytes",
        operations.join(", "),
        plan.envelope_bytes
    );
    println!(
        "Gas bill: {} KCAL (offer {} KCAL)",
        summary.gas_bill, summary.gas_offer
    );
    println!(
        "Storage deposit ceiling: {} SOUL ({} new rows)",
        summary.storage_ceiling, plan.new_storage_quanta
    );
    println!("Nothing was signed or sent.");
    Ok(())
}
