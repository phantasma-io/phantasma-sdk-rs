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

Under gas model v2 the chain bills every byte a transaction puts in the block, and it escrows every
new storage row. The fee of a native operation is therefore a function of the message and the chain's
prices, and the SDK computes it from the message. Builders carry no prices. A message built without a
`max_gas` has a zero offer, which marks it as unplanned and refuses to sign. The steps are:

1. **Build** the message with a builder (`build_transfer_fungible_tx`, `build_create_token_tx`,
   `build_mint_phantasma_non_fungible_tx`, ...). The builders write only the limits you pass
   (`TxLimits`).
2. **Plan** it against the chain: `client.fees().plan(&msg, &PlanRequestOptions::default())` reads
   the chain's gas config through the client (cached for a minute), recognises the operation and
   prices it. For every operation the SDK models, the bill is exact for the facts it was given. Some
   of the price depends on chain state the message does not carry. Examples are whether the recipient
   already holds the token, and which mode a series mints in. Each such fact is taken at the value
   that costs MORE, so an unstated plan is an upper bound the settlement can only undercut, and the
   unused part of the offer is refunded. State what you know in `PlanRequestOptions` to get the exact
   quote. A burn of an NFT is planned for what the NFT holds: the client reads the NFT's address for
   you, and the pure `plan_fees` demands the list.
3. **Sign** with every witness the message needs: `sign_tx_msg_with_keys(&planned, &[&keys])` for
   in-memory keys, `sign_tx_msg_with(&planned, &signers).await` for a `TxSigner` such as a hardware
   wallet. A gas-payer transfer takes the payer's and the owner's keys; the SDK puts them in the
   order the chain reads.
4. **Send** the envelope with `client.send_carbon_transaction(&bytes).await`.

`client.send_tx_msg(&msg, &signers, &SendTransactionOptions::default()).await` does all four in one
step, plus a pre-flight. A token creation pays its policy fee before the chain looks at the symbol,
so the client asks whether the symbol is taken and refuses to send unless the chain answered that it
is free. `client.preflight_transaction(&msg)` reports that verdict to callers that want to decide for
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

### Which fact each operation reads

Only the facts an operation reads can move its price, so this table is the whole of what is worth
stating. Every default is the reading that costs MORE. One storage quantum is `data_escrow_per_row`
of escrow plus 25 gas units of block data, and the chain's `fee_multiplier` scales both. Balance rows
of the gas and data tokens are free, so for those tokens `recipient_holds_token` changes nothing.

| `NativeFeeKind` | facts it reads | what the default assumes | what the default costs |
|---|---|---|---|
| `TransferFungible` | `recipient_holds_token` | the recipient has no row for this token | 1 quantum |
| `TransferNonFungible` | `recipient_holds_token` | the recipient has no row for this token | 1 quantum |
| `MintFungible` | `recipient_holds_token`, `supply_row_exists`, `big_fungible` | no recipient row; the supply row was dropped and must be recreated; the resulting balance needs the widest answer | 1 quantum each, and 24 more result bytes (33 against 9) |
| `BurnFungible` | `token_burned_before`, `supply_row_exists`, `big_fungible` | the token's burnt counter does not exist yet; the supply row must be recreated; widest answer | 1 quantum each, and 24 more result bytes |
| `MintNonFungible` | `recipient_holds_token`, `supply_row_exists`, `rom_has_meta_id` | as above, plus: the ROM carries an `_i` id, which is indexed in one more row | 1 quantum each, and 1 quantum per instance for the id |
| `MintPhantasmaNonFungible` | `recipient_holds_token`, `supply_row_exists`, `duplicated_series` | as above, plus: the series mints duplicates | 1 quantum each, and one query fee per instance plus one per distinct series |
| `BurnNonFungible` | `token_burned_before`, `supply_row_exists`, **`infusions` (required)** | burnt counter does not exist; supply row must be recreated | 1 quantum each. `infusions` has no default at all. A burn returns whatever the NFT holds, and there is no upper bound on that, so the plan demands the list and `client.fees()` reads it from the chain. `rom_has_meta_id` is read here too but cannot move the bill: a burn deletes more rows than it creates, so its net storage growth is zero either way |
| `CreateToken` | none | nothing is assumed | the price comes entirely from the message: the symbol length, the serialized `TokenInfo`, and which keys its metadata carries |
| `CreateTokenSeries` | `series_has_meta_id` | the series metadata carries an `_i` id | 1 quantum |
| `RegisterName` | none | nothing is assumed | governance rows are free data; the price is the length-shifted policy fee and the envelope |
| `Script` | none | 5000 work units, 512 event bytes and 4 storage quanta, per unmodelled call | a budget and never a prediction. `plan.exact` is `false` whenever one is present |

Notes:

- `plan.kinds` says which operations were priced. A `CallMulti` performs several, and each one is
  priced and then summed, because the chain bills a batch as the sum of its calls with the envelope
  counted once. One kind is a budget and not a formula: `NativeFeeKind::Script`, which covers VM
  scripts and calls the SDK does not model. Their work depends on execution.
- `plan.exact` says whether the number is a prediction or a ceiling. It is `true` when nothing the
  plan had to assume could have changed it. It is `false` when a costlier reading decided part of the
  price, or when a part of the message had to be budgeted. A wallet shows the amount when the flag is
  `true`, and "up to" in front of the amount when it is `false`. The flag is answered by pricing the
  message a second time with every fact at its cheaper reading, so it is about THIS message and not
  about which options you set. A KCAL transfer is exact with nothing stated, because the chain's own
  token rows are free and `recipient_holds_token` cannot move its price.
- `burned_instances(&msg)` names the NFT instances a message burns, in any shape. It covers the
  native burn types, a `Token.BurnNonFungible` call, and every such call inside a `CallMulti`. Use it
  to tell whether `infusions` is required. An empty answer means the list is not required.
- `summarize_fee_plan` renders a plan in KCAL and SOUL. `plan.apply(&msg)` returns the message with
  the plan written in when you sign yourself.
- A message keeps a default lifetime of 45 seconds. When a person sits between building and signing,
  set `TxLimits::expiry` from the chain's own window with
  `expiry_within(params.expiry_window, Duration::ZERO)`, where
  `let params = client.fees().chain_params(false).await?`.
- Calls whose witness set the caller chooses (token creation, series creation, Phantasma mints, name
  registration, and every batch) need `witness_count` in the plan options. `send_tx_msg` and the
  `build_*_tx_and_sign` helpers fill it in from the signers they are given.
- The node's `estimate_transaction` gives the exact bill of a script, which the SDK can only budget.
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
