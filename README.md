# Phantasma Rust SDK

This repository contains the Rust SDK for Phantasma Gen3 Carbon and the classic
VM transaction surface. The crate was rewritten as a native Rust library rather
than a compatibility wrapper around the previous SDK shape.

The public API is organized around checked primitives:

- `crypto`: Ed25519 keys, WIF, Phantasma addresses, hashes, signatures.
- `binary`: classic VM binary readers and writers.
- `vm`: script building and VM object parsing.
- `transaction`: classic signed transactions and proof-of-work helpers.
- `carbon`: Gen3 Carbon wire formats, token/NFT schemas, signed Carbon tx
  messages, and builder helpers.
- `rpc`: async JSON-RPC client, response models, state helpers, and send
  helpers for classic and Carbon transactions.

All fallible APIs return `phantasma_sdk::Result<T>`. Public parsing code rejects
malformed input with explicit errors instead of panicking.

## Installation

```toml
[dependencies]
phantasma-sdk = { path = "." }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Read-Only RPC

```rust
use phantasma_sdk::{PhantasmaRpc, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let rpc = PhantasmaRpc::new("http://localhost:5172/rpc");
    let version = rpc.get_version().await?;
    println!("{} {}", version.version, version.commit);
    Ok(())
}
```

## Offline Classic Transaction

```rust
use phantasma_sdk::{Address, PhantasmaKeys, Result, ScriptBuilder, Transaction, encode_hex};

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
```

## Offline Carbon Transaction

```rust
use phantasma_sdk::{
    bytes32_from_public_key, sign_and_serialize_tx_msg_hex, PhantasmaKeys, Result, SmallString,
    TxMsg, TxMsgTransferFungible, TxPayload, TxType,
};

fn main() -> Result<()> {
    let keys = PhantasmaKeys::try_from_slice(&[7u8; 32])?;
    let signer = bytes32_from_public_key(&keys.public_key())?;
    let receiver = bytes32_from_public_key(&PhantasmaKeys::try_from_slice(&[9u8; 32])?.public_key())?;

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
```

## Verification

Use `just verify` for the normal local gate:

```bash
just verify
```

That runs formatting checks, all-target compilation, unit tests, Clippy with
warnings denied, and docs with rustdoc warnings denied. `cargo package
--allow-dirty` is also useful before publishing or reviewing package contents.

The test suite includes cross-SDK vectors copied from the Python SDK rewrite,
including Carbon primitives, `IntX`, VM structs, Carbon transactions, classic
VM scripts, WIF/address/signature behavior, RPC request parsing, and hostile
input rejection paths.
