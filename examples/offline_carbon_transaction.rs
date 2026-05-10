use phantasma_sdk::{
    bytes32_from_public_key, sign_and_serialize_tx_msg_hex, PhantasmaKeys, Result, SmallString,
    TxMsg, TxMsgTransferFungible, TxPayload, TxType,
};

fn main() -> Result<()> {
    let keys = PhantasmaKeys::try_from_slice(&[7u8; 32])?;
    let signer = bytes32_from_public_key(&keys.public_key())?;
    let receiver =
        bytes32_from_public_key(&PhantasmaKeys::try_from_slice(&[9u8; 32])?.public_key())?;

    let msg = TxMsg {
        tx_type: TxType::TransferFungible,
        expiry: 1_759_711_416_000,
        max_gas: 10_000_000,
        max_data: 1_000,
        gas_from: signer,
        payload: SmallString::new("example")?,
        msg: TxPayload::TransferFungible(TxMsgTransferFungible {
            to: receiver,
            token_id: 1,
            amount: 1_000_000,
        }),
    };

    println!("{}", sign_and_serialize_tx_msg_hex(&msg, &keys)?);
    Ok(())
}
