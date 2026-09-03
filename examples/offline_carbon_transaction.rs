//! Signs a KCAL transfer without a chain to plan against: the offer is stated. A 170-byte KCAL
//! transfer bills 0.00426 KCAL on mainnet, and the unused part of the offer is refunded.

use phantasma_sdk::{
    build_transfer_fungible_tx, bytes32_from_public_key, sign_and_serialize_tx_msg_hex,
    PhantasmaKeys, Result, TransferFungibleParams, TxLimits,
};

fn main() -> Result<()> {
    let keys = PhantasmaKeys::try_from_slice(&[7u8; 32])?;
    let receiver =
        bytes32_from_public_key(&PhantasmaKeys::try_from_slice(&[9u8; 32])?.public_key())?;

    let msg = build_transfer_fungible_tx(TransferFungibleParams {
        limits: TxLimits {
            max_gas: 50_000_000, // 0.005 KCAL
            max_data: 0,
            expiry: 1_759_711_416_000,
        },
        from: bytes32_from_public_key(&keys.public_key())?,
        to: receiver,
        token_id: 1,
        amount: 1_000_000,
        ..TransferFungibleParams::default()
    });

    println!("{}", sign_and_serialize_tx_msg_hex(&msg, &keys)?);
    Ok(())
}
