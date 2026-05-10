//! Cryptographic primitives used by both classic and Carbon transactions.
//!
//! Keys, addresses, WIF handling, signatures, and hash difficulty live here so
//! higher-level builders do not need to know Phantasma address byte layout.

use std::fmt;
use std::str::FromStr;

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use sha2::{Digest, Sha256};

use crate::binary::BinaryWriter;
use crate::encoding::{decode_base58, encode_base58, encode_hex_upper};
use crate::error::{crypto, Result};

pub const ADDRESS_LENGTH: usize = 34;
pub const PRIVATE_KEY_LENGTH: usize = 32;
pub const PUBLIC_KEY_LENGTH: usize = 32;
pub const SIGNATURE_LENGTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AddressKind {
    Invalid = 0,
    User = 1,
    System = 2,
    Interop = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SignatureKind {
    None = 0,
    Ed25519 = 1,
    Ecdsa = 2,
    Ring = 3,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address {
    data: [u8; ADDRESS_LENGTH],
}

impl Address {
    pub fn new(data: [u8; ADDRESS_LENGTH]) -> Self {
        Self { data }
    }

    pub fn try_from_slice(data: &[u8]) -> Result<Self> {
        let data: [u8; ADDRESS_LENGTH] = data.try_into().map_err(|_| {
            crate::error::PhantasmaError::Crypto(format!(
                "address length must be {ADDRESS_LENGTH}, got {}",
                data.len()
            ))
        })?;
        Ok(Self::new(data))
    }

    pub fn null() -> Self {
        Self {
            data: [0; ADDRESS_LENGTH],
        }
    }

    pub fn from_public_key(public_key: &[u8]) -> Result<Self> {
        let mut data = [0u8; ADDRESS_LENGTH];
        match public_key.len() {
            PUBLIC_KEY_LENGTH => {
                data[0] = AddressKind::User as u8;
                data[1] = 0;
                data[2..].copy_from_slice(public_key);
            }
            33 => {
                data[0] = AddressKind::User as u8;
                data[1..].copy_from_slice(public_key);
            }
            64 => {
                data[0] = AddressKind::User as u8;
                data[1] = 0;
                data[2..].copy_from_slice(&public_key[..PUBLIC_KEY_LENGTH]);
            }
            len => return crypto(format!("invalid public key length: {len}")),
        }
        Ok(Self { data })
    }

    pub fn from_text(text: impl AsRef<str>) -> Result<Self> {
        let text = text.as_ref();
        if text.is_empty() || text.eq_ignore_ascii_case("NULL") {
            return Ok(Self::null());
        }
        if text.len() < 2 {
            return crypto("address text is too short");
        }
        let prefix = text.as_bytes()[0] as char;
        let data = decode_base58(&text[1..])?;
        let address = Self::try_from_slice(&data)?;
        match prefix {
            'P' if address.kind() != AddressKind::User => crypto("address has to be of type User"),
            'S' if address.kind() != AddressKind::System => {
                crypto("address has to be of type System")
            }
            'X' if address.kind() != AddressKind::Interop => {
                crypto("address has to be of type Interop")
            }
            'P' | 'S' | 'X' => Ok(address),
            other => crypto(format!("unknown address prefix: {other}")),
        }
    }

    pub fn from_hash(value: impl AsRef<[u8]>) -> Self {
        let mut data = [0u8; ADDRESS_LENGTH];
        data[0] = AddressKind::User as u8;
        data[1] = 0;
        data[2..].copy_from_slice(&Sha256::digest(value.as_ref()));
        Self { data }
    }

    pub fn data(&self) -> &[u8; ADDRESS_LENGTH] {
        &self.data
    }

    pub fn into_bytes(self) -> [u8; ADDRESS_LENGTH] {
        self.data
    }

    pub fn is_null(&self) -> bool {
        self.data == [0; ADDRESS_LENGTH]
    }

    pub fn kind(&self) -> AddressKind {
        if self.is_null() {
            return AddressKind::System;
        }
        match self.data[0] {
            1 => AddressKind::User,
            2 => AddressKind::System,
            value if value >= 3 => AddressKind::Interop,
            _ => AddressKind::Invalid,
        }
    }

    pub fn public_key(&self) -> Result<[u8; PUBLIC_KEY_LENGTH]> {
        if self.kind() != AddressKind::User {
            return crypto("only user addresses contain an Ed25519 public key");
        }
        Ok(self.data[2..]
            .try_into()
            .expect("address public key length"))
    }

    pub fn to_text(&self) -> String {
        if self.is_null() {
            return "NULL".to_string();
        }
        let prefix = match self.kind() {
            AddressKind::System => 'S',
            AddressKind::Interop => 'X',
            _ => 'P',
        };
        format!("{prefix}{}", encode_base58(self.data))
    }

    pub fn prefixed_bytes(&self) -> Vec<u8> {
        let mut writer = BinaryWriter::new();
        writer.write_var_bytes(self.data);
        writer.into_bytes()
    }
}

impl Default for Address {
    fn default() -> Self {
        Self::null()
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Address").field(&self.to_text()).finish()
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_text())
    }
}

impl FromStr for Address {
    type Err = crate::error::PhantasmaError;

    fn from_str(s: &str) -> Result<Self> {
        Self::from_text(s)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub fn new(data: [u8; 32]) -> Self {
        Self(data)
    }

    pub fn try_from_slice(data: &[u8]) -> Result<Self> {
        let data: [u8; 32] = data.try_into().map_err(|_| {
            crate::error::PhantasmaError::Crypto(format!(
                "hash length must be 32, got {}",
                data.len()
            ))
        })?;
        Ok(Self(data))
    }

    pub fn sha256(data: impl AsRef<[u8]>) -> Self {
        Self(Sha256::digest(data.as_ref()).into())
    }

    pub fn to_hex(&self) -> String {
        encode_hex_upper(self.0)
    }

    pub fn difficulty(&self) -> u32 {
        // Phantasma PoW difficulty is measured from the last set bit in the
        // little-endian hash representation, matching the classic SDKs.
        let mut last_set_bit = 0u32;
        for (byte_index, byte) in self.0.iter().copied().enumerate() {
            for bit_index in 0..8 {
                if byte & (1 << bit_index) != 0 {
                    last_set_bit = 1 + ((byte_index as u32) << 3) + bit_index;
                }
            }
        }
        256 - last_set_bit
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ed25519Signature {
    data: [u8; SIGNATURE_LENGTH],
}

impl Ed25519Signature {
    pub fn new(data: [u8; SIGNATURE_LENGTH]) -> Self {
        Self { data }
    }

    pub fn try_from_slice(data: &[u8]) -> Result<Self> {
        let data: [u8; SIGNATURE_LENGTH] = data.try_into().map_err(|_| {
            crate::error::PhantasmaError::Crypto(format!(
                "signature length must be {SIGNATURE_LENGTH}, got {}",
                data.len()
            ))
        })?;
        Ok(Self { data })
    }

    pub fn data(&self) -> &[u8; SIGNATURE_LENGTH] {
        &self.data
    }

    pub fn kind(&self) -> SignatureKind {
        SignatureKind::Ed25519
    }

    pub fn verify<'a>(
        &self,
        message: &[u8],
        addresses: impl IntoIterator<Item = &'a Address>,
    ) -> bool {
        let signature = Signature::from_bytes(&self.data);
        addresses.into_iter().any(|address| {
            if address.kind() != AddressKind::User {
                return false;
            }
            let Ok(public_key) = address.public_key() else {
                return false;
            };
            let Ok(key) = VerifyingKey::from_bytes(&public_key) else {
                return false;
            };
            key.verify(message, &signature).is_ok()
        })
    }

    pub fn serialize_data(&self) -> Vec<u8> {
        let mut writer = BinaryWriter::new();
        writer.write_var_bytes(self.data);
        writer.into_bytes()
    }
}

#[derive(Clone)]
pub struct PhantasmaKeys {
    private_key: [u8; PRIVATE_KEY_LENGTH],
}

impl PhantasmaKeys {
    pub fn new(private_key: [u8; PRIVATE_KEY_LENGTH]) -> Self {
        Self { private_key }
    }

    pub fn try_from_slice(private_key: &[u8]) -> Result<Self> {
        let raw = if private_key.len() == 64 {
            &private_key[..PRIVATE_KEY_LENGTH]
        } else {
            private_key
        };
        let private_key: [u8; PRIVATE_KEY_LENGTH] = raw.try_into().map_err(|_| {
            crate::error::PhantasmaError::Crypto(format!(
                "private key length must be {PRIVATE_KEY_LENGTH}, got {}",
                private_key.len()
            ))
        })?;
        Ok(Self { private_key })
    }

    pub fn generate() -> Self {
        Self {
            private_key: SigningKey::generate(&mut OsRng).to_bytes(),
        }
    }

    pub fn from_wif(wif: &str) -> Result<Self> {
        // Phantasma SDKs use the compressed private-key WIF envelope even
        // though the signing algorithm is Ed25519. Rejecting other WIF shapes
        // avoids silently deriving a different address from user input.
        if wif.is_empty() {
            return crypto("WIF is required");
        }
        let data = decode_base58(wif)?;
        if data.len() != 38 {
            return crypto("invalid WIF length");
        }
        let (payload, checksum) = data.split_at(data.len() - 4);
        if &double_sha256(payload)[..4] != checksum {
            return crypto("invalid WIF checksum");
        }
        if payload.len() != 34 || payload[0] != 0x80 || payload[33] != 0x01 {
            return crypto("invalid compressed Ed25519 WIF payload");
        }
        Self::try_from_slice(&payload[1..33])
    }

    pub fn private_key(&self) -> &[u8; PRIVATE_KEY_LENGTH] {
        &self.private_key
    }

    pub fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.private_key)
    }

    pub fn public_key(&self) -> [u8; PUBLIC_KEY_LENGTH] {
        self.signing_key().verifying_key().to_bytes()
    }

    pub fn address(&self) -> Address {
        Address::from_public_key(&self.public_key()).expect("derived Ed25519 public key is valid")
    }

    pub fn to_wif(&self) -> String {
        let mut payload = Vec::with_capacity(34);
        payload.push(0x80);
        payload.extend_from_slice(&self.private_key);
        payload.push(0x01);
        let checksum = double_sha256(&payload);
        payload.extend_from_slice(&checksum[..4]);
        encode_base58(payload)
    }

    pub fn sign(&self, message: impl AsRef<[u8]>) -> Ed25519Signature {
        Ed25519Signature::new(self.signing_key().sign(message.as_ref()).to_bytes())
    }
}

impl fmt::Debug for PhantasmaKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PhantasmaKeys")
            .field("address", &self.address().to_text())
            .finish_non_exhaustive()
    }
}

impl fmt::Display for PhantasmaKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.address())
    }
}

pub fn double_sha256(data: impl AsRef<[u8]>) -> [u8; 32] {
    let first = Sha256::digest(data.as_ref());
    Sha256::digest(first).into()
}
