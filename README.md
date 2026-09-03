# Phantasma Rust SDK

Rust SDK for the Phantasma blockchain with support for the Phoenix chain update.

The crate provides transaction building and signing, VM script helpers, Ed25519
keys/signatures, JSON-RPC access, and Carbon wire-format support.

The public API is organized around checked primitives:

- `crypto`: Ed25519 keys, WIF, Phantasma addresses, hashes, signatures.
- `binary`: VM binary readers and writers.
- `vm`: script building and VM object parsing.
- `transaction`: VM script transactions and proof-of-work helpers.
- `carbon`: Carbon wire formats, token/NFT schemas, signed Carbon transaction
  messages, and builder helpers for native transfers, mints, burns and token calls.
- `fees`: the gas-model-v2 fee model - prices a Carbon message from the message
  and the chain's gas config, exactly for every operation the chain prices by formula.
- `rpc`: async JSON-RPC client, response models, state helpers, a fee planner
  per client, and send helpers for VM script and Carbon transactions.

All fallible APIs return `phantasma_sdk::Result<T>`. Public parsing code rejects
malformed input with explicit errors instead of panicking.

## Installation

```toml
[dependencies]
phantasma-sdk = "2"
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

Indexers that need both typed models and archival/parity payloads can use the
`*_with_raw` helpers:

```rust
let response = rpc.get_block_by_height_with_raw("main", 123).await?;
println!("{} {}", response.value.hash, response.raw_result);
```

## Offline VM Script Transaction

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

## Fee planning: build, plan, sign, send

Under gas model v2 every byte a transaction puts in the block is billed and every new storage row is
escrowed, so the fee of a native operation is a function of the message and the chain's prices - and
the SDK computes it from the message. Builders carry no prices: a message built without a `max_gas`
has a zero offer, which marks it as unplanned and refuses to sign. The steps are:

1. **Build** the message with a builder (`build_transfer_fungible_tx`, `build_create_token_tx`,
   `build_mint_phantasma_non_fungible_tx`, ...). The builders write only the limits you pass
   (`TxLimits`).
2. **Plan** it against the chain: `client.fees().plan(&msg, &PlanRequestOptions::default())` reads
   the chain's gas config through the client (cached for a minute), recognises the operation and
   prices it. For every operation the SDK models the bill is exact for the facts it was given; a
   chain-state fact the message does not carry - whether the recipient already holds the token,
   whether the series is duplicated - defaults to the reading that costs more, so an unstated plan is
   an upper bound the settlement can only undercut, and the unused part of the offer is refunded.
   State what you know in `PlanRequestOptions` to get the exact quote. A burn of an NFT is planned
   for what the NFT holds: the client reads the NFT's address for you; the pure `plan_fees` demands
   the list instead.
3. **Sign** with every witness the message needs: `sign_tx_msg_with_keys(&planned, &[&keys])` for
   in-memory keys, `sign_tx_msg_with(&planned, &signers).await` for a `TxSigner` such as a hardware
   wallet. A gas-payer transfer takes the payer's and the owner's keys; the SDK puts them in the
   order the chain reads.
4. **Send** the envelope with `client.send_carbon_transaction(&bytes).await`.

`client.send_tx_msg(&msg, &signers, &SendTransactionOptions::default()).await` does all four in one
step, plus a pre-flight: a token creation pays its policy fee before the chain looks at the symbol,
so the client asks whether the symbol is taken and refuses to send unless the chain answered that it
is free. `client.preflight_transaction(&msg)` reports that verdict to callers who want to decide for
themselves.

```rust
use phantasma_sdk::{
    build_transfer_fungible_tx, bytes32_from_public_key, summarize_fee_plan, PhantasmaKeys,
    PhantasmaRpc, PlanRequestOptions, Result, SendTransactionOptions, TransferFungibleParams,
    TxSigner,
};

async fn send_kcal(client: &PhantasmaRpc, keys: &PhantasmaKeys, receiver: [u8; 32]) -> Result<String> {
    let msg = build_transfer_fungible_tx(TransferFungibleParams {
        from: bytes32_from_public_key(&keys.public_key())?,
        to: receiver.into(),
        token_id: 1, // KCAL
        amount: 100_000_000,
        ..TransferFungibleParams::default()
    });

    let plan = client.fees().plan(&msg, &PlanRequestOptions::default()).await?;
    let summary = summarize_fee_plan(&plan);
    println!("gas {} KCAL, storage deposit up to {} SOUL", summary.gas_bill, summary.storage_ceiling);

    // The wallet shows the summary and asks; then:
    let signers: [&dyn TxSigner; 1] = [keys];
    client.send_tx_msg(&msg, &signers, &SendTransactionOptions::default()).await
}
```

Notes:

- `summarize_fee_plan` renders a plan in KCAL and SOUL; `plan.apply(&msg)` returns the message with
  the plan written in when you sign yourself.
- A message keeps a default lifetime of 45 seconds. When a person sits between building and signing,
  set `TxLimits::expiry` from the chain's own window: `expiry_within(params.expiry_window, Duration::ZERO)`
  with `let params = client.fees().chain_params(false).await?`.
- Calls whose witness set the caller chooses (token creation, series creation, Phantasma mints, name
  registration) need `witness_count` in the plan options; `send_tx_msg` and the `build_*_tx_and_sign`
  helpers fill it in from the signers they are given.
- Scripts and calls the SDK does not model are planned as a budget (`NativeFeeKind::Script`), not a
  formula; the node's `estimate_transaction` gives their exact bill.
- A node that refuses a request with an HTTP error and a JSON-RPC body surfaces the body as
  `PhantasmaError::Rpc` with the node's code and message, so a match on the error tells the node's
  refusal from a transport failure.

`examples/plan_carbon_transfer_fee.rs` plans a one-atom KCAL transfer against a node and prints the
summary without signing or sending anything.

## Offline Carbon Transaction

Without a chain to plan against, state the offer yourself; a 170-byte KCAL transfer bills
0.00426 KCAL on mainnet, and the unused part of the offer is refunded.

```rust
use phantasma_sdk::{
    build_transfer_fungible_tx, bytes32_from_public_key, sign_and_serialize_tx_msg_hex,
    PhantasmaKeys, Result, TransferFungibleParams, TxLimits,
};

fn main() -> Result<()> {
    let keys = PhantasmaKeys::try_from_slice(&[7u8; 32])?;
    let receiver = bytes32_from_public_key(&PhantasmaKeys::try_from_slice(&[9u8; 32])?.public_key())?;

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
```

## Verification

Use `just verify` for the normal local gate:

```bash
just verify
```

That runs formatting checks, all-target compilation, unit tests, Clippy with
warnings denied, and docs with rustdoc warnings denied. Use `just release-check`
before publishing; it also verifies strict package creation and runs
`cargo publish --dry-run`.

The test suite includes cross-SDK vectors copied from the Python SDK fixture set,
including Carbon primitives, `IntX`, VM structs, Carbon transactions, VM
scripts, WIF/address/signature behavior, RPC request parsing, and hostile
input rejection paths.
