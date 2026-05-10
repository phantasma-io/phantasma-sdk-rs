# Rust SDK Feature Parity

This crate follows the Python SDK rewrite as the immediate reference and keeps
wire-format behavior aligned with the C#, TypeScript, C++, Go, and Python SDKs
where those SDKs expose the same surface.

## Implemented Surface

- Classic binary encoding: varuint, varbytes, strings, timestamps, BigInt VM
  encoding, bounded readers.
- Cryptography: Base58, hex, WIF, Ed25519 key derivation, Phantasma address text,
  signatures, SHA-256 hash difficulty.
- Classic VM: opcodes, VM object decoding, script builder labels, contract calls,
  gas helpers, token transfer helpers.
- Classic transactions: hash, sign, verify signer, serialize, deserialize,
  low-difficulty local proof-of-work.
- Carbon serialization: fixed bytes, zero-terminated strings, arrays, BigInt,
  `IntX`, generic and typed integer arrays, dynamic VM schemas/structs, token
  metadata, token info, series info, NFT ROM/RAM helpers.
- Carbon transactions: typed `TxMsg` payloads, `SignedTxMsg`, witnesses,
  deterministic signing, token creation, series creation, NFT minting helpers,
  parsed token-schema JSON shape (`TokenSchemasJson` / `TokenSchemasJSON`),
  schema JSON-to-wire builders, market/config call args, result parsers.
- JSON-RPC: async client, injectable transport for tests, read methods for common
  account/block/token/NFT/archive/contract/state calls, send helpers for classic
  and Carbon transactions, response DTOs with serde defaults and scalar
  coercion for reference RPC response quirks.

## Rust API Decisions

- Errors use one `PhantasmaError` enum and `Result<T>` alias.
- Public builders validate inputs before producing bytes.
- Readers reject truncated payloads, oversized arrays, unsupported tags, and
  trailing bytes where a complete object parser is expected.
- Data structures use Rust naming and strong types instead of mirroring legacy
  class names or nullable dynamic maps.
- Python names like `PhantasmaRPC`, `ModuleID`, and `ABIParameterResult` map to
  Rust names like `PhantasmaRpc`, `ModuleId`, and `AbiParameterResult`.
- Python exception subclasses map to variants of `PhantasmaError`.
- Async RPC is transport-generic so unit tests do not require a live node.
- Examples avoid funded or broadcasting workflows unless the caller explicitly
  chooses to run a send method.

## Test Sources

- `tests/fixtures/carbon_vectors.tsv` is copied from the Python SDK rewrite and
  covers shared Carbon vectors, including non-canonical read cases that must
  match the reference behavior.
- `tests/binary_transaction_vm.rs` covers classic wire formats and script output.
- `tests/encoding_crypto.rs` covers WIF/address/signature/hash behavior.
- `tests/carbon_builders.rs` covers higher-level Carbon builders and result
  parsers, including Python metadata validation behavior for required fields,
  case-sensitive schema names, ROM bytes, fixed bytes, unsigned-to-signed
  integer coercion, and array-of-struct metadata.
- `tests/carbon_python_parity.rs` covers Python rewrite parity for Carbon token,
  market, config, parsed schema JSON plus schema builders, and deterministic
  Phantasma NFT helper paths.
- `tests/rpc.rs` covers JSON-RPC request/response behavior through a mock
  transport.

Live localnet execution and funded/broadcasting examples are intentionally not
part of the default test suite. The read-only RPC example can be run against an
existing endpoint; offline examples never broadcast.
