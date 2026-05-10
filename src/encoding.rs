//! Text/binary encodings shared across SDK modules.
//!
//! These helpers normalize hex and Base58 behavior at the boundary so address,
//! key, transaction, and RPC APIs return typed SDK errors instead of raw crate
//! errors or panics.

use crate::error::{encoding, Result};

const BASE58_ALPHABET: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

pub fn encode_base58(data: impl AsRef<[u8]>) -> String {
    bs58::encode(data.as_ref())
        .with_alphabet(bs58::Alphabet::BITCOIN)
        .into_string()
}

pub fn decode_base58(text: &str) -> Result<Vec<u8>> {
    if text.is_empty() {
        return encoding("base58 text is empty");
    }
    if let Some(ch) = text.chars().find(|ch| !BASE58_ALPHABET.contains(*ch)) {
        return encoding(format!("invalid base58 character: {ch:?}"));
    }
    bs58::decode(text)
        .with_alphabet(bs58::Alphabet::BITCOIN)
        .into_vec()
        .map_err(|err| crate::error::PhantasmaError::Encoding(err.to_string()))
}

pub fn decode_hex(value: &str) -> Result<Vec<u8>> {
    let text = value.trim();
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    if text.len() % 2 != 0 {
        return encoding("hex value must contain an even number of digits");
    }
    Ok(hex::decode(text)?)
}

pub fn encode_hex(data: impl AsRef<[u8]>) -> String {
    hex::encode(data.as_ref())
}

pub fn encode_hex_upper(data: impl AsRef<[u8]>) -> String {
    hex::encode_upper(data.as_ref())
}
