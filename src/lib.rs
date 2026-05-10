//! Rust SDK for the Phantasma blockchain with support for the Phoenix chain update.
//!
//! The crate exposes checked primitives for transaction building and signing,
//! VM scripts, VM script transactions, Carbon payloads, Ed25519 keys, and
//! JSON-RPC access. Public APIs return `Result` instead of panicking on
//! malformed user input or hostile external data.

pub mod binary;
pub mod carbon;
pub mod crypto;
pub mod encoding;
pub mod error;
pub mod rpc;
pub mod transaction;
pub mod vm;

pub use binary::{
    big_int_to_vm_bytes, vm_bytes_to_big_int, BinaryReader, BinaryWriter, MAX_ARRAY_SIZE,
};
pub use carbon::*;
pub use crypto::{
    Address, AddressKind, Ed25519Signature, Hash, PhantasmaKeys, SignatureKind, ADDRESS_LENGTH,
    PRIVATE_KEY_LENGTH, PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH,
};
pub use encoding::{decode_base58, decode_hex, encode_base58, encode_hex, encode_hex_upper};
pub use error::{PhantasmaError, Result};
pub use rpc::*;
pub use transaction::{tx_state_is_fault, tx_state_is_success, Transaction, SDK_PAYLOAD};
pub use vm::{Opcode, ScriptArg, ScriptBuilder, VMObject};

pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");
