//! Classic VM binary primitives.
//!
//! This module intentionally keeps classic VM serialization separate from
//! Carbon serialization. The two formats share some concepts but not enough
//! byte-level rules to safely reuse one reader/writer for both.

use num_bigint::{BigInt, Sign};
use num_traits::{One, Zero};

use crate::error::{serialization, Result};

/// Upper bound used when reading length-prefixed external data.
///
/// The VM format can express larger lengths, but SDK readers should fail
/// before hostile payloads can force unbounded allocation.
pub const MAX_ARRAY_SIZE: usize = 0x0100_0000;

#[derive(Debug, Clone, Default)]
pub struct BinaryWriter {
    buffer: Vec<u8>,
}

impl BinaryWriter {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn write_u8(&mut self, value: u8) {
        self.buffer.push(value);
    }

    pub fn write_u16_le(&mut self, value: u16) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u32_le(&mut self, value: u32) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_u64_le(&mut self, value: u64) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_i64_le(&mut self, value: i64) {
        self.buffer.extend_from_slice(&value.to_le_bytes());
    }

    pub fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) {
        self.buffer.extend_from_slice(data.as_ref());
    }

    pub fn write_var_uint(&mut self, value: u64) {
        if value < 0xFD {
            self.write_u8(value as u8);
        } else if value <= 0xFFFF {
            self.write_u8(0xFD);
            self.write_u16_le(value as u16);
        } else if value <= 0xFFFF_FFFF {
            self.write_u8(0xFE);
            self.write_u32_le(value as u32);
        } else {
            self.write_u8(0xFF);
            self.write_u64_le(value);
        }
    }

    pub fn write_var_bytes(&mut self, data: impl AsRef<[u8]>) {
        let raw = data.as_ref();
        self.write_var_uint(raw.len() as u64);
        self.write(raw);
    }

    pub fn write_string(&mut self, value: &str) {
        self.write_var_bytes(value.as_bytes());
    }

    pub fn write_big_integer(&mut self, value: &BigInt) -> Result<()> {
        self.write_var_bytes(big_int_to_vm_bytes(value)?);
        Ok(())
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buffer
    }

    pub fn bytes(&self) -> &[u8] {
        &self.buffer
    }
}

#[derive(Debug, Clone)]
pub struct BinaryReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> BinaryReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    pub fn assert_eof(&self) -> Result<()> {
        if self.remaining() != 0 {
            return serialization(format!("unexpected trailing bytes: {}", self.remaining()));
        }
        Ok(())
    }

    pub fn read(&mut self, count: usize) -> Result<Vec<u8>> {
        if count > self.remaining() {
            return serialization("end of stream reached");
        }
        let start = self.offset;
        self.offset += count;
        Ok(self.data[start..start + count].to_vec())
    }

    pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let raw = self.read(N)?;
        Ok(raw.try_into().expect("fixed read length"))
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_array::<1>()?[0])
    }

    pub fn read_u16_le(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.read_array::<2>()?))
    }

    pub fn read_u32_le(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    pub fn read_u64_le(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    pub fn read_i64_le(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read_array::<8>()?))
    }

    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    pub fn read_var_uint(&mut self) -> Result<u64> {
        match self.read_u8()? {
            0xFD => Ok(u64::from(self.read_u16_le()?)),
            0xFE => Ok(u64::from(self.read_u32_le()?)),
            0xFF => self.read_u64_le(),
            value => Ok(u64::from(value)),
        }
    }

    pub fn read_var_bytes(&mut self, max_size: usize) -> Result<Vec<u8>> {
        let size = self.read_var_uint()?;
        let size: usize = size.try_into().map_err(|_| {
            crate::error::PhantasmaError::Serialization("byte array too large".into())
        })?;
        if size > max_size {
            return serialization(format!("byte array too large: {size}"));
        }
        self.read(size)
    }

    pub fn read_string(&mut self) -> Result<String> {
        String::from_utf8(self.read_var_bytes(MAX_ARRAY_SIZE)?)
            .map_err(|err| crate::error::PhantasmaError::Serialization(err.to_string()))
    }

    pub fn read_big_integer(&mut self) -> Result<BigInt> {
        Ok(vm_bytes_to_big_int(&self.read_var_bytes(MAX_ARRAY_SIZE)?))
    }
}

pub(crate) fn big_int_to_csharp_bytes(value: &BigInt) -> Vec<u8> {
    if value.is_zero() {
        vec![0]
    } else {
        value.to_signed_bytes_le()
    }
}

pub fn big_int_to_vm_bytes(value: &BigInt) -> Result<Vec<u8>> {
    // BinaryWriter/VMObject numbers use Phantasma's Gen2 `ToSignedByteArray`
    // shape. It starts from normal C# BigInteger bytes, then adds VM-specific
    // sign padding that ScriptBuilder's LOAD instruction deliberately does not
    // use.
    let mut raw = big_int_to_csharp_bytes(value);
    if value.sign() == Sign::Minus {
        if raw.len() == 1 {
            raw.extend_from_slice(&[0xFF, 0xFF]);
        } else if raw.last() == Some(&0xFF) {
            raw.push(0xFF);
        }
    } else if raw.last() != Some(&0x00) {
        raw.push(0x00);
    }
    Ok(raw)
}

pub fn vm_bytes_to_big_int(data: &[u8]) -> BigInt {
    if data.is_empty() {
        BigInt::zero()
    } else {
        BigInt::from_signed_bytes_le(data)
    }
}

pub(crate) fn signed_word_256(value: &BigInt) -> Result<[u8; 32]> {
    // Carbon BigInt-compatible fields are stored as a 256-bit signed word. The
    // unsigned upper range is accepted because reference SDK metadata builders
    // allow callers to pass two's-complement values before wire conversion.
    let min = -(BigInt::one() << 255usize);
    let max = (BigInt::one() << 256usize) - BigInt::one();
    if value < &min || value > &max {
        return serialization("BigInt overflow");
    }
    let modulus = BigInt::one() << 256usize;
    let unsigned = if value.sign() == Sign::Minus {
        value + &modulus
    } else {
        value.clone()
    };
    let (_sign, mut bytes) = unsigned.to_bytes_le();
    bytes.resize(32, 0);
    Ok(bytes.try_into().expect("word resized to 32 bytes"))
}

pub(crate) fn signed_word_to_big_int(word: &[u8; 32]) -> BigInt {
    BigInt::from_signed_bytes_le(word)
}

pub(crate) fn ensure_u32_len(len: usize) -> Result<u32> {
    len.try_into()
        .map_err(|_| crate::error::PhantasmaError::Serialization("array length exceeds u32".into()))
}
