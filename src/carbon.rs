//! Gen3 Carbon wire formats and transaction builders.
//!
//! Carbon uses a different binary format from the classic VM. This module keeps
//! the low-level reader/writer, schema-aware dynamic values, token/NFT helper
//! structures, and signed transaction messages together so shared vectors can
//! validate byte-level parity against the reference SDKs.

use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use bitflags::bitflags;
use num_bigint::BigInt;
use num_traits::{ToPrimitive, Zero};
use serde_json::Value;

use crate::binary::{ensure_u32_len, signed_word_256, signed_word_to_big_int};
use crate::crypto::{Address, AddressKind, PhantasmaKeys};
use crate::encoding::decode_hex;
use crate::error::{builder, crypto, serialization, PhantasmaError, Result};

pub trait CarbonSerializable: Sized {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()>;
    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self>;
}

pub fn serialize<T: CarbonSerializable>(value: &T) -> Result<Vec<u8>> {
    let mut writer = CarbonWriter::new();
    value.write_carbon(&mut writer)?;
    Ok(writer.into_bytes())
}

pub fn deserialize<T: CarbonSerializable>(data: impl AsRef<[u8]>) -> Result<T> {
    let mut reader = CarbonReader::new(data.as_ref());
    let value = T::read_carbon(&mut reader)?;
    reader.assert_eof()?;
    Ok(value)
}

macro_rules! fixed_bytes {
    ($name:ident, $size:expr) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; $size]);

        impl $name {
            pub const SIZE: usize = $size;

            pub fn new(data: [u8; $size]) -> Self {
                Self(data)
            }

            pub fn try_from_slice(data: &[u8]) -> Result<Self> {
                let data: [u8; $size] = data.try_into().map_err(|_| {
                    PhantasmaError::Serialization(format!(
                        "{} length must be {}, got {}",
                        stringify!($name),
                        $size,
                        data.len()
                    ))
                })?;
                Ok(Self(data))
            }

            pub fn from_hex(value: &str) -> Result<Self> {
                Self::try_from_slice(&decode_hex(value)?)
            }

            pub fn as_bytes(&self) -> &[u8; $size] {
                &self.0
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), hex::encode(self.0))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&hex::encode(self.0))
            }
        }

        impl From<[u8; $size]> for $name {
            fn from(value: [u8; $size]) -> Self {
                Self(value)
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self([0; $size])
            }
        }

        impl FromStr for $name {
            type Err = PhantasmaError;

            fn from_str(s: &str) -> Result<Self> {
                Self::from_hex(s)
            }
        }
    };
}

fixed_bytes!(Bytes16, 16);
fixed_bytes!(Bytes32, 32);
fixed_bytes!(Bytes64, 64);

pub const EMPTY_BYTES16: Bytes16 = Bytes16([0; 16]);
pub const EMPTY_BYTES32: Bytes32 = Bytes32([0; 32]);
pub const EMPTY_BYTES64: Bytes64 = Bytes64([0; 64]);
pub const SYSTEM_ADDRESS_NULL: Bytes32 = EMPTY_BYTES32;
pub const SYSTEM_ADDRESS_GAS_POOL: Bytes32 = Bytes32([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
]);
pub const SYSTEM_ADDRESS_DATA_POOL: Bytes32 = Bytes32([
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2,
]);
pub const STANDARD_META_ID: &str = "_i";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SmallString(pub String);

impl SmallString {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.len() > 255 {
            return serialization("SmallString exceeds 255 UTF-8 bytes");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SmallString {
    fn from(value: &str) -> Self {
        Self::new(value).expect("static SmallString fits")
    }
}

impl From<String> for SmallString {
    fn from(value: String) -> Self {
        Self::new(value).expect("SmallString conversion fits")
    }
}

impl fmt::Display for SmallString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl CarbonSerializable for SmallString {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        let raw = self.0.as_bytes();
        writer.write1(raw.len().try_into().map_err(|_| {
            PhantasmaError::Serialization("SmallString exceeds 255 UTF-8 bytes".into())
        })?);
        writer.write(raw);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let len = reader.read1()? as usize;
        let raw = reader.read(len)?;
        String::from_utf8(raw)
            .map(Self)
            .map_err(|err| PhantasmaError::Serialization(err.to_string()))
    }
}

#[derive(Debug, Clone, Default)]
pub struct CarbonWriter {
    buffer: Vec<u8>,
}

impl CarbonWriter {
    pub fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    pub fn write(&mut self, data: impl AsRef<[u8]>) {
        self.buffer.extend_from_slice(data.as_ref());
    }

    pub fn write1(&mut self, value: u8) {
        self.buffer.push(value);
    }

    pub fn write2(&mut self, value: i16) {
        self.write(value.to_le_bytes());
    }

    pub fn write4(&mut self, value: i32) {
        self.write(value.to_le_bytes());
    }

    pub fn write4u(&mut self, value: u32) {
        self.write(value.to_le_bytes());
    }

    pub fn write8(&mut self, value: i64) {
        self.write(value.to_le_bytes());
    }

    pub fn write8u(&mut self, value: u64) {
        self.write(value.to_le_bytes());
    }

    pub fn write16(&mut self, value: Bytes16) {
        self.write(value.0);
    }

    pub fn write32(&mut self, value: Bytes32) {
        self.write(value.0);
    }

    pub fn write64(&mut self, value: Bytes64) {
        self.write(value.0);
    }

    pub fn write_big_int(&mut self, value: &BigInt) -> Result<()> {
        // Carbon BigInt writes a compact header plus the shortest little-endian
        // slice of a 256-bit two's-complement word. The header sign bit carries
        // the fill byte used to reconstruct the full word on read.
        if value.is_zero() {
            self.write1(0);
            return Ok(());
        }

        let word = signed_word_256(value)?;
        let fill = if word[31] & 0x80 != 0 { 0xFF } else { 0x00 };
        let mut len = word.len();
        while len > 0 && word[len - 1] == fill {
            len -= 1;
        }
        let mut header = (len & 0x3F) as u8;
        if fill == 0xFF {
            header |= 0x80;
        }
        self.write1(header);
        self.write(&word[..len]);
        Ok(())
    }

    pub fn write_big_int_array(&mut self, values: &[BigInt]) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            self.write_big_int(value)?;
        }
        Ok(())
    }

    pub fn write_string_z(&mut self, value: &str) -> Result<()> {
        if value.as_bytes().contains(&0) {
            return serialization("zero-terminated string contains a zero byte");
        }
        self.write(value.as_bytes());
        self.write1(0);
        Ok(())
    }

    pub fn write_string_z_array(&mut self, values: &[String]) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            self.write_string_z(value)?;
        }
        Ok(())
    }

    pub fn write_byte_array(&mut self, data: impl AsRef<[u8]>) -> Result<()> {
        let raw = data.as_ref();
        self.write4(ensure_u32_len(raw.len())? as i32);
        self.write(raw);
        Ok(())
    }

    pub fn write_byte_arrays(&mut self, values: &[Vec<u8>]) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            self.write_byte_array(value)?;
        }
        Ok(())
    }

    pub fn write_i8_array(&mut self, values: &[i8]) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            self.write1(*value as u8);
        }
        Ok(())
    }

    pub fn write_i16_array(&mut self, values: &[i16]) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            self.write2(*value);
        }
        Ok(())
    }

    pub fn write_i32_array(&mut self, values: &[i32]) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            self.write4(*value);
        }
        Ok(())
    }

    pub fn write_i64_array(&mut self, values: &[i64]) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            self.write8(*value);
        }
        Ok(())
    }

    pub fn write_u64_array(&mut self, values: &[u64]) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            self.write8u(*value);
        }
        Ok(())
    }

    pub fn write_int_array(&mut self, values: &[i128], width: u8, signed: bool) -> Result<()> {
        self.write4(ensure_u32_len(values.len())? as i32);
        for value in values {
            match width {
                1 => {
                    ensure_i128_range(*value, i128::from(i8::MIN), i128::from(u8::MAX), "i8")?;
                    self.write1(*value as u8);
                }
                2 if signed => {
                    ensure_i128_range(*value, i128::from(i16::MIN), i128::from(i16::MAX), "i16")?;
                    self.write2(*value as i16);
                }
                2 => {
                    ensure_i128_range(*value, 0, i128::from(u16::MAX), "u16")?;
                    self.write((*value as u16).to_le_bytes());
                }
                4 if signed => {
                    ensure_i128_range(*value, i128::from(i32::MIN), i128::from(i32::MAX), "i32")?;
                    self.write4(*value as i32);
                }
                4 => {
                    ensure_i128_range(*value, 0, i128::from(u32::MAX), "u32")?;
                    self.write4u(*value as u32);
                }
                8 if signed => {
                    ensure_i128_range(*value, i128::from(i64::MIN), i128::from(i64::MAX), "i64")?;
                    self.write8(*value as i64);
                }
                8 => {
                    ensure_i128_range(*value, 0, i128::from(u64::MAX), "u64")?;
                    self.write8u(*value as u64);
                }
                _ => return serialization(format!("unsupported integer width: {width}")),
            }
        }
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
pub struct CarbonReader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> CarbonReader<'a> {
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

    pub fn read_length(&mut self) -> Result<usize> {
        let length = self.read4()?;
        if length < 0 {
            return serialization("negative array length");
        }
        let length = length as usize;
        if length > self.remaining() {
            return serialization(format!(
                "array length {length} exceeds remaining bytes {}",
                self.remaining()
            ));
        }
        Ok(length)
    }

    pub fn read1(&mut self) -> Result<u8> {
        Ok(self.read_array::<1>()?[0])
    }

    pub fn read2(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.read_array::<2>()?))
    }

    pub fn read4(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.read_array::<4>()?))
    }

    pub fn read4u(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.read_array::<4>()?))
    }

    pub fn read8(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read_array::<8>()?))
    }

    pub fn read8u(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.read_array::<8>()?))
    }

    pub fn read16(&mut self) -> Result<Bytes16> {
        Ok(Bytes16(self.read_array::<16>()?))
    }

    pub fn read32(&mut self) -> Result<Bytes32> {
        Ok(Bytes32(self.read_array::<32>()?))
    }

    pub fn read64(&mut self) -> Result<Bytes64> {
        Ok(Bytes64(self.read_array::<64>()?))
    }

    pub fn read_big_int(&mut self) -> Result<BigInt> {
        let header = self.read1()?;
        self.read_big_int_with_header(header)
    }

    pub fn read_big_int_with_header(&mut self, header: u8) -> Result<BigInt> {
        // Bit 6 is reserved in the current Carbon encoding. Rejecting it keeps
        // readers fail-closed instead of accepting an ambiguous future format.
        if header == 0 {
            return Ok(BigInt::zero());
        }
        let len = (header & 0x3F) as usize;
        if header & 0x40 != 0 || len > 32 {
            return serialization("BigInt too big");
        }
        let fill = if header & 0x80 != 0 { 0xFF } else { 0x00 };
        let mut word = [fill; 32];
        if len > 0 {
            let raw = self.read(len)?;
            word[..len].copy_from_slice(&raw);
        }
        if ((word[31] & 0x80) != 0) != ((header & 0x80) != 0) {
            return serialization("non-standard BigInt header");
        }
        Ok(signed_word_to_big_int(&word))
    }

    pub fn read_big_int_array(&mut self) -> Result<Vec<BigInt>> {
        let count = self.read_length()?;
        (0..count).map(|_| self.read_big_int()).collect()
    }

    pub fn read_string_z(&mut self) -> Result<String> {
        let start = self.offset;
        while self.offset < self.data.len() && self.data[self.offset] != 0 {
            self.offset += 1;
        }
        if self.offset >= self.data.len() {
            return serialization("end of stream reached");
        }
        let raw = self.data[start..self.offset].to_vec();
        self.offset += 1;
        String::from_utf8(raw).map_err(|err| PhantasmaError::Serialization(err.to_string()))
    }

    pub fn read_string_z_array(&mut self) -> Result<Vec<String>> {
        let count = self.read_length()?;
        (0..count).map(|_| self.read_string_z()).collect()
    }

    pub fn read_byte_array(&mut self) -> Result<Vec<u8>> {
        let len = self.read_length()?;
        self.read(len)
    }

    pub fn read_byte_arrays(&mut self) -> Result<Vec<Vec<u8>>> {
        let count = self.read_length()?;
        (0..count).map(|_| self.read_byte_array()).collect()
    }

    pub fn read_i8_array(&mut self) -> Result<Vec<i8>> {
        let count = self.read_length()?;
        (0..count).map(|_| Ok(self.read1()? as i8)).collect()
    }

    pub fn read_i16_array(&mut self) -> Result<Vec<i16>> {
        let count = self.read_length()?;
        (0..count).map(|_| self.read2()).collect()
    }

    pub fn read_i32_array(&mut self) -> Result<Vec<i32>> {
        let count = self.read_length()?;
        (0..count).map(|_| self.read4()).collect()
    }

    pub fn read_i64_array(&mut self) -> Result<Vec<i64>> {
        let count = self.read_length()?;
        (0..count).map(|_| self.read8()).collect()
    }

    pub fn read_u64_array(&mut self) -> Result<Vec<u64>> {
        let count = self.read_length()?;
        (0..count).map(|_| self.read8u()).collect()
    }

    pub fn read_int_array(&mut self, width: u8, signed: bool) -> Result<Vec<i128>> {
        let count = self.read_length()?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let value = match width {
                1 if signed => i128::from(self.read1()? as i8),
                1 => i128::from(self.read1()?),
                2 if signed => i128::from(self.read2()?),
                2 => i128::from(u16::from_le_bytes(self.read_array::<2>()?)),
                4 if signed => i128::from(self.read4()?),
                4 => i128::from(self.read4u()?),
                8 if signed => i128::from(self.read8()?),
                8 => i128::from(self.read8u()?),
                _ => return serialization(format!("unsupported integer width: {width}")),
            };
            out.push(value);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IntX(pub BigInt);

impl IntX {
    pub fn new(value: impl Into<BigInt>) -> Self {
        Self(value.into())
    }

    pub fn is_8_byte_safe(&self) -> bool {
        self.0 >= BigInt::from(i64::MIN) && self.0 <= BigInt::from(i64::MAX)
    }

    pub fn as_bigint(&self) -> &BigInt {
        &self.0
    }
}

impl From<i64> for IntX {
    fn from(value: i64) -> Self {
        Self(BigInt::from(value))
    }
}

impl From<u64> for IntX {
    fn from(value: u64) -> Self {
        Self(BigInt::from(value))
    }
}

impl From<i32> for IntX {
    fn from(value: i32) -> Self {
        Self(BigInt::from(value))
    }
}

impl fmt::Display for IntX {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl CarbonSerializable for IntX {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        // IntX keeps the 8-byte fast path used by Carbon configs and call args,
        // then falls back to the generic compact BigInt encoding for wider IDs.
        if self.is_8_byte_safe() {
            let value = self.0.to_i64().expect("checked i64 range");
            writer.write1(if value < 0 { 0x88 } else { 0x08 });
            writer.write8(value);
            return Ok(());
        }
        writer.write_big_int(&self.0)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let header = reader.read1()?;
        let len = header & 0x3F;
        if len < 8 {
            return serialization("invalid IntX packing");
        }
        if len == 8 {
            let raw = reader.read_array::<8>()?;
            let value = i64::from_le_bytes(raw);
            let header_negative = header & 0x80 != 0;
            if header_negative == (value < 0) {
                return Ok(Self(BigInt::from(value)));
            }
            let fill = if header_negative { 0xFF } else { 0x00 };
            let mut word = [fill; 32];
            word[..8].copy_from_slice(&raw);
            return Ok(Self(signed_word_to_big_int(&word)));
        }
        Ok(Self(reader.read_big_int_with_header(header)?))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TxType {
    Call = 0,
    CallMulti = 1,
    Trade = 2,
    TransferFungible = 3,
    TransferFungibleGasPayer = 4,
    TransferNonFungibleSingle = 5,
    TransferNonFungibleSingleGasPayer = 6,
    TransferNonFungibleMulti = 7,
    TransferNonFungibleMultiGasPayer = 8,
    MintFungible = 9,
    BurnFungible = 10,
    BurnFungibleGasPayer = 11,
    MintNonFungible = 12,
    BurnNonFungible = 13,
    BurnNonFungibleGasPayer = 14,
    Phantasma = 15,
    PhantasmaRaw = 16,
}

impl TryFrom<u8> for TxType {
    type Error = PhantasmaError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Call),
            1 => Ok(Self::CallMulti),
            2 => Ok(Self::Trade),
            3 => Ok(Self::TransferFungible),
            4 => Ok(Self::TransferFungibleGasPayer),
            5 => Ok(Self::TransferNonFungibleSingle),
            6 => Ok(Self::TransferNonFungibleSingleGasPayer),
            7 => Ok(Self::TransferNonFungibleMulti),
            8 => Ok(Self::TransferNonFungibleMultiGasPayer),
            9 => Ok(Self::MintFungible),
            10 => Ok(Self::BurnFungible),
            11 => Ok(Self::BurnFungibleGasPayer),
            12 => Ok(Self::MintNonFungible),
            13 => Ok(Self::BurnNonFungible),
            14 => Ok(Self::BurnNonFungibleGasPayer),
            15 => Ok(Self::Phantasma),
            16 => Ok(Self::PhantasmaRaw),
            _ => serialization(format!("unsupported transaction type: {value}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ModuleId {
    Governance = 0,
    Token = 1,
    Phantasma = 2,
    Org = 3,
    Market = 4,
    Internal = 0xFFFF_FFFF,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TokenContractMethod {
    TransferFungible = 0,
    TransferNonFungible = 1,
    CreateToken = 2,
    MintFungible = 3,
    BurnFungible = 4,
    GetBalance = 5,
    CreateTokenSeries = 6,
    DeleteTokenSeries = 7,
    MintNonFungible = 8,
    BurnNonFungible = 9,
    GetInstances = 10,
    GetNonFungibleInfo = 11,
    GetNonFungibleInfoByRomId = 12,
    GetSeriesInfo = 13,
    GetSeriesInfoByMetaId = 14,
    GetTokenInfo = 15,
    GetTokenInfoBySymbol = 16,
    GetTokenSupply = 17,
    GetSeriesSupply = 18,
    GetTokenIdBySymbol = 19,
    GetBalances = 20,
    CreateMintedTokenSeries = 21,
    ApplyInflation = 22,
    UpdateTokenMetadata = 23,
    GetNextTokenInflation = 24,
    SetTokensConfig = 25,
    UpdateSeriesMetadata = 26,
    MintPhantasmaNonFungible = 27,
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct TokenFlags: u8 {
        const NONE = 0;
        const BIG_FUNGIBLE = 1 << 0;
        const NON_FUNGIBLE = 1 << 1;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct TokensConfigFlags: u8 {
        const NONE = 0;
        const REQUIRE_METADATA = 1 << 0;
        const REQUIRE_SYMBOL = 1 << 1;
        const REQUIRE_NFT_META_ID = 1 << 2;
        const REQUIRE_NFT_STANDARD = 1 << 3;
        const ALLOW_EXPLICIT_NFT_META_ID_MINT = 1 << 4;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct MarketConfigFlags: u32 {
        const NONE = 0;
        const PRICE_REQUIRED = 1 << 0;
        const ENFORCE_ROYALTIES = 1 << 1;
        const CAN_CANCEL_EARLY = 1 << 2;
        const CAN_PURCHASE_LATE = 1 << 3;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct VMStructFlags: u8 {
        const NONE = 0;
        const DYNAMIC_EXTRAS = 1 << 0;
        const IS_SORTED = 1 << 1;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ListingType {
    FixedPrice = 0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum MarketContractMethod {
    SellToken = 0,
    SellTokenById = 1,
    CancelSale = 2,
    CancelSaleById = 3,
    BuyToken = 4,
    BuyTokenById = 5,
    GetTokenListingCount = 6,
    GetTokenListingInfo = 7,
    GetTokenListingInfoById = 8,
}

pub const MARKET_MINIMUM_LISTING_TIME_MS: u64 = 1_000;
pub const MARKET_MAXIMUM_LISTING_TIME_MS: u64 = 1_000 * 60 * 60 * 24 * 90;
pub const MARKET_DELISTING_GRACE_MS: u64 = 1_000 * 60 * 60 * 24;
pub const MARKET_ROYALTY_ONE_PERCENT: u64 = 10_000_000;
pub const MARKET_ROYALTY_HUNDRED_PERCENT: u64 = 100 * MARKET_ROYALTY_ONE_PERCENT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VMType {
    Dynamic,
    Array,
    Bytes,
    Struct,
    Int8,
    Int16,
    Int32,
    Int64,
    Int256,
    Bytes16,
    Bytes32,
    Bytes64,
    String,
    ArrayDynamic,
    ArrayBytes,
    ArrayStruct,
    ArrayInt8,
    ArrayInt16,
    ArrayInt32,
    ArrayInt64,
    ArrayInt256,
    ArrayBytes16,
    ArrayBytes32,
    ArrayBytes64,
    ArrayString,
}

impl VMType {
    pub fn code(self) -> u8 {
        match self {
            Self::Dynamic => 0,
            Self::Array | Self::ArrayDynamic => 1,
            Self::Bytes => 2,
            Self::Struct => 4,
            Self::Int8 => 6,
            Self::Int16 => 8,
            Self::Int32 => 10,
            Self::Int64 => 12,
            Self::Int256 => 14,
            Self::Bytes16 => 16,
            Self::Bytes32 => 18,
            Self::Bytes64 => 20,
            Self::String => 22,
            Self::ArrayBytes => 3,
            Self::ArrayStruct => 5,
            Self::ArrayInt8 => 7,
            Self::ArrayInt16 => 9,
            Self::ArrayInt32 => 11,
            Self::ArrayInt64 => 13,
            Self::ArrayInt256 => 15,
            Self::ArrayBytes16 => 17,
            Self::ArrayBytes32 => 19,
            Self::ArrayBytes64 => 21,
            Self::ArrayString => 23,
        }
    }
}

impl TryFrom<u8> for VMType {
    type Error = PhantasmaError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Dynamic),
            1 => Ok(Self::ArrayDynamic),
            2 => Ok(Self::Bytes),
            4 => Ok(Self::Struct),
            6 => Ok(Self::Int8),
            8 => Ok(Self::Int16),
            10 => Ok(Self::Int32),
            12 => Ok(Self::Int64),
            14 => Ok(Self::Int256),
            16 => Ok(Self::Bytes16),
            18 => Ok(Self::Bytes32),
            20 => Ok(Self::Bytes64),
            22 => Ok(Self::String),
            3 => Ok(Self::ArrayBytes),
            5 => Ok(Self::ArrayStruct),
            7 => Ok(Self::ArrayInt8),
            9 => Ok(Self::ArrayInt16),
            11 => Ok(Self::ArrayInt32),
            13 => Ok(Self::ArrayInt64),
            15 => Ok(Self::ArrayInt256),
            17 => Ok(Self::ArrayBytes16),
            19 => Ok(Self::ArrayBytes32),
            21 => Ok(Self::ArrayBytes64),
            23 => Ok(Self::ArrayString),
            _ => serialization(format!("unsupported VM dynamic type: {value}")),
        }
    }
}

pub fn bytes32_from_public_key(public_key: &[u8]) -> Result<Bytes32> {
    if public_key.len() != 32 {
        return crypto(format!(
            "public key length must be 32, got {}",
            public_key.len()
        ));
    }
    Bytes32::try_from_slice(public_key)
}

pub fn bytes32_from_phantasma_address(address: &Address) -> Result<Bytes32> {
    if !matches!(address.kind(), AddressKind::User | AddressKind::System) {
        return crypto(format!("unsupported address kind {:?}", address.kind()));
    }
    Bytes32::try_from_slice(&address.data()[2..])
}

pub fn bytes32_from_phantasma_address_text(text: &str) -> Result<Bytes32> {
    bytes32_from_phantasma_address(&Address::from_text(text)?)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VMVariableSchema {
    pub vm_type: VMType,
    pub struct_def: Option<VMStructSchema>,
}

impl VMVariableSchema {
    pub fn new(vm_type: VMType) -> Self {
        Self {
            vm_type,
            struct_def: None,
        }
    }

    pub fn with_struct(vm_type: VMType, struct_def: VMStructSchema) -> Self {
        Self {
            vm_type,
            struct_def: Some(struct_def),
        }
    }
}

impl CarbonSerializable for VMVariableSchema {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write1(self.vm_type.code());
        if matches!(self.vm_type, VMType::Struct | VMType::ArrayStruct) {
            self.struct_def
                .clone()
                .unwrap_or_default()
                .write_carbon(writer)?;
        }
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let vm_type = VMType::try_from(reader.read1()?)?;
        let struct_def = if matches!(vm_type, VMType::Struct | VMType::ArrayStruct) {
            Some(VMStructSchema::read_carbon(reader)?)
        } else {
            None
        };
        Ok(Self {
            vm_type,
            struct_def,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VMNamedVariableSchema {
    pub name: SmallString,
    pub schema: VMVariableSchema,
}

impl VMNamedVariableSchema {
    pub fn new(name: impl Into<SmallString>, vm_type: VMType) -> Self {
        Self {
            name: name.into(),
            schema: VMVariableSchema::new(vm_type),
        }
    }

    pub fn make(name: impl Into<SmallString>, vm_type: VMType) -> Self {
        Self::new(name, vm_type)
    }

    pub fn with_struct(
        name: impl Into<SmallString>,
        vm_type: VMType,
        struct_def: VMStructSchema,
    ) -> Self {
        Self {
            name: name.into(),
            schema: VMVariableSchema::with_struct(vm_type, struct_def),
        }
    }

    pub fn make_with_struct(
        name: impl Into<SmallString>,
        vm_type: VMType,
        struct_def: VMStructSchema,
    ) -> Self {
        Self::with_struct(name, vm_type, struct_def)
    }
}

impl CarbonSerializable for VMNamedVariableSchema {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.name.write_carbon(writer)?;
        self.schema.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            name: SmallString::read_carbon(reader)?,
            schema: VMVariableSchema::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VMStructSchema {
    pub fields: Vec<VMNamedVariableSchema>,
    pub flags: VMStructFlags,
}

impl VMStructSchema {
    pub fn new(fields: Vec<VMNamedVariableSchema>) -> Self {
        Self {
            fields,
            flags: VMStructFlags::NONE,
        }
    }

    pub fn with_flags(fields: Vec<VMNamedVariableSchema>, flags: VMStructFlags) -> Self {
        Self { fields, flags }
    }
}

impl CarbonSerializable for VMStructSchema {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write4(ensure_u32_len(self.fields.len())? as i32);
        for item in &self.fields {
            item.write_carbon(writer)?;
        }
        writer.write1(self.flags.bits());
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let count = reader.read_length()?;
        let mut fields = Vec::with_capacity(count);
        for _ in 0..count {
            fields.push(VMNamedVariableSchema::read_carbon(reader)?);
        }
        Ok(Self {
            fields,
            flags: VMStructFlags::from_bits_retain(reader.read1()?),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VMValue {
    None,
    Dynamic(Box<VMDynamicVariable>),
    Bytes(Vec<u8>),
    Struct(VMDynamicStruct),
    Int(i64),
    Int256(BigInt),
    Bytes16(Bytes16),
    Bytes32(Bytes32),
    Bytes64(Bytes64),
    String(String),
    ArrayDynamic(Vec<VMDynamicVariable>),
    ArrayBytes(Vec<Vec<u8>>),
    ArrayStruct(VMStructArray),
    ArrayInt8(Vec<i8>),
    ArrayInt16(Vec<i16>),
    ArrayInt32(Vec<i32>),
    ArrayInt64(Vec<i64>),
    ArrayInt256(Vec<BigInt>),
    ArrayBytes16(Vec<Bytes16>),
    ArrayBytes32(Vec<Bytes32>),
    ArrayBytes64(Vec<Bytes64>),
    ArrayString(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VMDynamicVariable {
    pub vm_type: VMType,
    pub data: VMValue,
}

impl VMDynamicVariable {
    pub fn new(vm_type: VMType, data: VMValue) -> Self {
        Self { vm_type, data }
    }

    pub fn string(value: impl Into<String>) -> Self {
        Self::new(VMType::String, VMValue::String(value.into()))
    }

    pub fn int32(value: i32) -> Self {
        Self::new(VMType::Int32, VMValue::Int(i64::from(value)))
    }

    pub fn int64(value: i64) -> Self {
        Self::new(VMType::Int64, VMValue::Int(value))
    }

    pub fn int256(value: impl Into<BigInt>) -> Self {
        Self::new(VMType::Int256, VMValue::Int256(value.into()))
    }

    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::new(VMType::Bytes, VMValue::Bytes(value.into()))
    }

    pub fn write_static(
        &self,
        vm_type: VMType,
        schema: Option<&VMStructSchema>,
        writer: &mut CarbonWriter,
    ) -> Result<bool> {
        match vm_type {
            VMType::Dynamic => {
                let inner = match &self.data {
                    VMValue::Dynamic(value) => value.as_ref().clone(),
                    _ => VMDynamicVariable::new(
                        VMType::ArrayDynamic,
                        VMValue::ArrayDynamic(Vec::new()),
                    ),
                };
                inner.write_carbon(writer)?;
            }
            VMType::Bytes => writer.write_byte_array(match &self.data {
                VMValue::Bytes(value) => value.as_slice(),
                _ => &[],
            })?,
            VMType::Struct => {
                let structure = match &self.data {
                    VMValue::Struct(value) => value.clone(),
                    _ => VMDynamicStruct::default(),
                };
                return match schema {
                    Some(schema) => structure.write_with_schema(schema, writer),
                    None => {
                        structure.write_carbon(writer)?;
                        Ok(true)
                    }
                };
            }
            VMType::Int8 => writer.write1(self.as_i64() as i8 as u8),
            VMType::Int16 => writer.write2(self.as_i64() as i16),
            VMType::Int32 => writer.write4(self.as_i64() as i32),
            VMType::Int64 => writer.write8(self.as_i64()),
            VMType::Int256 => writer.write_big_int(&self.as_bigint())?,
            VMType::Bytes16 => writer.write16(match &self.data {
                VMValue::Bytes16(value) => *value,
                _ => EMPTY_BYTES16,
            }),
            VMType::Bytes32 => writer.write32(match &self.data {
                VMValue::Bytes32(value) => *value,
                _ => EMPTY_BYTES32,
            }),
            VMType::Bytes64 => writer.write64(match &self.data {
                VMValue::Bytes64(value) => *value,
                _ => EMPTY_BYTES64,
            }),
            VMType::String => writer.write_string_z(match &self.data {
                VMValue::String(value) => value,
                _ => "",
            })?,
            VMType::ArrayDynamic => match &self.data {
                VMValue::ArrayDynamic(values) => write_dynamic_variables(writer, values)?,
                _ => write_dynamic_variables(writer, &[])?,
            },
            VMType::ArrayBytes => {
                if let VMValue::ArrayBytes(values) = &self.data {
                    writer.write_byte_arrays(values)?;
                } else {
                    writer.write_byte_arrays(&[])?;
                }
            }
            VMType::ArrayStruct => {
                let array = match &self.data {
                    VMValue::ArrayStruct(value) => value.clone(),
                    _ => VMStructArray::default(),
                };
                let used_schema = schema.or(Some(&array.schema));
                writer.write4(ensure_u32_len(array.structs.len())? as i32);
                if schema.is_none() {
                    array.schema.write_carbon(writer)?;
                }
                for item in &array.structs {
                    if let Some(schema) = used_schema {
                        item.write_with_schema(schema, writer)?;
                    } else {
                        item.write_carbon(writer)?;
                    }
                }
            }
            VMType::ArrayInt8 => {
                if let VMValue::ArrayInt8(values) = &self.data {
                    writer.write_i8_array(values)?;
                } else {
                    writer.write_i8_array(&[])?;
                }
            }
            VMType::ArrayInt16 => {
                if let VMValue::ArrayInt16(values) = &self.data {
                    writer.write_i16_array(values)?;
                } else {
                    writer.write_i16_array(&[])?;
                }
            }
            VMType::ArrayInt32 => {
                if let VMValue::ArrayInt32(values) = &self.data {
                    writer.write_i32_array(values)?;
                } else {
                    writer.write_i32_array(&[])?;
                }
            }
            VMType::ArrayInt64 => {
                if let VMValue::ArrayInt64(values) = &self.data {
                    writer.write_i64_array(values)?;
                } else {
                    writer.write_i64_array(&[])?;
                }
            }
            VMType::ArrayInt256 => {
                if let VMValue::ArrayInt256(values) = &self.data {
                    writer.write_big_int_array(values)?;
                } else {
                    writer.write_big_int_array(&[])?;
                }
            }
            VMType::ArrayBytes16 => {
                let values = match &self.data {
                    VMValue::ArrayBytes16(values) => values.as_slice(),
                    _ => &[],
                };
                writer.write4(ensure_u32_len(values.len())? as i32);
                for value in values {
                    writer.write16(*value);
                }
            }
            VMType::ArrayBytes32 => {
                let values = match &self.data {
                    VMValue::ArrayBytes32(values) => values.as_slice(),
                    _ => &[],
                };
                writer.write4(ensure_u32_len(values.len())? as i32);
                for value in values {
                    writer.write32(*value);
                }
            }
            VMType::ArrayBytes64 => {
                let values = match &self.data {
                    VMValue::ArrayBytes64(values) => values.as_slice(),
                    _ => &[],
                };
                writer.write4(ensure_u32_len(values.len())? as i32);
                for value in values {
                    writer.write64(*value);
                }
            }
            VMType::ArrayString => {
                if let VMValue::ArrayString(values) = &self.data {
                    writer.write_string_z_array(values)?;
                } else {
                    writer.write_string_z_array(&[])?;
                }
            }
            VMType::Array => return serialization("unsupported VM dynamic type: Array"),
        }
        Ok(true)
    }

    pub fn read_static(
        vm_type: VMType,
        schema: Option<&VMStructSchema>,
        reader: &mut CarbonReader<'_>,
    ) -> Result<VMValue> {
        Ok(match vm_type {
            VMType::Dynamic => VMValue::Dynamic(Box::new(Self::read_carbon(reader)?)),
            VMType::Bytes => VMValue::Bytes(reader.read_byte_array()?),
            VMType::Struct => VMValue::Struct(match schema {
                Some(schema) => VMDynamicStruct::read_with_schema(schema, reader)?,
                None => VMDynamicStruct::read_carbon(reader)?,
            }),
            VMType::Int8 => VMValue::Int(reader.read1()? as i8 as i64),
            VMType::Int16 => VMValue::Int(i64::from(reader.read2()?)),
            VMType::Int32 => VMValue::Int(i64::from(reader.read4()?)),
            VMType::Int64 => VMValue::Int(reader.read8()?),
            VMType::Int256 => VMValue::Int256(reader.read_big_int()?),
            VMType::Bytes16 => VMValue::Bytes16(reader.read16()?),
            VMType::Bytes32 => VMValue::Bytes32(reader.read32()?),
            VMType::Bytes64 => VMValue::Bytes64(reader.read64()?),
            VMType::String => VMValue::String(reader.read_string_z()?),
            VMType::ArrayDynamic => VMValue::ArrayDynamic(read_dynamic_variables(reader)?),
            VMType::ArrayBytes => VMValue::ArrayBytes(reader.read_byte_arrays()?),
            VMType::ArrayStruct => {
                let count = reader.read_length()?;
                let owned_schema;
                let used_schema = if let Some(schema) = schema {
                    schema
                } else {
                    owned_schema = VMStructSchema::read_carbon(reader)?;
                    &owned_schema
                };
                let mut structs = Vec::with_capacity(count);
                for _ in 0..count {
                    structs.push(VMDynamicStruct::read_with_schema(used_schema, reader)?);
                }
                VMValue::ArrayStruct(VMStructArray {
                    schema: used_schema.clone(),
                    structs,
                })
            }
            VMType::ArrayInt8 => VMValue::ArrayInt8(reader.read_i8_array()?),
            VMType::ArrayInt16 => VMValue::ArrayInt16(reader.read_i16_array()?),
            VMType::ArrayInt32 => VMValue::ArrayInt32(reader.read_i32_array()?),
            VMType::ArrayInt64 => VMValue::ArrayInt64(reader.read_i64_array()?),
            VMType::ArrayInt256 => VMValue::ArrayInt256(reader.read_big_int_array()?),
            VMType::ArrayBytes16 => {
                let count = reader.read_length()?;
                let mut out = Vec::with_capacity(count);
                for _ in 0..count {
                    out.push(reader.read16()?);
                }
                VMValue::ArrayBytes16(out)
            }
            VMType::ArrayBytes32 => {
                let count = reader.read_length()?;
                let mut out = Vec::with_capacity(count);
                for _ in 0..count {
                    out.push(reader.read32()?);
                }
                VMValue::ArrayBytes32(out)
            }
            VMType::ArrayBytes64 => {
                let count = reader.read_length()?;
                let mut out = Vec::with_capacity(count);
                for _ in 0..count {
                    out.push(reader.read64()?);
                }
                VMValue::ArrayBytes64(out)
            }
            VMType::ArrayString => VMValue::ArrayString(reader.read_string_z_array()?),
            VMType::Array => return serialization("unsupported VM dynamic type: Array"),
        })
    }

    fn as_i64(&self) -> i64 {
        match &self.data {
            VMValue::Int(value) => *value,
            VMValue::Int256(value) => value.to_i64().unwrap_or(0),
            _ => 0,
        }
    }

    fn as_bigint(&self) -> BigInt {
        match &self.data {
            VMValue::Int(value) => BigInt::from(*value),
            VMValue::Int256(value) => value.clone(),
            _ => BigInt::zero(),
        }
    }
}

impl CarbonSerializable for VMDynamicVariable {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write1(self.vm_type.code());
        self.write_static(self.vm_type, None, writer)?;
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let vm_type = VMType::try_from(reader.read1()?)?;
        let data = Self::read_static(vm_type, None, reader)?;
        Ok(Self { vm_type, data })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VMNamedDynamicVariable {
    pub name: SmallString,
    pub value: VMDynamicVariable,
}

impl VMNamedDynamicVariable {
    pub fn new(name: impl Into<SmallString>, value: VMDynamicVariable) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    pub fn make(name: impl Into<SmallString>, vm_type: VMType, value: VMValue) -> Self {
        Self::new(name, VMDynamicVariable::new(vm_type, value))
    }
}

impl CarbonSerializable for VMNamedDynamicVariable {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.name.write_carbon(writer)?;
        self.value.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            name: SmallString::read_carbon(reader)?,
            value: VMDynamicVariable::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VMDynamicStruct {
    pub fields: Vec<VMNamedDynamicVariable>,
}

impl VMDynamicStruct {
    pub fn new(fields: Vec<VMNamedDynamicVariable>) -> Self {
        let mut out = Self { fields };
        out.sort();
        out
    }

    pub fn sort(&mut self) {
        self.fields
            .sort_by(|left, right| left.name.0.cmp(&right.name.0));
    }

    pub fn get(&self, name: &str) -> Option<&VMDynamicVariable> {
        self.fields
            .iter()
            .find(|field| field.name.0 == name)
            .map(|field| &field.value)
    }

    pub fn write_with_schema(
        &self,
        schema: &VMStructSchema,
        writer: &mut CarbonWriter,
    ) -> Result<bool> {
        let mut ok = true;
        let mut found = 0usize;
        for schema_field in &schema.fields {
            let field = self.get(&schema_field.name.0);
            let value = field
                .cloned()
                .unwrap_or_else(|| default_dynamic_variable(schema_field.schema.vm_type));
            if field.is_some() {
                found += 1;
            }
            ok = value.write_static(
                schema_field.schema.vm_type,
                schema_field.schema.struct_def.as_ref(),
                writer,
            )? && ok;
        }

        if !schema.flags.contains(VMStructFlags::DYNAMIC_EXTRAS) {
            return Ok(ok);
        }

        let extras: Vec<_> = self
            .fields
            .iter()
            .filter(|field| {
                !schema
                    .fields
                    .iter()
                    .any(|schema_field| schema_field.name == field.name)
            })
            .cloned()
            .collect();
        if found == schema.fields.len() && extras.is_empty() {
            writer.write4u(0);
            return Ok(ok);
        }
        write_named_dynamic_variables(writer, &extras)?;
        Ok(ok)
    }

    pub fn read_with_schema(
        schema: &VMStructSchema,
        reader: &mut CarbonReader<'_>,
    ) -> Result<Self> {
        let mut fields = Vec::new();
        for schema_field in &schema.fields {
            let data = VMDynamicVariable::read_static(
                schema_field.schema.vm_type,
                schema_field.schema.struct_def.as_ref(),
                reader,
            )?;
            fields.push(VMNamedDynamicVariable {
                name: schema_field.name.clone(),
                value: VMDynamicVariable {
                    vm_type: schema_field.schema.vm_type,
                    data,
                },
            });
        }
        if schema.flags.contains(VMStructFlags::DYNAMIC_EXTRAS) {
            fields.extend(read_named_dynamic_variables(reader)?);
        }
        Ok(Self::new(fields))
    }
}

impl CarbonSerializable for VMDynamicStruct {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        let mut cloned = self.clone();
        cloned.sort();
        write_named_dynamic_variables(writer, &cloned.fields)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self::new(read_named_dynamic_variables(reader)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VMStructArray {
    pub schema: VMStructSchema,
    pub structs: Vec<VMDynamicStruct>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenSchemas {
    pub series_metadata: VMStructSchema,
    pub rom: VMStructSchema,
    pub ram: VMStructSchema,
}

impl CarbonSerializable for TokenSchemas {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.series_metadata.write_carbon(writer)?;
        self.rom.write_carbon(writer)?;
        self.ram.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            series_metadata: VMStructSchema::read_carbon(reader)?,
            rom: VMStructSchema::read_carbon(reader)?,
            ram: VMStructSchema::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenSchemaField {
    pub name: String,
    pub vm_type: VMType,
}

impl TokenSchemaField {
    pub fn new(name: impl Into<String>, vm_type: VMType) -> Self {
        Self {
            name: name.into(),
            vm_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokenSchemasJson {
    pub series_metadata: Vec<TokenSchemaField>,
    pub rom: Vec<TokenSchemaField>,
    pub ram: Vec<TokenSchemaField>,
}

#[allow(clippy::upper_case_acronyms)]
pub type TokenSchemasJSON = TokenSchemasJson;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChainConfig {
    pub version: u8,
    pub reserved1: u8,
    pub reserved2: u8,
    pub reserved3: u8,
    pub allowed_tx_types: u32,
    pub expiry_window: u32,
    pub block_rate_target: u32,
}

impl CarbonSerializable for ChainConfig {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write1(self.version);
        writer.write1(self.reserved1);
        writer.write1(self.reserved2);
        writer.write1(self.reserved3);
        writer.write4u(self.allowed_tx_types);
        writer.write4u(self.expiry_window);
        writer.write4u(self.block_rate_target);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            version: reader.read1()?,
            reserved1: reader.read1()?,
            reserved2: reader.read1()?,
            reserved3: reader.read1()?,
            allowed_tx_types: reader.read4u()?,
            expiry_window: reader.read4u()?,
            block_rate_target: reader.read4u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GasConfig {
    pub version: u8,
    pub max_name_length: u8,
    pub max_token_symbol_length: u8,
    pub fee_shift: u8,
    pub max_structure_size: u32,
    pub fee_multiplier: u64,
    pub gas_token_id: u64,
    pub data_token_id: u64,
    pub minimum_gas_offer: u64,
    pub data_escrow_per_row: u64,
    pub gas_fee_transfer: u64,
    pub gas_fee_query: u64,
    pub gas_fee_create_token_base: u64,
    pub gas_fee_create_token_symbol: u64,
    pub gas_fee_create_token_series: u64,
    pub gas_fee_per_byte: u64,
    pub gas_fee_register_name: u64,
    pub gas_burn_ratio_mul: u64,
    pub gas_burn_ratio_shift: u8,
}

impl CarbonSerializable for GasConfig {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write1(self.version);
        writer.write1(self.max_name_length);
        writer.write1(self.max_token_symbol_length);
        writer.write1(self.fee_shift);
        writer.write4u(self.max_structure_size);
        writer.write8u(self.fee_multiplier);
        writer.write8u(self.gas_token_id);
        writer.write8u(self.data_token_id);
        writer.write8u(self.minimum_gas_offer);
        writer.write8u(self.data_escrow_per_row);
        writer.write8u(self.gas_fee_transfer);
        writer.write8u(self.gas_fee_query);
        writer.write8u(self.gas_fee_create_token_base);
        writer.write8u(self.gas_fee_create_token_symbol);
        writer.write8u(self.gas_fee_create_token_series);
        writer.write8u(self.gas_fee_per_byte);
        writer.write8u(self.gas_fee_register_name);
        writer.write8u(self.gas_burn_ratio_mul);
        writer.write1(self.gas_burn_ratio_shift);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            version: reader.read1()?,
            max_name_length: reader.read1()?,
            max_token_symbol_length: reader.read1()?,
            fee_shift: reader.read1()?,
            max_structure_size: reader.read4u()?,
            fee_multiplier: reader.read8u()?,
            gas_token_id: reader.read8u()?,
            data_token_id: reader.read8u()?,
            minimum_gas_offer: reader.read8u()?,
            data_escrow_per_row: reader.read8u()?,
            gas_fee_transfer: reader.read8u()?,
            gas_fee_query: reader.read8u()?,
            gas_fee_create_token_base: reader.read8u()?,
            gas_fee_create_token_symbol: reader.read8u()?,
            gas_fee_create_token_series: reader.read8u()?,
            gas_fee_per_byte: reader.read8u()?,
            gas_fee_register_name: reader.read8u()?,
            gas_burn_ratio_mul: reader.read8u()?,
            gas_burn_ratio_shift: reader.read1()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TokensConfig {
    pub flags: TokensConfigFlags,
}

impl CarbonSerializable for TokensConfig {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write1(self.flags.bits());
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            flags: TokensConfigFlags::from_bits_retain(reader.read1()?),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenInfo {
    pub max_supply: IntX,
    pub flags: TokenFlags,
    pub decimals: u8,
    pub owner: Bytes32,
    pub symbol: SmallString,
    pub metadata: Vec<u8>,
    pub token_schemas: Vec<u8>,
}

impl CarbonSerializable for TokenInfo {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.max_supply.write_carbon(writer)?;
        writer.write1(self.flags.bits());
        writer.write1(self.decimals);
        writer.write32(self.owner);
        self.symbol.write_carbon(writer)?;
        writer.write_byte_array(&self.metadata)?;
        if self.flags.contains(TokenFlags::NON_FUNGIBLE) {
            writer.write_byte_array(&self.token_schemas)?;
        }
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let max_supply = IntX::read_carbon(reader)?;
        let flags = TokenFlags::from_bits_retain(reader.read1()?);
        let decimals = reader.read1()?;
        let owner = reader.read32()?;
        let symbol = SmallString::read_carbon(reader)?;
        let metadata = reader.read_byte_array()?;
        let token_schemas = if flags.contains(TokenFlags::NON_FUNGIBLE) {
            reader.read_byte_array()?
        } else {
            Vec::new()
        };
        Ok(Self {
            max_supply,
            flags,
            decimals,
            owner,
            symbol,
            metadata,
            token_schemas,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SeriesInfo {
    pub max_mint: u32,
    pub max_supply: u32,
    pub owner: Bytes32,
    pub metadata: Vec<u8>,
    pub rom: VMStructSchema,
    pub ram: VMStructSchema,
}

impl CarbonSerializable for SeriesInfo {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write4u(self.max_mint);
        writer.write4u(self.max_supply);
        writer.write32(self.owner);
        writer.write_byte_array(&self.metadata)?;
        self.rom.write_carbon(writer)?;
        self.ram.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            max_mint: reader.read4u()?,
            max_supply: reader.read4u()?,
            owner: reader.read32()?,
            metadata: reader.read_byte_array()?,
            rom: VMStructSchema::read_carbon(reader)?,
            ram: VMStructSchema::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NFTMintInfo {
    pub series_id: u32,
    pub rom: Vec<u8>,
    pub ram: Vec<u8>,
}

impl CarbonSerializable for NFTMintInfo {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write4u(self.series_id);
        writer.write_byte_array(&self.rom)?;
        writer.write_byte_array(&self.ram)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            series_id: reader.read4u()?,
            rom: reader.read_byte_array()?,
            ram: reader.read_byte_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MintNonFungibleArgs {
    pub token_id: u64,
    pub address: Bytes32,
    pub tokens: Vec<NFTMintInfo>,
}

impl CarbonSerializable for MintNonFungibleArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.address);
        write_carbon_array(writer, &self.tokens)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            address: reader.read32()?,
            tokens: read_carbon_array(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CreateTokenSeriesArgs {
    pub token_id: u64,
    pub info: SeriesInfo,
}

impl CarbonSerializable for CreateTokenSeriesArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        self.info.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            info: SeriesInfo::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CreateMintedTokenSeriesArgs {
    pub token_id: u64,
    pub info: SeriesInfo,
    pub address: Bytes32,
    pub roms: Vec<Vec<u8>>,
    pub rams: Vec<Vec<u8>>,
}

impl CarbonSerializable for CreateMintedTokenSeriesArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        self.info.write_carbon(writer)?;
        writer.write32(self.address);
        writer.write_byte_arrays(&self.roms)?;
        writer.write_byte_arrays(&self.rams)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            info: SeriesInfo::read_carbon(reader)?,
            address: reader.read32()?,
            roms: reader.read_byte_arrays()?,
            rams: reader.read_byte_arrays()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhantasmaNFTMintInfo {
    pub phantasma_series_id: IntX,
    pub rom: Vec<u8>,
    pub ram: Vec<u8>,
}

impl CarbonSerializable for PhantasmaNFTMintInfo {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.phantasma_series_id.write_carbon(writer)?;
        writer.write_byte_array(&self.rom)?;
        writer.write_byte_array(&self.ram)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            phantasma_series_id: IntX::read_carbon(reader)?,
            rom: reader.read_byte_array()?,
            ram: reader.read_byte_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MintPhantasmaNonFungibleArgs {
    pub token_id: u64,
    pub address: Bytes32,
    pub tokens: Vec<PhantasmaNFTMintInfo>,
}

impl CarbonSerializable for MintPhantasmaNonFungibleArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.address);
        write_carbon_array(writer, &self.tokens)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            address: reader.read32()?,
            tokens: read_carbon_array(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhantasmaNFTMintResult {
    pub phantasma_nft_id: Bytes32,
    pub carbon_instance_id: u64,
}

impl CarbonSerializable for PhantasmaNFTMintResult {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.phantasma_nft_id);
        writer.write8u(self.carbon_instance_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            phantasma_nft_id: reader.read32()?,
            carbon_instance_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MintFungibleArgs {
    pub token_id: u64,
    pub to: Bytes32,
    pub amount: IntX,
}

impl CarbonSerializable for MintFungibleArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.to);
        self.amount.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            to: reader.read32()?,
            amount: IntX::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransferFungibleArgs {
    pub to: Bytes32,
    pub from_address: Bytes32,
    pub token_id: u64,
    pub amount: IntX,
}

impl CarbonSerializable for TransferFungibleArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.to);
        writer.write32(self.from_address);
        writer.write8u(self.token_id);
        self.amount.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            to: reader.read32()?,
            from_address: reader.read32()?,
            token_id: reader.read8u()?,
            amount: IntX::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TransferNonFungibleArgs {
    pub to: Bytes32,
    pub from_address: Bytes32,
    pub token_id: u64,
    pub instance_ids: Vec<u64>,
}

impl CarbonSerializable for TransferNonFungibleArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.to);
        writer.write32(self.from_address);
        writer.write8u(self.token_id);
        writer.write_u64_array(&self.instance_ids)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            to: reader.read32()?,
            from_address: reader.read32()?,
            token_id: reader.read8u()?,
            instance_ids: reader.read_u64_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BurnFungibleArgs {
    pub token_id: u64,
    pub from_address: Bytes32,
    pub amount: IntX,
}

impl CarbonSerializable for BurnFungibleArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.from_address);
        self.amount.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            from_address: reader.read32()?,
            amount: IntX::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BurnNonFungibleArgs {
    pub token_id: u64,
    pub from_address: Bytes32,
    pub instance_ids: Vec<u64>,
}

impl CarbonSerializable for BurnNonFungibleArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.from_address);
        writer.write_u64_array(&self.instance_ids)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            from_address: reader.read32()?,
            instance_ids: reader.read_u64_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateTokenMetadataArgs {
    pub token_id: u64,
    pub metadata: VMDynamicStruct,
}

impl CarbonSerializable for UpdateTokenMetadataArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        self.metadata.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            metadata: VMDynamicStruct::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateSeriesMetadataArgs {
    pub token_id: u64,
    pub series_id: u32,
    pub metadata: Vec<u8>,
}

impl CarbonSerializable for UpdateSeriesMetadataArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write4u(self.series_id);
        writer.write_byte_array(&self.metadata)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            series_id: reader.read4u()?,
            metadata: reader.read_byte_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenListing {
    pub listing_type: ListingType,
    pub seller: Bytes32,
    pub quote_token_id: u64,
    pub price: IntX,
    pub start_date: i64,
    pub end_date: i64,
}

impl Default for TokenListing {
    fn default() -> Self {
        Self {
            listing_type: ListingType::FixedPrice,
            seller: EMPTY_BYTES32,
            quote_token_id: 0,
            price: IntX::default(),
            start_date: 0,
            end_date: 0,
        }
    }
}

impl CarbonSerializable for TokenListing {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write1(self.listing_type as u8);
        writer.write32(self.seller);
        writer.write8u(self.quote_token_id);
        self.price.write_carbon(writer)?;
        writer.write8(self.start_date);
        writer.write8(self.end_date);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let listing_type = match reader.read1()? {
            0 => ListingType::FixedPrice,
            value => return serialization(format!("unsupported listing type: {value}")),
        };
        Ok(Self {
            listing_type,
            seller: reader.read32()?,
            quote_token_id: reader.read8u()?,
            price: IntX::read_carbon(reader)?,
            start_date: reader.read8()?,
            end_date: reader.read8()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketConfig {
    pub minimum_listing_time: u64,
    pub maximum_listing_time: u64,
    pub delisting_grace: u64,
    pub flags: MarketConfigFlags,
}

impl Default for MarketConfig {
    fn default() -> Self {
        Self {
            minimum_listing_time: MARKET_MINIMUM_LISTING_TIME_MS,
            maximum_listing_time: MARKET_MAXIMUM_LISTING_TIME_MS,
            delisting_grace: MARKET_DELISTING_GRACE_MS,
            flags: MarketConfigFlags::PRICE_REQUIRED | MarketConfigFlags::ENFORCE_ROYALTIES,
        }
    }
}

impl CarbonSerializable for MarketConfig {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.minimum_listing_time);
        writer.write8u(self.maximum_listing_time);
        writer.write8u(self.delisting_grace);
        writer.write4u(self.flags.bits());
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            minimum_listing_time: reader.read8u()?,
            maximum_listing_time: reader.read8u()?,
            delisting_grace: reader.read8u()?,
            flags: MarketConfigFlags::from_bits_retain(reader.read4u()?),
        })
    }
}

pub fn default_market_config() -> MarketConfig {
    MarketConfig::default()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MarketSellTokenArgs {
    pub from_address: Bytes32,
    pub token_id: u64,
    pub instance_id: u64,
    pub quote_token_id: u64,
    pub price: IntX,
    pub end_date: i64,
}

impl CarbonSerializable for MarketSellTokenArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.from_address);
        writer.write8u(self.token_id);
        writer.write8u(self.instance_id);
        writer.write8u(self.quote_token_id);
        self.price.write_carbon(writer)?;
        writer.write8(self.end_date);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            from_address: reader.read32()?,
            token_id: reader.read8u()?,
            instance_id: reader.read8u()?,
            quote_token_id: reader.read8u()?,
            price: IntX::read_carbon(reader)?,
            end_date: reader.read8()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketSellTokenByIdArgs {
    pub from_address: Bytes32,
    pub symbol: SmallString,
    pub instance_id: VMDynamicVariable,
    pub quote_symbol: SmallString,
    pub price: IntX,
    pub end_date: i64,
}

impl Default for MarketSellTokenByIdArgs {
    fn default() -> Self {
        Self {
            from_address: EMPTY_BYTES32,
            symbol: SmallString::default(),
            instance_id: VMDynamicVariable::int64(0),
            quote_symbol: SmallString::default(),
            price: IntX::default(),
            end_date: 0,
        }
    }
}

impl CarbonSerializable for MarketSellTokenByIdArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.from_address);
        self.symbol.write_carbon(writer)?;
        self.instance_id.write_carbon(writer)?;
        self.quote_symbol.write_carbon(writer)?;
        self.price.write_carbon(writer)?;
        writer.write8(self.end_date);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            from_address: reader.read32()?,
            symbol: SmallString::read_carbon(reader)?,
            instance_id: VMDynamicVariable::read_carbon(reader)?,
            quote_symbol: SmallString::read_carbon(reader)?,
            price: IntX::read_carbon(reader)?,
            end_date: reader.read8()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MarketCancelSaleArgs {
    pub token_id: u64,
    pub instance_id: u64,
}

impl CarbonSerializable for MarketCancelSaleArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write8u(self.instance_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            instance_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketCancelSaleByIdArgs {
    pub symbol: SmallString,
    pub instance_id: VMDynamicVariable,
}

impl Default for MarketCancelSaleByIdArgs {
    fn default() -> Self {
        Self {
            symbol: SmallString::default(),
            instance_id: VMDynamicVariable::int64(0),
        }
    }
}

impl CarbonSerializable for MarketCancelSaleByIdArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.symbol.write_carbon(writer)?;
        self.instance_id.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            symbol: SmallString::read_carbon(reader)?,
            instance_id: VMDynamicVariable::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MarketBuyTokenArgs {
    pub from_address: Bytes32,
    pub token_id: u64,
    pub instance_id: u64,
}

impl CarbonSerializable for MarketBuyTokenArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.from_address);
        writer.write8u(self.token_id);
        writer.write8u(self.instance_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            from_address: reader.read32()?,
            token_id: reader.read8u()?,
            instance_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketBuyTokenByIdArgs {
    pub from_address: Bytes32,
    pub symbol: SmallString,
    pub instance_id: VMDynamicVariable,
}

impl Default for MarketBuyTokenByIdArgs {
    fn default() -> Self {
        Self {
            from_address: EMPTY_BYTES32,
            symbol: SmallString::default(),
            instance_id: VMDynamicVariable::int64(0),
        }
    }
}

impl CarbonSerializable for MarketBuyTokenByIdArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.from_address);
        self.symbol.write_carbon(writer)?;
        self.instance_id.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            from_address: reader.read32()?,
            symbol: SmallString::read_carbon(reader)?,
            instance_id: VMDynamicVariable::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MarketGetTokenListingCountArgs {
    pub token_id: u64,
}

impl CarbonSerializable for MarketGetTokenListingCountArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MarketGetTokenListingInfoArgs {
    pub token_id: u64,
    pub instance_id: u64,
}

impl CarbonSerializable for MarketGetTokenListingInfoArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write8u(self.instance_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            instance_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketGetTokenListingInfoByIdArgs {
    pub symbol: SmallString,
    pub instance_id: VMDynamicVariable,
}

impl Default for MarketGetTokenListingInfoByIdArgs {
    fn default() -> Self {
        Self {
            symbol: SmallString::default(),
            instance_id: VMDynamicVariable::int64(0),
        }
    }
}

impl CarbonSerializable for MarketGetTokenListingInfoByIdArgs {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.symbol.write_carbon(writer)?;
        self.instance_id.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            symbol: SmallString::read_carbon(reader)?,
            instance_id: VMDynamicVariable::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TxMsgCall {
    pub module_id: u32,
    pub method_id: u32,
    pub args: Vec<u8>,
    pub sections: Option<MsgCallArgSections>,
}

impl CarbonSerializable for TxMsgCall {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write4u(self.module_id);
        writer.write4u(self.method_id);
        if let Some(sections) = &self.sections {
            if sections.has_sections() {
                return sections.write_carbon(writer);
            }
        }
        writer.write4(ensure_u32_len(self.args.len())? as i32);
        writer.write(&self.args);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let module_id = reader.read4u()?;
        let method_id = reader.read4u()?;
        let len = reader.read4()?;
        if len >= 0 {
            return Ok(Self {
                module_id,
                method_id,
                args: reader.read(len as usize)?,
                sections: None,
            });
        }
        Ok(Self {
            module_id,
            method_id,
            args: Vec::new(),
            sections: Some(MsgCallArgSections::read_with_count(reader, len)?),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CallArgSection {
    pub register_offset: i32,
    pub args: Vec<u8>,
}

impl CarbonSerializable for CallArgSection {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        if self.register_offset < 0 {
            writer.write4(self.register_offset);
            return Ok(());
        }
        writer.write4(ensure_u32_len(self.args.len())? as i32);
        writer.write(&self.args);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let value = reader.read4()?;
        if value < 0 {
            return Ok(Self {
                register_offset: value,
                args: Vec::new(),
            });
        }
        Ok(Self {
            register_offset: 0,
            args: reader.read(value as usize)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MsgCallArgSections {
    pub sections: Vec<CallArgSection>,
}

impl MsgCallArgSections {
    pub fn has_sections(&self) -> bool {
        !self.sections.is_empty()
    }

    pub fn read_with_count(reader: &mut CarbonReader<'_>, count_negative: i32) -> Result<Self> {
        if count_negative >= 0 {
            return serialization("arg sections count must be negative");
        }
        let count = (-count_negative) as usize;
        let mut sections = Vec::with_capacity(count);
        for _ in 0..count {
            sections.push(CallArgSection::read_carbon(reader)?);
        }
        Ok(Self { sections })
    }
}

impl CarbonSerializable for MsgCallArgSections {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        if self.sections.is_empty() {
            return serialization("arg sections are empty");
        }
        writer.write4(-(self.sections.len() as i32));
        for section in &self.sections {
            section.write_carbon(writer)?;
        }
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let count = reader.read4()?;
        Self::read_with_count(reader, count)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TxMsgCallMulti {
    pub calls: Vec<TxMsgCall>,
}

impl CarbonSerializable for TxMsgCallMulti {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        write_carbon_array(writer, &self.calls)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            calls: read_carbon_array(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TxMsgSpecialResolution {
    pub resolution_id: u64,
    pub calls: Vec<TxMsgCall>,
}

impl CarbonSerializable for TxMsgSpecialResolution {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.resolution_id);
        write_carbon_array(writer, &self.calls)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            resolution_id: reader.read8u()?,
            calls: read_carbon_array(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgTransferFungible {
    pub to: Bytes32,
    pub token_id: u64,
    pub amount: u64,
}

impl CarbonSerializable for TxMsgTransferFungible {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.to);
        writer.write8u(self.token_id);
        writer.write8u(self.amount);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            to: reader.read32()?,
            token_id: reader.read8u()?,
            amount: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgTransferFungibleGasPayer {
    pub to: Bytes32,
    pub from_address: Bytes32,
    pub token_id: u64,
    pub amount: u64,
}

impl CarbonSerializable for TxMsgTransferFungibleGasPayer {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.to);
        writer.write32(self.from_address);
        writer.write8u(self.token_id);
        writer.write8u(self.amount);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            to: reader.read32()?,
            from_address: reader.read32()?,
            token_id: reader.read8u()?,
            amount: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgTransferNonFungibleSingle {
    pub to: Bytes32,
    pub token_id: u64,
    pub instance_id: u64,
}

impl CarbonSerializable for TxMsgTransferNonFungibleSingle {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.to);
        writer.write8u(self.token_id);
        writer.write8u(self.instance_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            to: reader.read32()?,
            token_id: reader.read8u()?,
            instance_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgTransferNonFungibleSingleGasPayer {
    pub to: Bytes32,
    pub from_address: Bytes32,
    pub token_id: u64,
    pub instance_id: u64,
}

impl CarbonSerializable for TxMsgTransferNonFungibleSingleGasPayer {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.to);
        writer.write32(self.from_address);
        writer.write8u(self.token_id);
        writer.write8u(self.instance_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            to: reader.read32()?,
            from_address: reader.read32()?,
            token_id: reader.read8u()?,
            instance_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgTransferNonFungibleMulti {
    pub to: Bytes32,
    pub token_id: u64,
    pub instance_ids: Vec<u64>,
}

impl CarbonSerializable for TxMsgTransferNonFungibleMulti {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.to);
        writer.write8u(self.token_id);
        writer.write_u64_array(&self.instance_ids)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            to: reader.read32()?,
            token_id: reader.read8u()?,
            instance_ids: reader.read_u64_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgTransferNonFungibleMultiGasPayer {
    pub to: Bytes32,
    pub from_address: Bytes32,
    pub token_id: u64,
    pub instance_ids: Vec<u64>,
}

impl CarbonSerializable for TxMsgTransferNonFungibleMultiGasPayer {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.to);
        writer.write32(self.from_address);
        writer.write8u(self.token_id);
        writer.write_u64_array(&self.instance_ids)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            to: reader.read32()?,
            from_address: reader.read32()?,
            token_id: reader.read8u()?,
            instance_ids: reader.read_u64_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgMintFungible {
    pub token_id: u64,
    pub to: Bytes32,
    pub amount: IntX,
}

impl CarbonSerializable for TxMsgMintFungible {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.to);
        self.amount.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            to: reader.read32()?,
            amount: IntX::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgBurnFungible {
    pub token_id: u64,
    pub amount: IntX,
}

impl CarbonSerializable for TxMsgBurnFungible {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        self.amount.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            amount: IntX::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgBurnFungibleGasPayer {
    pub token_id: u64,
    pub from_address: Bytes32,
    pub amount: IntX,
}

impl CarbonSerializable for TxMsgBurnFungibleGasPayer {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.from_address);
        self.amount.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            from_address: reader.read32()?,
            amount: IntX::read_carbon(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgMintNonFungible {
    pub token_id: u64,
    pub to: Bytes32,
    pub series_id: u32,
    pub rom: Vec<u8>,
    pub ram: Vec<u8>,
}

impl CarbonSerializable for TxMsgMintNonFungible {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.to);
        writer.write4u(self.series_id);
        writer.write_byte_array(&self.rom)?;
        writer.write_byte_array(&self.ram)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            to: reader.read32()?,
            series_id: reader.read4u()?,
            rom: reader.read_byte_array()?,
            ram: reader.read_byte_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgBurnNonFungible {
    pub token_id: u64,
    pub instance_id: u64,
}

impl CarbonSerializable for TxMsgBurnNonFungible {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write8u(self.instance_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            instance_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgBurnNonFungibleGasPayer {
    pub token_id: u64,
    pub from_address: Bytes32,
    pub instance_id: u64,
}

impl CarbonSerializable for TxMsgBurnNonFungibleGasPayer {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write8u(self.token_id);
        writer.write32(self.from_address);
        writer.write8u(self.instance_id);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            token_id: reader.read8u()?,
            from_address: reader.read32()?,
            instance_id: reader.read8u()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TxMsgTrade {
    pub transfer_f: Vec<TxMsgTransferFungibleGasPayer>,
    pub transfer_n: Vec<TxMsgTransferNonFungibleSingleGasPayer>,
    pub mint_f: Vec<TxMsgMintFungible>,
    pub burn_f: Vec<TxMsgBurnFungibleGasPayer>,
    pub mint_n: Vec<TxMsgMintNonFungible>,
    pub burn_n: Vec<TxMsgBurnNonFungibleGasPayer>,
}

impl CarbonSerializable for TxMsgTrade {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        write_carbon_array(writer, &self.transfer_f)?;
        write_carbon_array(writer, &self.transfer_n)?;
        write_carbon_array(writer, &self.mint_f)?;
        write_carbon_array(writer, &self.burn_f)?;
        write_carbon_array(writer, &self.mint_n)?;
        write_carbon_array(writer, &self.burn_n)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            transfer_f: read_carbon_array(reader)?,
            transfer_n: read_carbon_array(reader)?,
            mint_f: read_carbon_array(reader)?,
            burn_f: read_carbon_array(reader)?,
            mint_n: read_carbon_array(reader)?,
            burn_n: read_carbon_array(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsgPhantasma {
    pub nexus: SmallString,
    pub chain: SmallString,
    pub script: Vec<u8>,
}

impl CarbonSerializable for TxMsgPhantasma {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.nexus.write_carbon(writer)?;
        self.chain.write_carbon(writer)?;
        writer.write_byte_array(&self.script)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            nexus: SmallString::read_carbon(reader)?,
            chain: SmallString::read_carbon(reader)?,
            script: reader.read_byte_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TxMsgPhantasmaRaw {
    pub transaction: Vec<u8>,
}

impl CarbonSerializable for TxMsgPhantasmaRaw {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write_byte_array(&self.transaction)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            transaction: reader.read_byte_array()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TxPayload {
    Call(TxMsgCall),
    CallMulti(TxMsgCallMulti),
    Trade(TxMsgTrade),
    TransferFungible(TxMsgTransferFungible),
    TransferFungibleGasPayer(TxMsgTransferFungibleGasPayer),
    TransferNonFungibleSingle(TxMsgTransferNonFungibleSingle),
    TransferNonFungibleSingleGasPayer(TxMsgTransferNonFungibleSingleGasPayer),
    TransferNonFungibleMulti(TxMsgTransferNonFungibleMulti),
    TransferNonFungibleMultiGasPayer(TxMsgTransferNonFungibleMultiGasPayer),
    MintFungible(TxMsgMintFungible),
    BurnFungible(TxMsgBurnFungible),
    BurnFungibleGasPayer(TxMsgBurnFungibleGasPayer),
    MintNonFungible(TxMsgMintNonFungible),
    BurnNonFungible(TxMsgBurnNonFungible),
    BurnNonFungibleGasPayer(TxMsgBurnNonFungibleGasPayer),
    Phantasma(TxMsgPhantasma),
    PhantasmaRaw(TxMsgPhantasmaRaw),
}

impl TxPayload {
    pub fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        match self {
            Self::Call(value) => value.write_carbon(writer),
            Self::CallMulti(value) => value.write_carbon(writer),
            Self::Trade(value) => value.write_carbon(writer),
            Self::TransferFungible(value) => value.write_carbon(writer),
            Self::TransferFungibleGasPayer(value) => value.write_carbon(writer),
            Self::TransferNonFungibleSingle(value) => value.write_carbon(writer),
            Self::TransferNonFungibleSingleGasPayer(value) => value.write_carbon(writer),
            Self::TransferNonFungibleMulti(value) => value.write_carbon(writer),
            Self::TransferNonFungibleMultiGasPayer(value) => value.write_carbon(writer),
            Self::MintFungible(value) => value.write_carbon(writer),
            Self::BurnFungible(value) => value.write_carbon(writer),
            Self::BurnFungibleGasPayer(value) => value.write_carbon(writer),
            Self::MintNonFungible(value) => value.write_carbon(writer),
            Self::BurnNonFungible(value) => value.write_carbon(writer),
            Self::BurnNonFungibleGasPayer(value) => value.write_carbon(writer),
            Self::Phantasma(value) => value.write_carbon(writer),
            Self::PhantasmaRaw(value) => value.write_carbon(writer),
        }
    }

    pub fn from_type(tx_type: TxType, reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(match tx_type {
            TxType::Call => Self::Call(TxMsgCall::read_carbon(reader)?),
            TxType::CallMulti => Self::CallMulti(TxMsgCallMulti::read_carbon(reader)?),
            TxType::Trade => Self::Trade(TxMsgTrade::read_carbon(reader)?),
            TxType::TransferFungible => {
                Self::TransferFungible(TxMsgTransferFungible::read_carbon(reader)?)
            }
            TxType::TransferFungibleGasPayer => {
                Self::TransferFungibleGasPayer(TxMsgTransferFungibleGasPayer::read_carbon(reader)?)
            }
            TxType::TransferNonFungibleSingle => Self::TransferNonFungibleSingle(
                TxMsgTransferNonFungibleSingle::read_carbon(reader)?,
            ),
            TxType::TransferNonFungibleSingleGasPayer => Self::TransferNonFungibleSingleGasPayer(
                TxMsgTransferNonFungibleSingleGasPayer::read_carbon(reader)?,
            ),
            TxType::TransferNonFungibleMulti => {
                Self::TransferNonFungibleMulti(TxMsgTransferNonFungibleMulti::read_carbon(reader)?)
            }
            TxType::TransferNonFungibleMultiGasPayer => Self::TransferNonFungibleMultiGasPayer(
                TxMsgTransferNonFungibleMultiGasPayer::read_carbon(reader)?,
            ),
            TxType::MintFungible => Self::MintFungible(TxMsgMintFungible::read_carbon(reader)?),
            TxType::BurnFungible => Self::BurnFungible(TxMsgBurnFungible::read_carbon(reader)?),
            TxType::BurnFungibleGasPayer => {
                Self::BurnFungibleGasPayer(TxMsgBurnFungibleGasPayer::read_carbon(reader)?)
            }
            TxType::MintNonFungible => {
                Self::MintNonFungible(TxMsgMintNonFungible::read_carbon(reader)?)
            }
            TxType::BurnNonFungible => {
                Self::BurnNonFungible(TxMsgBurnNonFungible::read_carbon(reader)?)
            }
            TxType::BurnNonFungibleGasPayer => {
                Self::BurnNonFungibleGasPayer(TxMsgBurnNonFungibleGasPayer::read_carbon(reader)?)
            }
            TxType::Phantasma => Self::Phantasma(TxMsgPhantasma::read_carbon(reader)?),
            TxType::PhantasmaRaw => Self::PhantasmaRaw(TxMsgPhantasmaRaw::read_carbon(reader)?),
        })
    }

    pub fn from_address(&self) -> Bytes32 {
        match self {
            Self::TransferFungibleGasPayer(value) => value.from_address,
            Self::TransferNonFungibleSingleGasPayer(value) => value.from_address,
            Self::TransferNonFungibleMultiGasPayer(value) => value.from_address,
            Self::BurnFungibleGasPayer(value) => value.from_address,
            Self::BurnNonFungibleGasPayer(value) => value.from_address,
            _ => EMPTY_BYTES32,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxMsg {
    pub tx_type: TxType,
    pub expiry: i64,
    pub max_gas: u64,
    pub max_data: u64,
    pub gas_from: Bytes32,
    pub payload: SmallString,
    pub msg: TxPayload,
}

impl CarbonSerializable for TxMsg {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write1(self.tx_type as u8);
        writer.write8(self.expiry);
        writer.write8u(self.max_gas);
        writer.write8u(self.max_data);
        writer.write32(self.gas_from);
        self.payload.write_carbon(writer)?;
        self.msg.write_carbon(writer)
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let tx_type = TxType::try_from(reader.read1()?)?;
        let expiry = reader.read8()?;
        let max_gas = reader.read8u()?;
        let max_data = reader.read8u()?;
        let gas_from = reader.read32()?;
        let payload = SmallString::read_carbon(reader)?;
        let msg = TxPayload::from_type(tx_type, reader)?;
        Ok(Self {
            tx_type,
            expiry,
            max_gas,
            max_data,
            gas_from,
            payload,
            msg,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Witness {
    pub address: Bytes32,
    pub signature: Bytes64,
}

impl CarbonSerializable for Witness {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        writer.write32(self.address);
        writer.write64(self.signature);
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        Ok(Self {
            address: reader.read32()?,
            signature: reader.read64()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedTxMsg {
    pub msg: TxMsg,
    pub witnesses: Vec<Witness>,
}

impl CarbonSerializable for SignedTxMsg {
    fn write_carbon(&self, writer: &mut CarbonWriter) -> Result<()> {
        self.msg.write_carbon(writer)?;
        match self.msg.tx_type {
            TxType::TransferFungible
            | TxType::TransferNonFungibleSingle
            | TxType::TransferNonFungibleMulti
            | TxType::MintFungible
            | TxType::BurnFungible
            | TxType::MintNonFungible
            | TxType::BurnNonFungible => {
                if self.witnesses.len() != 1 || self.witnesses[0].address != self.msg.gas_from {
                    return serialization("single-witness transaction address mismatch");
                }
                writer.write64(self.witnesses[0].signature);
            }
            TxType::TransferFungibleGasPayer
            | TxType::TransferNonFungibleSingleGasPayer
            | TxType::TransferNonFungibleMultiGasPayer
            | TxType::BurnFungibleGasPayer
            | TxType::BurnNonFungibleGasPayer => {
                if self.witnesses.len() != 2 || self.witnesses[0].address != self.msg.gas_from {
                    return serialization("gas witness address mismatch");
                }
                writer.write64(self.witnesses[0].signature);
                writer.write64(self.witnesses[1].signature);
            }
            TxType::Call | TxType::CallMulti | TxType::Trade | TxType::Phantasma => {
                write_carbon_array(writer, &self.witnesses)?;
            }
            TxType::PhantasmaRaw => {
                if !self.witnesses.is_empty() {
                    return serialization("raw Phantasma transaction must not contain witnesses");
                }
            }
        }
        Ok(())
    }

    fn read_carbon(reader: &mut CarbonReader<'_>) -> Result<Self> {
        let msg = TxMsg::read_carbon(reader)?;
        let witnesses = match msg.tx_type {
            TxType::TransferFungible
            | TxType::TransferNonFungibleSingle
            | TxType::TransferNonFungibleMulti
            | TxType::MintFungible
            | TxType::BurnFungible
            | TxType::MintNonFungible
            | TxType::BurnNonFungible => vec![Witness {
                address: msg.gas_from,
                signature: reader.read64()?,
            }],
            TxType::TransferFungibleGasPayer
            | TxType::TransferNonFungibleSingleGasPayer
            | TxType::TransferNonFungibleMultiGasPayer
            | TxType::BurnFungibleGasPayer
            | TxType::BurnNonFungibleGasPayer => vec![
                Witness {
                    address: msg.gas_from,
                    signature: reader.read64()?,
                },
                Witness {
                    address: msg.msg.from_address(),
                    signature: reader.read64()?,
                },
            ],
            TxType::Call | TxType::CallMulti | TxType::Trade | TxType::Phantasma => {
                read_carbon_array(reader)?
            }
            TxType::PhantasmaRaw => Vec::new(),
        };
        Ok(Self { msg, witnesses })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeeOptions {
    pub gas_fee_base: u64,
    pub fee_multiplier: u64,
}

impl Default for FeeOptions {
    fn default() -> Self {
        Self {
            gas_fee_base: 10_000,
            fee_multiplier: 1_000,
        }
    }
}

impl FeeOptions {
    pub fn calculate_max_gas(&self) -> u64 {
        self.gas_fee_base * self.fee_multiplier
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateTokenFeeOptions {
    pub gas_fee_base: u64,
    pub fee_multiplier: u64,
    pub gas_fee_create_token_base: u64,
    pub gas_fee_create_token_symbol: u64,
}

impl Default for CreateTokenFeeOptions {
    fn default() -> Self {
        Self {
            gas_fee_base: 10_000,
            fee_multiplier: 10_000,
            gas_fee_create_token_base: 10_000_000_000,
            gas_fee_create_token_symbol: 10_000_000_000,
        }
    }
}

impl CreateTokenFeeOptions {
    pub fn calculate_max_gas_for_symbol(&self, symbol: &SmallString) -> u64 {
        let shift = symbol.0.len().saturating_sub(1);
        let symbol_part = if shift < 64 {
            self.gas_fee_create_token_symbol >> shift
        } else {
            0
        };
        (self.gas_fee_base + self.gas_fee_create_token_base + symbol_part) * self.fee_multiplier
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSeriesFeeOptions {
    pub gas_fee_base: u64,
    pub fee_multiplier: u64,
    pub gas_fee_create_series_base: u64,
}

impl Default for CreateSeriesFeeOptions {
    fn default() -> Self {
        Self {
            gas_fee_base: 10_000,
            fee_multiplier: 10_000,
            gas_fee_create_series_base: 2_500_000_000,
        }
    }
}

impl CreateSeriesFeeOptions {
    pub fn calculate_max_gas(&self) -> u64 {
        (self.gas_fee_base + self.gas_fee_create_series_base) * self.fee_multiplier
    }
}

pub type MintNftFeeOptions = FeeOptions;

#[allow(clippy::upper_case_acronyms)]
pub type MintNFTFeeOptions = FeeOptions;

pub fn now_unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn prepare_standard_token_schemas(shared_metadata: bool) -> TokenSchemas {
    let mut series_fields = standard_series_fields();
    let mut rom_fields = standard_nft_fields();
    let metadata_fields = standard_metadata_fields();
    if shared_metadata {
        series_fields.extend(metadata_fields);
    } else {
        rom_fields.extend(metadata_fields);
    }
    TokenSchemas {
        series_metadata: VMStructSchema::new(series_fields),
        rom: VMStructSchema::new(rom_fields),
        ram: VMStructSchema::with_flags(Vec::new(), VMStructFlags::DYNAMIC_EXTRAS),
    }
}

pub fn serialize_token_schemas(schemas: &TokenSchemas) -> Result<Vec<u8>> {
    serialize(schemas)
}

pub fn serialize_token_schemas_hex(schemas: &TokenSchemas) -> Result<String> {
    Ok(hex::encode_upper(serialize_token_schemas(schemas)?))
}

pub fn build_and_serialize_token_schemas(schemas: Option<&TokenSchemas>) -> Result<Vec<u8>> {
    let owned;
    let schemas = if let Some(schemas) = schemas {
        schemas
    } else {
        owned = prepare_standard_token_schemas(false);
        &owned
    };
    serialize_token_schemas(schemas)
}

pub fn token_schemas_from_json(data: &str) -> Result<TokenSchemas> {
    let parsed = parse_token_schemas_json(data)?;
    build_token_schemas_from_fields(&parsed.series_metadata, &parsed.rom, &parsed.ram)
}

pub fn parse_token_schemas_json(data: &str) -> Result<TokenSchemasJson> {
    let raw: Value = serde_json::from_str(data)
        .map_err(|err| PhantasmaError::Builder(format!("invalid token schema JSON: {err}")))?;
    let Some(object) = raw.as_object() else {
        return builder("token schema JSON must be an object");
    };
    Ok(TokenSchemasJson {
        series_metadata: parse_token_schema_field_array(object, "seriesMetadata")?,
        rom: parse_token_schema_field_array(object, "rom")?,
        ram: parse_token_schema_field_array(object, "ram")?,
    })
}

pub fn vm_type_from_string(value: &str) -> Result<VMType> {
    match value.trim() {
        "Dynamic" => Ok(VMType::Dynamic),
        "Array" => Ok(VMType::Array),
        "Bytes" => Ok(VMType::Bytes),
        "Struct" => Ok(VMType::Struct),
        "Int8" => Ok(VMType::Int8),
        "Int16" => Ok(VMType::Int16),
        "Int32" => Ok(VMType::Int32),
        "Int64" => Ok(VMType::Int64),
        "Int256" => Ok(VMType::Int256),
        "Bytes16" => Ok(VMType::Bytes16),
        "Bytes32" => Ok(VMType::Bytes32),
        "Bytes64" => Ok(VMType::Bytes64),
        "String" => Ok(VMType::String),
        "Array_Dynamic" | "ArrayDynamic" => Ok(VMType::ArrayDynamic),
        "Array_Bytes" | "ArrayBytes" => Ok(VMType::ArrayBytes),
        "Array_Struct" | "ArrayStruct" => Ok(VMType::ArrayStruct),
        "Array_Int8" | "ArrayInt8" => Ok(VMType::ArrayInt8),
        "Array_Int16" | "ArrayInt16" => Ok(VMType::ArrayInt16),
        "Array_Int32" | "ArrayInt32" => Ok(VMType::ArrayInt32),
        "Array_Int64" | "ArrayInt64" => Ok(VMType::ArrayInt64),
        "Array_Int256" | "ArrayInt256" => Ok(VMType::ArrayInt256),
        "Array_Bytes16" | "ArrayBytes16" => Ok(VMType::ArrayBytes16),
        "Array_Bytes32" | "ArrayBytes32" => Ok(VMType::ArrayBytes32),
        "Array_Bytes64" | "ArrayBytes64" => Ok(VMType::ArrayBytes64),
        "Array_String" | "ArrayString" => Ok(VMType::ArrayString),
        other => builder(format!("unknown VM type: {other}")),
    }
}

pub fn vm_type_name(vm_type: VMType) -> Result<&'static str> {
    Ok(match vm_type {
        VMType::Dynamic => "Dynamic",
        VMType::Array => "Array",
        VMType::Bytes => "Bytes",
        VMType::Struct => "Struct",
        VMType::Int8 => "Int8",
        VMType::Int16 => "Int16",
        VMType::Int32 => "Int32",
        VMType::Int64 => "Int64",
        VMType::Int256 => "Int256",
        VMType::Bytes16 => "Bytes16",
        VMType::Bytes32 => "Bytes32",
        VMType::Bytes64 => "Bytes64",
        VMType::String => "String",
        VMType::ArrayDynamic => "Array_Dynamic",
        VMType::ArrayBytes => "Array_Bytes",
        VMType::ArrayStruct => "Array_Struct",
        VMType::ArrayInt8 => "Array_Int8",
        VMType::ArrayInt16 => "Array_Int16",
        VMType::ArrayInt32 => "Array_Int32",
        VMType::ArrayInt64 => "Array_Int64",
        VMType::ArrayInt256 => "Array_Int256",
        VMType::ArrayBytes16 => "Array_Bytes16",
        VMType::ArrayBytes32 => "Array_Bytes32",
        VMType::ArrayBytes64 => "Array_Bytes64",
        VMType::ArrayString => "Array_String",
    })
}

pub fn verify_token_schemas(schemas: &TokenSchemas) -> Result<()> {
    assert_metadata_fields(
        &[&schemas.series_metadata, &schemas.rom],
        &standard_metadata_fields(),
    )?;
    assert_metadata_fields(&[&schemas.series_metadata], &standard_series_fields())?;
    assert_metadata_fields(&[&schemas.rom], &standard_nft_fields())?;
    Ok(())
}

pub fn build_token_schemas_from_fields(
    series_metadata: &[TokenSchemaField],
    rom: &[TokenSchemaField],
    ram: &[TokenSchemaField],
) -> Result<TokenSchemas> {
    let mut series = standard_series_fields();
    series.extend(
        series_metadata
            .iter()
            .map(|field| VMNamedVariableSchema::new(field.name.as_str(), field.vm_type)),
    );
    let mut rom_fields = standard_nft_fields();
    rom_fields.extend(
        rom.iter()
            .map(|field| VMNamedVariableSchema::new(field.name.as_str(), field.vm_type)),
    );
    let ram_fields: Vec<_> = ram
        .iter()
        .map(|field| VMNamedVariableSchema::new(field.name.as_str(), field.vm_type))
        .collect();
    let schemas = TokenSchemas {
        series_metadata: VMStructSchema::new(series),
        rom: VMStructSchema::new(rom_fields),
        ram: VMStructSchema::with_flags(
            ram_fields,
            if ram.is_empty() {
                VMStructFlags::DYNAMIC_EXTRAS
            } else {
                VMStructFlags::NONE
            },
        ),
    };
    verify_token_schemas(&schemas)?;
    Ok(schemas)
}

pub fn build_token_metadata(fields: &[(&str, &str)]) -> Result<Vec<u8>> {
    for required in ["name", "icon", "url", "description"] {
        let Some((_, value)) = fields.iter().find(|(name, _)| *name == required) else {
            return builder(format!(
                "token metadata is missing required field: {required}"
            ));
        };
        if value.trim().is_empty() {
            return builder(format!(
                "token metadata is missing required field: {required}"
            ));
        }
    }
    let icon = fields
        .iter()
        .find(|(name, _)| *name == "icon")
        .map(|(_, value)| *value)
        .unwrap_or("");
    validate_icon_data_uri(icon)?;
    let structure = VMDynamicStruct::new(
        fields
            .iter()
            .map(|(name, value)| {
                VMNamedDynamicVariable::new(*name, VMDynamicVariable::string(*value))
            })
            .collect(),
    );
    serialize(&structure)
}

pub fn build_token_info(
    symbol: &str,
    max_supply: IntX,
    is_nft: bool,
    decimals: u8,
    owner: Bytes32,
    metadata: Vec<u8>,
    token_schemas: Vec<u8>,
) -> Result<TokenInfo> {
    check_token_symbol(symbol)?;
    if metadata.is_empty() {
        return builder("metadata is required for all tokens");
    }
    let flags = if is_nft {
        if !max_supply.is_8_byte_safe() {
            return builder("NFT maximum supply must fit into Int64");
        }
        if token_schemas.is_empty() {
            return builder("token schemas are required for NFTs");
        }
        TokenFlags::NON_FUNGIBLE
    } else if !max_supply.is_8_byte_safe() {
        TokenFlags::BIG_FUNGIBLE
    } else {
        TokenFlags::NONE
    };
    Ok(TokenInfo {
        max_supply,
        flags,
        decimals,
        owner,
        symbol: SmallString::new(symbol)?,
        metadata,
        token_schemas,
    })
}

pub fn build_series_info(
    phantasma_series_id: impl Into<BigInt>,
    max_mint: u32,
    max_supply: u32,
    owner: Bytes32,
) -> Result<SeriesInfo> {
    let schemas = prepare_standard_token_schemas(false);
    let metadata = build_token_series_metadata(&schemas.series_metadata, phantasma_series_id, &[])?;
    Ok(SeriesInfo {
        max_mint,
        max_supply,
        owner,
        metadata,
        rom: VMStructSchema::default(),
        ram: VMStructSchema::default(),
    })
}

pub fn build_token_series_metadata(
    schema: &VMStructSchema,
    phantasma_series_id: impl Into<BigInt>,
    metadata: &[(&str, VMValue)],
) -> Result<Vec<u8>> {
    let rom = metadata_bytes(metadata, "rom")?;
    let defaults = standard_series_fields();
    let mut fields = vec![
        VMNamedDynamicVariable::new(
            STANDARD_META_ID,
            VMDynamicVariable::new(VMType::Int256, VMValue::Int256(phantasma_series_id.into())),
        ),
        VMNamedDynamicVariable::new(
            "mode",
            VMDynamicVariable::new(VMType::Int8, VMValue::Int(i64::from(!rom.is_empty()))),
        ),
        VMNamedDynamicVariable::new("rom", VMDynamicVariable::bytes(rom)),
    ];
    fields.extend(metadata_dynamic_fields_for_schema(
        schema, &defaults, metadata,
    )?);
    write_dynamic_struct_with_schema(&VMDynamicStruct::new(fields), schema)
}

pub fn build_nft_rom(
    schema: &VMStructSchema,
    phantasma_nft_id: impl Into<BigInt>,
    metadata: &[(&str, VMValue)],
) -> Result<Vec<u8>> {
    let rom = metadata_bytes(metadata, "rom")?;
    let defaults = standard_nft_fields();
    let mut fields = vec![
        VMNamedDynamicVariable::new(
            STANDARD_META_ID,
            VMDynamicVariable::new(VMType::Int256, VMValue::Int256(phantasma_nft_id.into())),
        ),
        VMNamedDynamicVariable::new("rom", VMDynamicVariable::bytes(rom)),
    ];
    fields.extend(metadata_dynamic_fields_for_schema(
        schema, &defaults, metadata,
    )?);
    write_dynamic_struct_with_schema(&VMDynamicStruct::new(fields), schema)
}

pub fn build_phantasma_nft_public_mint_schema(nft_rom_schema: &VMStructSchema) -> VMStructSchema {
    VMStructSchema {
        fields: nft_rom_schema
            .fields
            .iter()
            .filter(|field| !is_phantasma_nft_reserved_field(&field.name.0))
            .cloned()
            .collect(),
        flags: nft_rom_schema.flags,
    }
}

pub fn build_phantasma_nft_rom(
    nft_rom_schema: &VMStructSchema,
    metadata: &[(&str, VMValue)],
) -> Result<Vec<u8>> {
    if metadata.is_empty() {
        return builder("metadata is required");
    }
    for (name, _) in metadata {
        if is_phantasma_nft_reserved_field(name) {
            return builder(format!(
                "metadata field \"{name}\" is reserved for chain-owned deterministic mint fields"
            ));
        }
    }
    let public_schema = build_phantasma_nft_public_mint_schema(nft_rom_schema);
    let fields = metadata_dynamic_fields_for_schema(&public_schema, &[], metadata)?;
    write_dynamic_struct_with_schema(&VMDynamicStruct::new(fields), &public_schema)
}

pub fn build_create_token_tx(
    token_info: TokenInfo,
    creator: Bytes32,
    fees: Option<CreateTokenFeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<TxMsg> {
    let fees = fees.unwrap_or_default();
    Ok(TxMsg {
        tx_type: TxType::Call,
        expiry: if expiry == 0 {
            now_unix_millis() + 60_000
        } else {
            expiry
        },
        max_gas: fees.calculate_max_gas_for_symbol(&token_info.symbol),
        max_data,
        gas_from: creator,
        payload: SmallString::default(),
        msg: TxPayload::Call(TxMsgCall {
            module_id: ModuleId::Token as u32,
            method_id: TokenContractMethod::CreateToken as u32,
            args: serialize(&token_info)?,
            sections: None,
        }),
    })
}

pub fn build_create_token_tx_and_sign(
    token_info: TokenInfo,
    signer: &PhantasmaKeys,
) -> Result<Vec<u8>> {
    build_create_token_tx_and_sign_with_options(token_info, signer, None, 100_000_000, 0)
}

pub fn build_create_token_tx_and_sign_with_options(
    token_info: TokenInfo,
    signer: &PhantasmaKeys,
    fees: Option<CreateTokenFeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<Vec<u8>> {
    let creator = bytes32_from_public_key(&signer.public_key())?;
    let msg = build_create_token_tx(token_info, creator, fees, max_data, expiry)?;
    sign_and_serialize_tx_msg(&msg, signer)
}

pub fn build_create_token_tx_and_sign_hex(
    token_info: TokenInfo,
    signer: &PhantasmaKeys,
) -> Result<String> {
    Ok(hex::encode(build_create_token_tx_and_sign(
        token_info, signer,
    )?))
}

pub fn build_create_token_tx_and_sign_hex_with_options(
    token_info: TokenInfo,
    signer: &PhantasmaKeys,
    fees: Option<CreateTokenFeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<String> {
    Ok(hex::encode(build_create_token_tx_and_sign_with_options(
        token_info, signer, fees, max_data, expiry,
    )?))
}

pub fn build_create_token_series_tx(
    token_id: u64,
    series_info: SeriesInfo,
    creator: Bytes32,
    fees: Option<CreateSeriesFeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<TxMsg> {
    let fees = fees.unwrap_or_default();
    let args = CreateTokenSeriesArgs {
        token_id,
        info: series_info,
    };
    Ok(TxMsg {
        tx_type: TxType::Call,
        expiry: if expiry == 0 {
            now_unix_millis() + 60_000
        } else {
            expiry
        },
        max_gas: fees.calculate_max_gas(),
        max_data,
        gas_from: creator,
        payload: SmallString::default(),
        msg: TxPayload::Call(TxMsgCall {
            module_id: ModuleId::Token as u32,
            method_id: TokenContractMethod::CreateTokenSeries as u32,
            args: serialize(&args)?,
            sections: None,
        }),
    })
}

pub fn build_create_token_series_tx_and_sign(
    token_id: u64,
    series_info: SeriesInfo,
    signer: &PhantasmaKeys,
) -> Result<Vec<u8>> {
    build_create_token_series_tx_and_sign_with_options(
        token_id,
        series_info,
        signer,
        None,
        100_000_000,
        0,
    )
}

pub fn build_create_token_series_tx_and_sign_with_options(
    token_id: u64,
    series_info: SeriesInfo,
    signer: &PhantasmaKeys,
    fees: Option<CreateSeriesFeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<Vec<u8>> {
    let creator = bytes32_from_public_key(&signer.public_key())?;
    let msg = build_create_token_series_tx(token_id, series_info, creator, fees, max_data, expiry)?;
    sign_and_serialize_tx_msg(&msg, signer)
}

pub fn build_create_token_series_tx_and_sign_hex(
    token_id: u64,
    series_info: SeriesInfo,
    signer: &PhantasmaKeys,
) -> Result<String> {
    Ok(hex::encode(build_create_token_series_tx_and_sign(
        token_id,
        series_info,
        signer,
    )?))
}

pub fn build_create_token_series_tx_and_sign_hex_with_options(
    token_id: u64,
    series_info: SeriesInfo,
    signer: &PhantasmaKeys,
    fees: Option<CreateSeriesFeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<String> {
    Ok(hex::encode(
        build_create_token_series_tx_and_sign_with_options(
            token_id,
            series_info,
            signer,
            fees,
            max_data,
            expiry,
        )?,
    ))
}

#[allow(clippy::too_many_arguments)]
pub fn build_mint_non_fungible_tx(
    token_id: u64,
    series_id: u32,
    sender: Bytes32,
    receiver: Bytes32,
    rom: Vec<u8>,
    ram: Vec<u8>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> TxMsg {
    let fees = fees.unwrap_or_default();
    TxMsg {
        tx_type: TxType::MintNonFungible,
        expiry: if expiry == 0 {
            now_unix_millis() + 60_000
        } else {
            expiry
        },
        max_gas: fees.calculate_max_gas(),
        max_data,
        gas_from: sender,
        payload: SmallString::default(),
        msg: TxPayload::MintNonFungible(TxMsgMintNonFungible {
            token_id,
            to: receiver,
            series_id,
            rom,
            ram,
        }),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_mint_non_fungible_tx_and_sign(
    token_id: u64,
    series_id: u32,
    signer: &PhantasmaKeys,
    receiver: Bytes32,
    rom: Vec<u8>,
    ram: Vec<u8>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<Vec<u8>> {
    let sender = bytes32_from_public_key(&signer.public_key())?;
    sign_and_serialize_tx_msg(
        &build_mint_non_fungible_tx(
            token_id, series_id, sender, receiver, rom, ram, fees, max_data, expiry,
        ),
        signer,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_mint_non_fungible_tx_and_sign_hex(
    token_id: u64,
    series_id: u32,
    signer: &PhantasmaKeys,
    receiver: Bytes32,
    rom: Vec<u8>,
    ram: Vec<u8>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<String> {
    Ok(hex::encode(build_mint_non_fungible_tx_and_sign(
        token_id, series_id, signer, receiver, rom, ram, fees, max_data, expiry,
    )?))
}

pub fn build_mint_phantasma_non_fungible_tx(
    token_id: u64,
    sender: Bytes32,
    receiver: Bytes32,
    tokens: Vec<PhantasmaNFTMintInfo>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<TxMsg> {
    let fees = fees.unwrap_or_default();
    let args = MintPhantasmaNonFungibleArgs {
        token_id,
        address: receiver,
        tokens,
    };
    Ok(TxMsg {
        tx_type: TxType::Call,
        expiry: if expiry == 0 {
            now_unix_millis() + 60_000
        } else {
            expiry
        },
        max_gas: fees.calculate_max_gas(),
        max_data,
        gas_from: sender,
        payload: SmallString::default(),
        msg: TxPayload::Call(TxMsgCall {
            module_id: ModuleId::Token as u32,
            method_id: TokenContractMethod::MintPhantasmaNonFungible as u32,
            args: serialize(&args)?,
            sections: None,
        }),
    })
}

#[allow(clippy::too_many_arguments)]
pub fn build_mint_phantasma_non_fungible_tx_and_sign(
    token_id: u64,
    signer: &PhantasmaKeys,
    receiver: Bytes32,
    tokens: Vec<PhantasmaNFTMintInfo>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<Vec<u8>> {
    let sender = bytes32_from_public_key(&signer.public_key())?;
    let msg = build_mint_phantasma_non_fungible_tx(
        token_id, sender, receiver, tokens, fees, max_data, expiry,
    )?;
    sign_and_serialize_tx_msg(&msg, signer)
}

#[allow(clippy::too_many_arguments)]
pub fn build_mint_phantasma_non_fungible_tx_and_sign_hex(
    token_id: u64,
    signer: &PhantasmaKeys,
    receiver: Bytes32,
    tokens: Vec<PhantasmaNFTMintInfo>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<String> {
    Ok(hex::encode(build_mint_phantasma_non_fungible_tx_and_sign(
        token_id, signer, receiver, tokens, fees, max_data, expiry,
    )?))
}

#[allow(clippy::too_many_arguments)]
pub fn build_mint_phantasma_non_fungible_single_tx(
    token_id: u64,
    phantasma_series_id: impl Into<BigInt>,
    sender: Bytes32,
    receiver: Bytes32,
    public_rom: Vec<u8>,
    ram: Vec<u8>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<TxMsg> {
    build_mint_phantasma_non_fungible_tx(
        token_id,
        sender,
        receiver,
        vec![PhantasmaNFTMintInfo {
            phantasma_series_id: IntX(phantasma_series_id.into()),
            rom: public_rom,
            ram,
        }],
        fees,
        max_data,
        expiry,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_mint_phantasma_non_fungible_single_tx_and_sign(
    token_id: u64,
    phantasma_series_id: impl Into<BigInt>,
    signer: &PhantasmaKeys,
    receiver: Bytes32,
    public_rom: Vec<u8>,
    ram: Vec<u8>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<Vec<u8>> {
    let sender = bytes32_from_public_key(&signer.public_key())?;
    let msg = build_mint_phantasma_non_fungible_single_tx(
        token_id,
        phantasma_series_id,
        sender,
        receiver,
        public_rom,
        ram,
        fees,
        max_data,
        expiry,
    )?;
    sign_and_serialize_tx_msg(&msg, signer)
}

#[allow(clippy::too_many_arguments)]
pub fn build_mint_phantasma_non_fungible_single_tx_and_sign_hex(
    token_id: u64,
    phantasma_series_id: impl Into<BigInt>,
    signer: &PhantasmaKeys,
    receiver: Bytes32,
    public_rom: Vec<u8>,
    ram: Vec<u8>,
    fees: Option<FeeOptions>,
    max_data: u64,
    expiry: i64,
) -> Result<String> {
    Ok(hex::encode(
        build_mint_phantasma_non_fungible_single_tx_and_sign(
            token_id,
            phantasma_series_id,
            signer,
            receiver,
            public_rom,
            ram,
            fees,
            max_data,
            expiry,
        )?,
    ))
}

pub fn sign_tx_msg(msg: &TxMsg, keys: &PhantasmaKeys) -> Result<SignedTxMsg> {
    let signature = keys.sign(serialize(msg)?);
    Ok(SignedTxMsg {
        msg: msg.clone(),
        witnesses: vec![Witness {
            address: bytes32_from_public_key(&keys.public_key())?,
            signature: Bytes64(*signature.data()),
        }],
    })
}

pub fn sign_and_serialize_tx_msg(msg: &TxMsg, keys: &PhantasmaKeys) -> Result<Vec<u8>> {
    serialize(&sign_tx_msg(msg, keys)?)
}

pub fn sign_and_serialize_tx_msg_hex(msg: &TxMsg, keys: &PhantasmaKeys) -> Result<String> {
    Ok(hex::encode(sign_and_serialize_tx_msg(msg, keys)?))
}

pub fn get_nft_address(carbon_token_id: u64, instance_id: u64) -> Bytes32 {
    let mut address = [0u8; 32];
    address[15] = 1;
    address[16..24].copy_from_slice(&carbon_token_id.to_le_bytes());
    address[24..32].copy_from_slice(&instance_id.to_le_bytes());
    Bytes32(address)
}

pub fn unpack_nft_instance_id(instance_id: u64) -> (u32, u32) {
    (
        (instance_id & 0xFFFF_FFFF) as u32,
        ((instance_id >> 32) & 0xFFFF_FFFF) as u32,
    )
}

pub fn parse_create_token_result(result_hex: &str) -> Result<u64> {
    let data = decode_hex(result_hex)?;
    let mut reader = CarbonReader::new(&data);
    let value = reader.read8u()?;
    reader.assert_eof()?;
    Ok(value)
}

pub fn parse_create_token_series_result(result_hex: &str) -> Result<u32> {
    let data = decode_hex(result_hex)?;
    let mut reader = CarbonReader::new(&data);
    let value = reader.read4u()?;
    reader.assert_eof()?;
    Ok(value)
}

pub fn parse_mint_non_fungible_result(
    carbon_token_id: u64,
    result_hex: &str,
) -> Result<Vec<Bytes32>> {
    let data = decode_hex(result_hex)?;
    let mut reader = CarbonReader::new(&data);
    let count = reader.read_length()?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(get_nft_address(carbon_token_id, reader.read8u()?));
    }
    reader.assert_eof()?;
    Ok(out)
}

pub fn parse_mint_phantasma_non_fungible_result(
    result_hex: &str,
) -> Result<Vec<PhantasmaNFTMintResult>> {
    let data = decode_hex(result_hex)?;
    let mut reader = CarbonReader::new(&data);
    let out = read_carbon_array(&mut reader)?;
    reader.assert_eof()?;
    Ok(out)
}

pub fn check_token_symbol(symbol: &str) -> Result<()> {
    if symbol.is_empty() {
        return builder("token symbol must not be empty");
    }
    if symbol.len() > 255 {
        return builder("token symbol exceeds 255 UTF-8 bytes");
    }
    if !symbol.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return builder("token symbol must contain only uppercase ASCII letters A-Z");
    }
    Ok(())
}

fn standard_series_fields() -> Vec<VMNamedVariableSchema> {
    vec![
        VMNamedVariableSchema::new(STANDARD_META_ID, VMType::Int256),
        VMNamedVariableSchema::new("mode", VMType::Int8),
        VMNamedVariableSchema::new("rom", VMType::Bytes),
    ]
}

fn standard_nft_fields() -> Vec<VMNamedVariableSchema> {
    vec![
        VMNamedVariableSchema::new(STANDARD_META_ID, VMType::Int256),
        VMNamedVariableSchema::new("rom", VMType::Bytes),
    ]
}

fn standard_metadata_fields() -> Vec<VMNamedVariableSchema> {
    vec![
        VMNamedVariableSchema::new("name", VMType::String),
        VMNamedVariableSchema::new("description", VMType::String),
        VMNamedVariableSchema::new("imageURL", VMType::String),
        VMNamedVariableSchema::new("infoURL", VMType::String),
        VMNamedVariableSchema::new("royalties", VMType::Int32),
    ]
}

fn assert_metadata_fields(
    schemas: &[&VMStructSchema],
    fields: &[VMNamedVariableSchema],
) -> Result<()> {
    // Required metadata fields may live in either series metadata or ROM,
    // depending on whether the token shares metadata at series level. Field
    // names are case-sensitive on-chain, so case-only matches are errors.
    for expected in fields {
        let mut case_mismatch = false;
        let mut found = false;
        for schema in schemas {
            for actual in &schema.fields {
                if actual.name == expected.name {
                    if actual.schema.vm_type != expected.schema.vm_type {
                        return builder(format!("type mismatch for {} field", expected.name.0));
                    }
                    found = true;
                    case_mismatch = false;
                    break;
                }
                if actual.name.0.eq_ignore_ascii_case(&expected.name.0) {
                    case_mismatch = true;
                }
            }
            if found {
                break;
            }
        }
        if !found {
            if case_mismatch {
                return builder(format!("case mismatch for {} field", expected.name.0));
            }
            return builder(format!(
                "mandatory metadata field not found: {}",
                expected.name.0
            ));
        }
    }
    Ok(())
}

fn parse_token_schema_field_array(
    raw: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<TokenSchemaField>> {
    let Some(items) = raw.get(key).and_then(Value::as_array) else {
        return builder(format!("{key} must be an array"));
    };
    let mut out = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let Some(object) = item.as_object() else {
            return builder(format!("{key} field {index} invalid"));
        };
        let Some(name) = object.get("name").and_then(Value::as_str) else {
            return builder(format!("{key} field name must be string"));
        };
        let Some(raw_type) = object.get("type").and_then(Value::as_str) else {
            return builder(format!("{key} field type must be string"));
        };
        out.push(TokenSchemaField::new(name, vm_type_from_string(raw_type)?));
    }
    Ok(out)
}

fn is_phantasma_nft_reserved_field(name: &str) -> bool {
    name.eq_ignore_ascii_case(STANDARD_META_ID) || name.eq_ignore_ascii_case("rom")
}

fn metadata_bytes(metadata: &[(&str, VMValue)], name: &str) -> Result<Vec<u8>> {
    match find_metadata_value(metadata, name)? {
        Some(value) => coerce_metadata_bytes(name, value),
        None => Ok(Vec::new()),
    }
}

fn metadata_dynamic_fields_for_schema(
    schema: &VMStructSchema,
    defaults: &[VMNamedVariableSchema],
    metadata: &[(&str, VMValue)],
) -> Result<Vec<VMNamedDynamicVariable>> {
    // Builders accept caller metadata only for fields declared by the schema.
    // Missing custom fields are rejected here rather than silently materialized
    // as zero values, because these bytes are later consumed by VM contracts.
    let mut out = Vec::new();
    for field_schema in &schema.fields {
        let name = field_schema.name.0.as_str();
        if defaults.iter().any(|field| field.name.0 == name) {
            continue;
        }
        let Some(value) = find_metadata_value(metadata, name)? else {
            return builder(format!("metadata field \"{name}\" is mandatory"));
        };
        out.push(VMNamedDynamicVariable::new(
            field_schema.name.clone(),
            VMDynamicVariable::new(
                field_schema.schema.vm_type,
                coerce_metadata_value(name, &field_schema.schema, value)?,
            ),
        ));
    }
    Ok(out)
}

fn find_metadata_value<'a>(
    metadata: &'a [(&str, VMValue)],
    name: &str,
) -> Result<Option<&'a VMValue>> {
    for (key, value) in metadata {
        if *key == name {
            return Ok(Some(value));
        }
    }
    for (key, _) in metadata {
        if key.eq_ignore_ascii_case(name) {
            return builder(format!(
                "metadata field \"{name}\" provided in incorrect case as \"{key}\""
            ));
        }
    }
    Ok(None)
}

fn coerce_metadata_value(
    name: &str,
    schema: &VMVariableSchema,
    value: &VMValue,
) -> Result<VMValue> {
    // Coercion is intentionally narrow: bytes may be real bytes or hex strings,
    // integer widths accept their unsigned two's-complement input range, and
    // nested structures must match the declared schema exactly.
    Ok(match schema.vm_type {
        VMType::Bytes => VMValue::Bytes(coerce_metadata_bytes(name, value)?),
        VMType::String => VMValue::String(coerce_metadata_string(name, value)?),
        VMType::Int8 | VMType::Int16 | VMType::Int32 | VMType::Int64 => VMValue::Int(
            coerce_metadata_integer_for_type(name, schema.vm_type, metadata_integer(name, value)?)?,
        ),
        VMType::Int256 => {
            let value = metadata_bigint(name, value)?;
            validate_int256(name, &value)?;
            VMValue::Int256(value)
        }
        VMType::Bytes16 => VMValue::Bytes16(coerce_metadata_fixed_bytes16(name, value)?),
        VMType::Bytes32 => VMValue::Bytes32(coerce_metadata_fixed_bytes32(name, value)?),
        VMType::Bytes64 => VMValue::Bytes64(coerce_metadata_fixed_bytes64(name, value)?),
        VMType::Struct => {
            let struct_def = schema.struct_def.as_ref().ok_or_else(|| {
                PhantasmaError::Builder(format!(
                    "metadata field \"{name}\" is missing struct schema"
                ))
            })?;
            let VMValue::Struct(value) = value else {
                return builder(format!("metadata field \"{name}\" must be a struct"));
            };
            validate_metadata_struct(name, struct_def, value)?;
            VMValue::Struct(value.clone())
        }
        VMType::ArrayStruct => {
            let struct_def = schema.struct_def.as_ref().ok_or_else(|| {
                PhantasmaError::Builder(format!(
                    "metadata field \"{name}\" is missing a struct schema"
                ))
            })?;
            let VMValue::ArrayStruct(value) = value else {
                return builder(format!(
                    "metadata field \"{name}\" must be provided as a struct array"
                ));
            };
            for (index, item) in value.structs.iter().enumerate() {
                validate_metadata_struct(&format!("{name}[{index}]"), struct_def, item)?;
            }
            VMValue::ArrayStruct(value.clone())
        }
        VMType::ArrayDynamic => match value {
            VMValue::ArrayDynamic(value) => VMValue::ArrayDynamic(value.clone()),
            _ => {
                return builder(format!(
                    "metadata field \"{name}\" must contain dynamic VM variables"
                ));
            }
        },
        VMType::ArrayBytes => match value {
            VMValue::ArrayBytes(value) => VMValue::ArrayBytes(value.clone()),
            _ => {
                return builder(format!(
                    "metadata field \"{name}\" must be a byte array array"
                ))
            }
        },
        VMType::ArrayInt8 => match value {
            VMValue::ArrayInt8(value) => VMValue::ArrayInt8(value.clone()),
            _ => return builder(format!("metadata field \"{name}\" must be an int8 array")),
        },
        VMType::ArrayInt16 => match value {
            VMValue::ArrayInt16(value) => VMValue::ArrayInt16(value.clone()),
            _ => return builder(format!("metadata field \"{name}\" must be an int16 array")),
        },
        VMType::ArrayInt32 => match value {
            VMValue::ArrayInt32(value) => VMValue::ArrayInt32(value.clone()),
            _ => return builder(format!("metadata field \"{name}\" must be an int32 array")),
        },
        VMType::ArrayInt64 => match value {
            VMValue::ArrayInt64(value) => VMValue::ArrayInt64(value.clone()),
            _ => return builder(format!("metadata field \"{name}\" must be an int64 array")),
        },
        VMType::ArrayInt256 => match value {
            VMValue::ArrayInt256(value) => {
                for item in value {
                    validate_int256(name, item)?;
                }
                VMValue::ArrayInt256(value.clone())
            }
            _ => return builder(format!("metadata field \"{name}\" must be an int256 array")),
        },
        VMType::ArrayBytes16 => match value {
            VMValue::ArrayBytes16(value) => VMValue::ArrayBytes16(value.clone()),
            _ => return builder(format!("metadata field \"{name}\" must be a bytes16 array")),
        },
        VMType::ArrayBytes32 => match value {
            VMValue::ArrayBytes32(value) => VMValue::ArrayBytes32(value.clone()),
            _ => return builder(format!("metadata field \"{name}\" must be a bytes32 array")),
        },
        VMType::ArrayBytes64 => match value {
            VMValue::ArrayBytes64(value) => VMValue::ArrayBytes64(value.clone()),
            _ => return builder(format!("metadata field \"{name}\" must be a bytes64 array")),
        },
        VMType::ArrayString => match value {
            VMValue::ArrayString(value) => {
                for (index, item) in value.iter().enumerate() {
                    if item.trim().is_empty() {
                        return builder(format!("metadata field \"{name}[{index}]\" is mandatory"));
                    }
                }
                VMValue::ArrayString(value.clone())
            }
            _ => return builder(format!("metadata field \"{name}\" must be a string array")),
        },
        VMType::Dynamic => match value {
            VMValue::Dynamic(value) => VMValue::Dynamic(value.clone()),
            _ => {
                return builder(format!(
                    "metadata field \"{name}\" must be a dynamic VM variable"
                ))
            }
        },
        VMType::Array => return serialization("unsupported VM dynamic type: Array"),
    })
}

fn validate_metadata_struct(
    parent_name: &str,
    schema: &VMStructSchema,
    value: &VMDynamicStruct,
) -> Result<()> {
    for field_schema in &schema.fields {
        let name = field_schema.name.0.as_str();
        let Some(value) = find_struct_field(value, name)? else {
            return builder(format!("metadata field \"{name}\" is mandatory"));
        };
        coerce_metadata_value(name, &field_schema.schema, &value.data)?;
    }
    for field in &value.fields {
        if schema
            .fields
            .iter()
            .any(|schema_field| schema_field.name == field.name)
        {
            continue;
        }
        return builder(format!(
            "metadata field \"{parent_name}\" received unknown property \"{}\"",
            field.name.0
        ));
    }
    Ok(())
}

fn find_struct_field<'a>(
    structure: &'a VMDynamicStruct,
    name: &str,
) -> Result<Option<&'a VMDynamicVariable>> {
    for field in &structure.fields {
        if field.name.0 == name {
            return Ok(Some(&field.value));
        }
    }
    for field in &structure.fields {
        if field.name.0.eq_ignore_ascii_case(name) {
            return builder(format!(
                "metadata field \"{name}\" provided in incorrect case as \"{}\"",
                field.name.0
            ));
        }
    }
    Ok(None)
}

fn coerce_metadata_bytes(name: &str, value: &VMValue) -> Result<Vec<u8>> {
    match value {
        VMValue::Bytes(value) => Ok(value.clone()),
        VMValue::String(value) => decode_metadata_hex(name, value),
        _ => builder(format!(
            "metadata field \"{name}\" must be a byte array or hex string"
        )),
    }
}

fn coerce_metadata_string(name: &str, value: &VMValue) -> Result<String> {
    let VMValue::String(value) = value else {
        return builder(format!("metadata field \"{name}\" must be a string"));
    };
    if value.trim().is_empty() {
        return builder(format!("metadata field \"{name}\" is mandatory"));
    }
    Ok(value.clone())
}

fn coerce_metadata_fixed_bytes16(name: &str, value: &VMValue) -> Result<Bytes16> {
    match value {
        VMValue::Bytes16(value) => Ok(*value),
        _ => Bytes16::try_from_slice(&coerce_metadata_bytes(name, value)?).map_err(|_| {
            PhantasmaError::Builder(format!(
                "metadata field \"{name}\" must be exactly 16 bytes"
            ))
        }),
    }
}

fn coerce_metadata_fixed_bytes32(name: &str, value: &VMValue) -> Result<Bytes32> {
    match value {
        VMValue::Bytes32(value) => Ok(*value),
        _ => Bytes32::try_from_slice(&coerce_metadata_bytes(name, value)?).map_err(|_| {
            PhantasmaError::Builder(format!(
                "metadata field \"{name}\" must be exactly 32 bytes"
            ))
        }),
    }
}

fn coerce_metadata_fixed_bytes64(name: &str, value: &VMValue) -> Result<Bytes64> {
    match value {
        VMValue::Bytes64(value) => Ok(*value),
        _ => Bytes64::try_from_slice(&coerce_metadata_bytes(name, value)?).map_err(|_| {
            PhantasmaError::Builder(format!(
                "metadata field \"{name}\" must be exactly 64 bytes"
            ))
        }),
    }
}

fn metadata_integer(name: &str, value: &VMValue) -> Result<i128> {
    match value {
        VMValue::Int(value) => Ok(i128::from(*value)),
        VMValue::Int256(value) => value.to_i128().ok_or_else(|| {
            PhantasmaError::Builder(format!("metadata field \"{name}\" must be an integer"))
        }),
        _ => builder(format!("metadata field \"{name}\" must be an integer")),
    }
}

fn metadata_bigint(name: &str, value: &VMValue) -> Result<BigInt> {
    match value {
        VMValue::Int(value) => Ok(BigInt::from(*value)),
        VMValue::Int256(value) => Ok(value.clone()),
        _ => builder(format!("metadata field \"{name}\" must be an integer")),
    }
}

fn coerce_metadata_integer_for_type(name: &str, vm_type: VMType, value: i128) -> Result<i64> {
    let bit_width = match vm_type {
        VMType::Int8 => 8,
        VMType::Int16 => 16,
        VMType::Int32 => 32,
        VMType::Int64 => 64,
        _ => unreachable!("integer VM type"),
    };
    let signed_min = -(1i128 << (bit_width - 1));
    let signed_max = (1i128 << (bit_width - 1)) - 1;
    let unsigned_max = (1i128 << bit_width) - 1;
    if !(signed_min..=signed_max).contains(&value) && !(0..=unsigned_max).contains(&value) {
        return builder(format!(
            "metadata field \"{name}\" must be between {signed_min} and {signed_max} or between 0 and {unsigned_max}"
        ));
    }
    if bit_width == 8 || value <= signed_max {
        return value.try_into().map_err(|_| {
            PhantasmaError::Builder(format!("metadata field \"{name}\" must be an integer"))
        });
    }
    // Reference SDKs let callers provide unsigned values for signed VM integer
    // fields, so convert the upper half of the unsigned range into the signed
    // two's-complement representation used on the wire.
    (value - (1i128 << bit_width)).try_into().map_err(|_| {
        PhantasmaError::Builder(format!("metadata field \"{name}\" must be an integer"))
    })
}

fn validate_int256(name: &str, value: &BigInt) -> Result<()> {
    let signed_min = -(BigInt::from(1u8) << 255usize);
    let signed_max = (BigInt::from(1u8) << 255usize) - BigInt::from(1u8);
    let unsigned_max = (BigInt::from(1u8) << 256usize) - BigInt::from(1u8);
    if !(value >= &signed_min && value <= &signed_max
        || value >= &BigInt::zero() && value <= &unsigned_max)
    {
        return builder(format!(
            "metadata field \"{name}\" must be between {signed_min} and {signed_max} or between 0 and {unsigned_max}"
        ));
    }
    Ok(())
}

fn decode_metadata_hex(name: &str, value: &str) -> Result<Vec<u8>> {
    let text = value.trim();
    if text.is_empty() {
        return builder(format!(
            "metadata field \"{name}\" must be a byte array or hex string"
        ));
    }
    let text = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    let padded;
    let text = if text.len() % 2 == 0 {
        text
    } else {
        padded = format!("0{text}");
        padded.as_str()
    };
    hex::decode(text).map_err(|_| {
        PhantasmaError::Builder(format!(
            "metadata field \"{name}\" must be a byte array or hex string"
        ))
    })
}

fn write_dynamic_struct_with_schema(
    structure: &VMDynamicStruct,
    schema: &VMStructSchema,
) -> Result<Vec<u8>> {
    let mut writer = CarbonWriter::new();
    if !structure.write_with_schema(schema, &mut writer)? {
        return builder("metadata does not match schema");
    }
    Ok(writer.into_bytes())
}

fn default_dynamic_variable(vm_type: VMType) -> VMDynamicVariable {
    // Schema-bound struct serialization in the reference SDKs supplies default
    // values for absent fields. Public metadata builders perform stricter
    // validation before reaching this lower-level serializer.
    VMDynamicVariable::new(
        vm_type,
        match vm_type {
            VMType::Bytes => VMValue::Bytes(Vec::new()),
            VMType::Struct => VMValue::Struct(VMDynamicStruct::default()),
            VMType::Int8 | VMType::Int16 | VMType::Int32 | VMType::Int64 => VMValue::Int(0),
            VMType::Int256 => VMValue::Int256(BigInt::zero()),
            VMType::Bytes16 => VMValue::Bytes16(EMPTY_BYTES16),
            VMType::Bytes32 => VMValue::Bytes32(EMPTY_BYTES32),
            VMType::Bytes64 => VMValue::Bytes64(EMPTY_BYTES64),
            VMType::String => VMValue::String(String::new()),
            VMType::ArrayStruct => VMValue::ArrayStruct(VMStructArray::default()),
            VMType::ArrayDynamic => VMValue::ArrayDynamic(Vec::new()),
            VMType::ArrayBytes => VMValue::ArrayBytes(Vec::new()),
            VMType::ArrayInt8 => VMValue::ArrayInt8(Vec::new()),
            VMType::ArrayInt16 => VMValue::ArrayInt16(Vec::new()),
            VMType::ArrayInt32 => VMValue::ArrayInt32(Vec::new()),
            VMType::ArrayInt64 => VMValue::ArrayInt64(Vec::new()),
            VMType::ArrayInt256 => VMValue::ArrayInt256(Vec::new()),
            VMType::ArrayBytes16 => VMValue::ArrayBytes16(Vec::new()),
            VMType::ArrayBytes32 => VMValue::ArrayBytes32(Vec::new()),
            VMType::ArrayBytes64 => VMValue::ArrayBytes64(Vec::new()),
            VMType::ArrayString => VMValue::ArrayString(Vec::new()),
            VMType::Dynamic | VMType::Array => VMValue::None,
        },
    )
}

fn ensure_i128_range(value: i128, min: i128, max: i128, label: &str) -> Result<()> {
    if value < min || value > max {
        return serialization(format!("{label} value out of range: {value}"));
    }
    Ok(())
}

fn write_named_dynamic_variables(
    writer: &mut CarbonWriter,
    values: &[VMNamedDynamicVariable],
) -> Result<()> {
    writer.write4(ensure_u32_len(values.len())? as i32);
    for value in values {
        value.write_carbon(writer)?;
    }
    Ok(())
}

fn read_named_dynamic_variables(
    reader: &mut CarbonReader<'_>,
) -> Result<Vec<VMNamedDynamicVariable>> {
    let count = reader.read_length()?;
    (0..count)
        .map(|_| VMNamedDynamicVariable::read_carbon(reader))
        .collect()
}

fn write_dynamic_variables(writer: &mut CarbonWriter, values: &[VMDynamicVariable]) -> Result<()> {
    writer.write4(ensure_u32_len(values.len())? as i32);
    for value in values {
        value.write_carbon(writer)?;
    }
    Ok(())
}

fn read_dynamic_variables(reader: &mut CarbonReader<'_>) -> Result<Vec<VMDynamicVariable>> {
    let count = reader.read_length()?;
    (0..count)
        .map(|_| VMDynamicVariable::read_carbon(reader))
        .collect()
}

fn write_carbon_array<T: CarbonSerializable>(
    writer: &mut CarbonWriter,
    values: &[T],
) -> Result<()> {
    writer.write4(ensure_u32_len(values.len())? as i32);
    for value in values {
        value.write_carbon(writer)?;
    }
    Ok(())
}

fn read_carbon_array<T: CarbonSerializable>(reader: &mut CarbonReader<'_>) -> Result<Vec<T>> {
    let count = reader.read_length()?;
    (0..count).map(|_| T::read_carbon(reader)).collect()
}

fn validate_icon_data_uri(value: &str) -> Result<()> {
    let lower = value.to_ascii_lowercase();
    let valid_prefix = lower.starts_with("data:image/png;base64,")
        || lower.starts_with("data:image/jpeg;base64,")
        || lower.starts_with("data:image/webp;base64,");
    if !valid_prefix {
        return builder("token icon must be a png/jpeg/webp data URI");
    }
    let Some((_, payload)) = value.split_once(',') else {
        return builder("token icon data URI must include a non-empty base64 payload");
    };
    if payload.is_empty() {
        return builder("token icon data URI must include a non-empty base64 payload");
    }
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .map_err(|err| {
            PhantasmaError::Builder(format!(
                "token icon data URI contains invalid base64: {err}"
            ))
        })?;
    Ok(())
}
