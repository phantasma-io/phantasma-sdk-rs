//! Classic VM transaction building, signing, parsing, and local PoW helpers.

use num_bigint::BigInt;

use crate::binary::{BinaryReader, BinaryWriter, MAX_ARRAY_SIZE};
use crate::crypto::{Ed25519Signature, Hash, PhantasmaKeys, SignatureKind};
use crate::error::{serialization, Result};

/// Default payload for newly built classic transactions.
///
/// The string is visible in signed transaction bytes, so tests keep it aligned
/// with the crate version declared in `Cargo.toml`.
pub const SDK_PAYLOAD: &[u8] = b"RS-SDK-v1.0.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
    pub nexus_name: String,
    pub chain_name: String,
    pub script: Vec<u8>,
    pub expiration: u32,
    pub payload: Vec<u8>,
    pub signatures: Vec<Ed25519Signature>,
}

impl Transaction {
    pub fn new(
        nexus_name: impl Into<String>,
        chain_name: impl Into<String>,
        script: impl Into<Vec<u8>>,
        expiration: u32,
    ) -> Self {
        Self {
            nexus_name: nexus_name.into(),
            chain_name: chain_name.into(),
            script: script.into(),
            expiration,
            payload: SDK_PAYLOAD.to_vec(),
            signatures: Vec::new(),
        }
    }

    pub fn with_payload(mut self, payload: impl Into<Vec<u8>>) -> Self {
        self.payload = payload.into();
        self
    }

    pub fn hash(&self) -> Hash {
        Hash::sha256(self.to_bytes(false))
    }

    pub fn to_bytes(&self, with_signatures: bool) -> Vec<u8> {
        let mut writer = BinaryWriter::new();
        writer.write_string(&self.nexus_name);
        writer.write_string(&self.chain_name);
        writer.write_var_bytes(&self.script);
        writer.write_u32_le(self.expiration);
        writer.write_var_bytes(&self.payload);
        if with_signatures {
            writer.write_var_uint(self.signatures.len() as u64);
            for signature in &self.signatures {
                writer.write_u8(signature.kind() as u8);
                writer.write_var_bytes(signature.data());
            }
        }
        writer.into_bytes()
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut reader = BinaryReader::new(data);
        let mut tx = Self {
            nexus_name: reader.read_string()?,
            chain_name: reader.read_string()?,
            script: reader.read_var_bytes(MAX_ARRAY_SIZE)?,
            expiration: reader.read_u32_le()?,
            payload: reader.read_var_bytes(MAX_ARRAY_SIZE)?,
            signatures: Vec::new(),
        };
        let count = reader.read_var_uint()?;
        for _ in 0..count {
            let kind = reader.read_u8()?;
            if kind != SignatureKind::Ed25519 as u8 {
                return serialization(format!("unsupported signature kind: {kind}"));
            }
            tx.signatures.push(Ed25519Signature::try_from_slice(
                &reader.read_var_bytes(MAX_ARRAY_SIZE)?,
            )?);
        }
        reader.assert_eof()?;
        Ok(tx)
    }

    pub fn sign(&mut self, key_pair: &PhantasmaKeys) -> Ed25519Signature {
        let signature = key_pair.sign(self.to_bytes(false));
        self.signatures.push(signature.clone());
        signature
    }

    pub fn is_signed_by(&self, key_pair: &PhantasmaKeys) -> bool {
        let message = self.to_bytes(false);
        let address = key_pair.address();
        self.signatures
            .iter()
            .any(|signature| signature.verify(&message, [&address]))
    }

    pub fn mine(&mut self, difficulty: u32) {
        // Mining only mutates the payload nonce, mirroring the lightweight SDK
        // helper behavior. The transaction script and signatures remain caller
        // controlled.
        if difficulty == 0 {
            return;
        }
        let mut nonce = 0u32;
        while self.hash().difficulty() < difficulty {
            nonce = nonce.wrapping_add(1);
            self.payload = nonce.to_le_bytes().to_vec();
        }
    }
}

pub fn tx_state_is_success(state: &str) -> bool {
    state.eq_ignore_ascii_case("HALT")
}

pub fn tx_state_is_fault(state: &str) -> bool {
    state.eq_ignore_ascii_case("FAULT") || state.eq_ignore_ascii_case("BREAK")
}

pub fn big_int(value: i64) -> BigInt {
    BigInt::from(value)
}
