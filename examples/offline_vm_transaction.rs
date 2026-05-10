use phantasma_sdk::{encode_hex, Address, PhantasmaKeys, Result, ScriptBuilder, Transaction};

fn main() -> Result<()> {
    let keys = PhantasmaKeys::try_from_slice(&[7u8; 32])?;
    let from = keys.address();
    let to = Address::from_hash(b"example receiver");

    let script = ScriptBuilder::begin()
        .allow_gas(from, Address::null(), 100_000, 21_000)
        .transfer_tokens("SOUL", from, to, 1)
        .spend_gas(from)
        .end_script()?;

    let mut tx = Transaction::new("mainnet", "main", script, 0).with_payload(b"example".to_vec());
    tx.sign(&keys);

    println!("{}", encode_hex(tx.to_bytes(true)));
    Ok(())
}
