# Rust SDK Feature Parity

This crate keeps public behavior and wire formats aligned with the Python, C#,
TypeScript, C++, and Go SDKs where those SDKs expose the same surface.

## Implemented Surface

- VM binary encoding: varuint, varbytes, strings, timestamps, BigInt VM
  encoding, bounded readers.
- Cryptography: Base58, hex, WIF, Ed25519 key derivation, Phantasma address text,
  signatures, SHA-256 hash difficulty.
- VM: opcodes, VM object decoding, script builder labels, contract calls,
  gas helpers, token transfer helpers.
- VM script transactions: hash, sign, verify signer, serialize, deserialize,
  low-difficulty local proof-of-work.
- Carbon serialization: fixed bytes, zero-terminated strings, arrays, BigInt,
  `IntX`, generic and typed integer arrays, dynamic VM schemas/structs, token
  metadata, token info, series info, NFT ROM/RAM helpers.
- Carbon transactions: typed `TxMsg` payloads, `SignedTxMsg`, witnesses,
  deterministic signing, token creation, series creation, NFT minting helpers,
  parsed token-schema JSON shape (`TokenSchemasJson` / `TokenSchemasJSON`),
  schema JSON-to-wire builders, market/config call args, result parsers.
- JSON-RPC: async client, injectable transport for tests, read methods for common
  account/block/token/NFT/archive/contract/state calls, send helpers for VM
  script and Carbon transactions, response DTOs with serde defaults and scalar
  coercion for reference RPC response quirks. Token, series, NFT and organization
  property values decode into `VmValue`, keeping the scalar, array or struct shape
  the node answers.
- Extended events: `EventExResult.data` decodes into `EventData`, typed by the
  event kind (token creation, series creation, market orders, special
  resolutions). Special-resolution call arguments are typed per module and
  method - the same 43 shapes the C# and TypeScript SDKs model - with `rawArgs`
  fallback detection by content.

## Rust API Decisions

- Errors use one `PhantasmaError` enum and `Result<T>` alias.
- Public builders validate inputs before producing bytes.
- Readers reject truncated payloads, oversized arrays, unsupported tags, and
  trailing bytes where whole-object parsing is expected.
- Data structures use Rust naming and strong types instead of mirroring C#/Python
  class names or nullable dynamic maps.
- Python names like `PhantasmaRPC`, `ModuleID`, and `ABIParameterResult` map to
  Rust names like `PhantasmaRpc`, `ModuleId`, and `AbiParameterResult`.
- Python exception subclasses map to variants of `PhantasmaError`.
- Async RPC is transport-generic so unit tests do not require a live node.
- `VmValue` keeps every scalar as text: chain values are big integers, and parsing
  them into a numeric type would either overflow or lose precision. Arrays and
  structs keep their own shape instead of collapsing into a packed string.
- Extended event data is a typed enum, not the C# `object`/TypeScript union:
  Rust consumers pattern match on `EventData` and on the per-method
  `SpecialResolutionArguments` instead of casting. Both enums keep unmodeled or
  mismatched payloads verbatim (`EventData::Unknown`,
  `SpecialResolutionArguments::Unrecognized`) where the C# converter returns
  null for an unmodeled method - a deliberate divergence so a node newer than
  this SDK never loses answered data, and decoding stays total (one unexpected
  event cannot fail a whole block answer).
- `TokenMintData` exists in the C# and TypeScript SDKs but is not ported: the
  node does not emit a `TokenMint` extended event (no construction site in
  RpcEventBuilder), so there is no wire shape to model. If the node starts
  emitting it, the payload arrives in `EventData::Unknown` until the variant is
  added.
- Numeric fields follow the wire exactly: counts and big-integer ids are
  strings, Carbon ids (`carbonTokenId`, `moduleId`, `resolutionId`) and
  timestamps are JSON numbers mapped to `u64`/`u32`/`i64`.
- Examples avoid funded or broadcasting workflows unless the caller explicitly
  chooses to run a send method.

## Test Sources

- `tests/fixtures/carbon_vectors.tsv` is copied from the Python SDK fixture set and
  covers shared Carbon vectors, including non-canonical read cases that must
  match the reference behavior.
- `tests/binary_transaction_vm.rs` covers VM wire formats and script output.
- `tests/encoding_crypto.rs` covers WIF/address/signature/hash behavior.
- `tests/carbon_builders.rs` covers higher-level Carbon builders and result
  parsers, including Python metadata validation behavior for required fields,
  case-sensitive schema names, ROM bytes, fixed bytes, unsigned-to-signed
  integer coercion, and array-of-struct metadata.
- `tests/carbon_python_parity.rs` covers Python SDK parity for Carbon token,
  market, config, parsed schema JSON plus schema builders, and deterministic
  Phantasma NFT helper paths.
- `tests/rpc.rs` covers JSON-RPC request/response behavior through a mock
  transport.
- `tests/extended_events.rs` covers typed extended-event decoding: kind
  dispatch, the full 43-pair argument dispatch table, raw fallbacks, and wire
  round-trips, with fixtures captured from devnet on 2026-08-01.

Live localnet execution and funded/broadcasting examples are intentionally not
part of the default test suite. The read-only RPC example can be run against an
existing endpoint; offline examples never broadcast.
