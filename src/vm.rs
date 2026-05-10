//! VM script and object helpers.
//!
//! ScriptBuilder follows the current Python SDK public surface while keeping Rust
//! error handling: builder methods latch errors and `end_script()` reports them
//! instead of emitting partially invalid scripts.

use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use num_bigint::{BigInt, Sign};
use num_traits::{ToPrimitive, Zero};

use crate::binary::{
    big_int_to_csharp_bytes, vm_bytes_to_big_int, BinaryReader, BinaryWriter, MAX_ARRAY_SIZE,
};
use crate::crypto::Address;
use crate::error::{builder, serialization, PhantasmaError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Opcode {
    Nop = 0,
    Move = 1,
    Copy = 2,
    Push = 3,
    Pop = 4,
    Swap = 5,
    Call = 6,
    ExtCall = 7,
    Jmp = 8,
    JmpIf = 9,
    JmpNot = 10,
    Ret = 11,
    Throw = 12,
    Load = 13,
    Cast = 14,
    Cat = 15,
    Range = 16,
    Left = 17,
    Right = 18,
    Size = 19,
    Count = 20,
    Not = 21,
    And = 22,
    Or = 23,
    Xor = 24,
    Equal = 25,
    Lt = 26,
    Gt = 27,
    Lte = 28,
    Gte = 29,
    Inc = 30,
    Dec = 31,
    Sign = 32,
    Negate = 33,
    Abs = 34,
    Add = 35,
    Sub = 36,
    Mul = 37,
    Div = 38,
    Mod = 39,
    Shl = 40,
    Shr = 41,
    Min = 42,
    Max = 43,
    Pow = 44,
    Ctx = 45,
    Switch = 46,
    Put = 47,
    Get = 48,
    Clear = 49,
    Unpack = 50,
    Pack = 51,
    Debug = 52,
    Substr = 53,
    Remove = 54,
    Evm = 255,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum VMType {
    None = 0,
    Struct = 1,
    Bytes = 2,
    Number = 3,
    String = 4,
    Timestamp = 5,
    Bool = 6,
    Enum = 7,
    Object = 8,
}

impl TryFrom<u8> for VMType {
    type Error = PhantasmaError;

    fn try_from(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Struct),
            2 => Ok(Self::Bytes),
            3 => Ok(Self::Number),
            4 => Ok(Self::String),
            5 => Ok(Self::Timestamp),
            6 => Ok(Self::Bool),
            7 => Ok(Self::Enum),
            8 => Ok(Self::Object),
            _ => serialization("unsupported VM object type"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum VMObject {
    None,
    Struct(Vec<(VMObject, VMObject)>),
    Bytes(Vec<u8>),
    Number(BigInt),
    String(String),
    Timestamp(u32),
    Bool(bool),
    Enum(u32),
    Object(Vec<u8>),
}

impl VMObject {
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut reader = BinaryReader::new(data);
        let out = Self::read(&mut reader)?;
        reader.assert_eof()?;
        Ok(out)
    }

    pub fn read(reader: &mut BinaryReader<'_>) -> Result<Self> {
        match VMType::try_from(reader.read_u8()?)? {
            VMType::None => Ok(Self::None),
            VMType::Bool => Ok(Self::Bool(reader.read_bool()?)),
            VMType::Bytes => Ok(Self::Bytes(reader.read_var_bytes(MAX_ARRAY_SIZE)?)),
            VMType::Enum => Ok(Self::Enum(u32::from(reader.read_u8()?))),
            VMType::Number => Ok(Self::Number(reader.read_big_integer()?)),
            VMType::String => Ok(Self::String(reader.read_string()?)),
            VMType::Timestamp => Ok(Self::Timestamp(reader.read_u32_le()?)),
            VMType::Object => {
                let bytes = reader.read_var_bytes(MAX_ARRAY_SIZE)?;
                if bytes.len() == 35 && bytes[0] == 34 {
                    Ok(Self::Object(bytes[1..].to_vec()))
                } else {
                    Ok(Self::Bytes(bytes))
                }
            }
            VMType::Struct => {
                let count = reader.read_var_uint()?;
                let mut items = Vec::new();
                for _ in 0..count {
                    items.push((Self::read(reader)?, Self::read(reader)?));
                }
                Ok(Self::Struct(items))
            }
        }
    }

    pub fn vm_type(&self) -> VMType {
        match self {
            Self::None => VMType::None,
            Self::Struct(_) => VMType::Struct,
            Self::Bytes(_) => VMType::Bytes,
            Self::Number(_) => VMType::Number,
            Self::String(_) => VMType::String,
            Self::Timestamp(_) => VMType::Timestamp,
            Self::Bool(_) => VMType::Bool,
            Self::Enum(_) => VMType::Enum,
            Self::Object(_) => VMType::Object,
        }
    }

    pub fn as_number(&self) -> Result<BigInt> {
        match self {
            Self::Number(value) => Ok(value.clone()),
            // Gen2 VMObject.AsNumber treats raw byte payloads as signed
            // little-endian integers and 32-byte Object payloads as unsigned
            // hash-backed integers. Other Object payloads remain non-numeric.
            Self::Bytes(value) => Ok(vm_bytes_to_big_int(value)),
            Self::Object(value) if value.len() == 32 => {
                Ok(BigInt::from_bytes_le(Sign::Plus, value))
            }
            Self::Bool(value) => Ok(if *value {
                BigInt::from(1)
            } else {
                BigInt::zero()
            }),
            Self::String(value) => value
                .parse::<BigInt>()
                .map_err(|err| PhantasmaError::Serialization(err.to_string())),
            Self::Enum(value) | Self::Timestamp(value) => Ok(BigInt::from(*value)),
            Self::None => Ok(BigInt::zero()),
            other => serialization(format!("cannot convert {:?} to number", other.vm_type())),
        }
    }

    pub fn as_string(&self) -> Result<String> {
        match self {
            Self::String(value) => Ok(value.clone()),
            Self::Bytes(value) => String::from_utf8(value.clone())
                .map_err(|err| PhantasmaError::Serialization(err.to_string())),
            Self::Bool(value) => Ok(if *value { "true" } else { "false" }.to_string()),
            Self::Number(value) => Ok(value.to_string()),
            Self::Enum(value) | Self::Timestamp(value) => Ok(value.to_string()),
            Self::None => Ok("Null".to_string()),
            Self::Object(_) => serialization("cannot convert Object to string"),
            Self::Struct(items) => {
                if self.array_type() == VMType::Number {
                    let mut code_units = Vec::with_capacity(items.len());
                    for index in 0..items.len() {
                        let key = VMObject::Number(BigInt::from(index));
                        let value = struct_get(items, &key).ok_or_else(|| {
                            PhantasmaError::Serialization("invalid number array struct".into())
                        })?;
                        let unit = value.as_number()?.to_u16().ok_or_else(|| {
                            PhantasmaError::Serialization("UTF-16 code unit out of range".into())
                        })?;
                        code_units.push(unit);
                    }
                    return String::from_utf16(&code_units)
                        .map_err(|err| PhantasmaError::Serialization(err.to_string()));
                }
                Ok(BASE64_STANDARD.encode(self.as_bytes()?))
            }
        }
    }

    pub fn as_bytes(&self) -> Result<Vec<u8>> {
        match self {
            Self::None => serialization("cannot convert None to bytes"),
            Self::String(value) => Ok(value.as_bytes().to_vec()),
            Self::Bytes(value) | Self::Object(value) => Ok(value.clone()),
            Self::Bool(value) => Ok(vec![u8::from(*value)]),
            Self::Enum(value) | Self::Timestamp(value) => Ok(value.to_le_bytes().to_vec()),
            Self::Number(value) => crate::binary::big_int_to_vm_bytes(value),
            Self::Struct(_) => self.to_bytes(),
        }
    }

    pub fn as_bool(&self) -> Result<bool> {
        match self {
            Self::Bool(value) => Ok(*value),
            Self::Bytes(value) if value.len() == 1 => Ok(value[0] != 0),
            Self::Number(value) => Ok(!value.is_zero()),
            other => serialization(format!("cannot convert {:?} to bool", other.vm_type())),
        }
    }

    pub fn cast_to(&self, target: VMType) -> Result<Self> {
        if self.vm_type() == target {
            return Ok(self.clone());
        }
        match target {
            VMType::None => Ok(Self::None),
            VMType::String => Ok(Self::String(self.as_string()?)),
            VMType::Bytes => Ok(Self::Bytes(self.as_bytes()?)),
            VMType::Number => Ok(Self::Number(self.as_number()?)),
            VMType::Bool => Ok(Self::Bool(self.as_bool()?)),
            VMType::Struct => match self {
                Self::String(value) => Ok(Self::Struct(
                    value
                        .encode_utf16()
                        .enumerate()
                        .map(|(index, unit)| {
                            (
                                VMObject::Number(BigInt::from(index)),
                                VMObject::Number(BigInt::from(unit)),
                            )
                        })
                        .collect(),
                )),
                Self::Object(_) => Ok(self.clone()),
                other => serialization(format!("invalid cast: {:?} to Struct", other.vm_type())),
            },
            VMType::Timestamp | VMType::Enum | VMType::Object => {
                serialization(format!("invalid cast: {:?} to {target:?}", self.vm_type()))
            }
        }
    }

    pub fn array_type(&self) -> VMType {
        let Self::Struct(items) = self else {
            return VMType::None;
        };
        let mut detected = None;
        for index in 0..items.len() {
            let key = VMObject::Number(BigInt::from(index));
            let Some(value) = struct_get(items, &key) else {
                return VMType::None;
            };
            let value_type = value.vm_type();
            if detected.is_some_and(|previous| previous != value_type) {
                return VMType::None;
            }
            detected = Some(value_type);
        }
        detected.unwrap_or(VMType::None)
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut writer = BinaryWriter::new();
        self.write(&mut writer)?;
        Ok(writer.into_bytes())
    }

    pub fn write(&self, writer: &mut BinaryWriter) -> Result<()> {
        writer.write_u8(self.vm_type() as u8);
        match self {
            Self::None => {}
            Self::Struct(items) => {
                writer.write_var_uint(items.len() as u64);
                for (key, value) in items {
                    key.write(writer)?;
                    value.write(writer)?;
                }
            }
            Self::Bytes(value) => writer.write_var_bytes(value),
            Self::Object(value) => {
                let mut object_writer = BinaryWriter::new();
                object_writer.write_var_bytes(value);
                writer.write_var_bytes(object_writer.bytes());
            }
            Self::Number(value) => writer.write_big_integer(value)?,
            Self::String(value) => writer.write_string(value),
            Self::Timestamp(value) => writer.write_u32_le(*value),
            Self::Bool(value) => writer.write_bool(*value),
            Self::Enum(value) => writer.write_u8((*value).try_into().map_err(|_| {
                PhantasmaError::Serialization("enum value exceeds one byte".into())
            })?),
        }
        Ok(())
    }
}

fn struct_get<'a>(items: &'a [(VMObject, VMObject)], key: &VMObject) -> Option<&'a VMObject> {
    items
        .iter()
        .find_map(|(item_key, value)| (item_key == key).then_some(value))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptArg {
    Address(Address),
    String(String),
    Bool(bool),
    Bytes(Vec<u8>),
    Number(BigInt),
    Timestamp(u64),
    Array(Vec<ScriptArg>),
}

impl From<Address> for ScriptArg {
    fn from(value: Address) -> Self {
        Self::Address(value)
    }
}

impl From<&Address> for ScriptArg {
    fn from(value: &Address) -> Self {
        Self::Address(*value)
    }
}

impl From<String> for ScriptArg {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for ScriptArg {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl From<bool> for ScriptArg {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<Vec<u8>> for ScriptArg {
    fn from(value: Vec<u8>) -> Self {
        Self::Bytes(value)
    }
}

impl From<&[u8]> for ScriptArg {
    fn from(value: &[u8]) -> Self {
        Self::Bytes(value.to_vec())
    }
}

impl From<i64> for ScriptArg {
    fn from(value: i64) -> Self {
        Self::Number(BigInt::from(value))
    }
}

impl From<u64> for ScriptArg {
    fn from(value: u64) -> Self {
        Self::Number(BigInt::from(value))
    }
}

impl From<i32> for ScriptArg {
    fn from(value: i32) -> Self {
        Self::Number(BigInt::from(value))
    }
}

impl From<u32> for ScriptArg {
    fn from(value: u32) -> Self {
        Self::Number(BigInt::from(value))
    }
}

impl From<usize> for ScriptArg {
    fn from(value: usize) -> Self {
        Self::Number(BigInt::from(value))
    }
}

impl From<BigInt> for ScriptArg {
    fn from(value: BigInt) -> Self {
        Self::Number(value)
    }
}

impl From<Vec<ScriptArg>> for ScriptArg {
    fn from(value: Vec<ScriptArg>) -> Self {
        Self::Array(value)
    }
}

#[derive(Debug, Clone)]
pub struct ScriptBuilder {
    writer: BinaryWriter,
    jump_locations: BTreeMap<usize, String>,
    label_locations: BTreeMap<String, usize>,
    // Builder methods return `&mut Self` for chaining, so invalid
    // operations are stored and surfaced only when the final script is requested.
    error: Option<PhantasmaError>,
}

impl Default for ScriptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptBuilder {
    pub const MAX_REGISTER_COUNT: u8 = 32;

    pub fn new() -> Self {
        Self {
            writer: BinaryWriter::new(),
            jump_locations: BTreeMap::new(),
            label_locations: BTreeMap::new(),
            error: None,
        }
    }

    pub fn begin() -> Self {
        Self::new()
    }

    pub fn current_size(&self) -> usize {
        self.writer.bytes().len()
    }

    pub fn end_script(&mut self) -> Result<Vec<u8>> {
        let (script, error) = self.end_script_with_error();
        if let Some(error) = error {
            return Err(error);
        }
        Ok(script)
    }

    pub fn end_script_hex(&mut self) -> Result<String> {
        Ok(hex::encode_upper(self.end_script()?))
    }

    pub fn end_script_with_error(&mut self) -> (Vec<u8>, Option<PhantasmaError>) {
        self.emit(Opcode::Ret);
        if let Some(error) = self.error.take() {
            return (Vec::new(), Some(error));
        }
        match self.to_script() {
            Ok(script) => (script, None),
            Err(error) => (Vec::new(), Some(error)),
        }
    }

    pub fn to_script(&self) -> Result<Vec<u8>> {
        let mut script = self.writer.bytes().to_vec();
        for (offset, label) in &self.jump_locations {
            // Labels are patched after all bytes are emitted because forward
            // jumps are common in script construction.
            let normalized = label.to_ascii_lowercase();
            let Some(target) = self.label_locations.get(&normalized) else {
                return builder(format!("could not find label: {label}"));
            };
            if *offset + 1 >= script.len() {
                return builder(format!("invalid jump patch offset: {offset}"));
            }
            script[*offset..*offset + 2].copy_from_slice(&(*target as u16).to_le_bytes());
        }
        Ok(script)
    }

    pub fn emit(&mut self, opcode: Opcode) -> &mut Self {
        self.writer.write_u8(opcode as u8);
        self
    }

    pub fn emit_raw(&mut self, data: impl AsRef<[u8]>) -> &mut Self {
        self.writer.write(data);
        self
    }

    pub fn emit_push(&mut self, reg: u8) -> &mut Self {
        self.emit(Opcode::Push).byte(reg)
    }

    pub fn emit_pop(&mut self, reg: u8) -> &mut Self {
        self.emit(Opcode::Pop).byte(reg)
    }

    pub fn emit_throw(&mut self, reg: u8) -> &mut Self {
        self.emit(Opcode::Throw).byte(reg)
    }

    pub fn emit_ext_call(&mut self, method: &str, reg: u8) -> &mut Self {
        self.emit_load_string(reg, method)
            .emit(Opcode::ExtCall)
            .byte(reg)
    }

    pub fn emit_load(&mut self, reg: u8, data: impl AsRef<[u8]>, vm_type: VMType) -> &mut Self {
        let raw = data.as_ref();
        if raw.len() > 0xFFFF {
            return self.fail(format!("tried to load too much data: {} bytes", raw.len()));
        }
        self.emit(Opcode::Load);
        self.byte(reg);
        self.byte(vm_type as u8);
        self.writer.write_var_uint(raw.len() as u64);
        self.writer.write(raw);
        self
    }

    pub fn emit_load_string(&mut self, reg: u8, value: &str) -> &mut Self {
        self.emit_load(reg, value.as_bytes(), VMType::String)
    }

    pub fn emit_load_bool(&mut self, reg: u8, value: bool) -> &mut Self {
        self.emit_load(reg, [u8::from(value)], VMType::Bool)
    }

    pub fn emit_load_number(&mut self, reg: u8, value: &BigInt) -> &mut Self {
        // Gen2 ScriptBuilder emits normal C# BigInteger bytes here. This must
        // stay separate from BinaryWriter/VMObject's padded VM BigInteger
        // storage or transaction scripts drift from the reference SDKs.
        self.emit_load(reg, big_int_to_csharp_bytes(value), VMType::Number)
    }

    pub fn emit_load_timestamp(&mut self, reg: u8, unix_seconds: u64) -> &mut Self {
        let Ok(value) = u32::try_from(unix_seconds) else {
            return self.fail(format!("timestamp out of VM uint32 range: {unix_seconds}"));
        };
        self.emit_load(reg, value.to_le_bytes(), VMType::Timestamp)
    }

    pub fn emit_load_time(&mut self, reg: u8, unix_seconds: u64) -> &mut Self {
        self.emit_load_timestamp(reg, unix_seconds)
    }

    pub fn emit_move(&mut self, src_reg: u8, dst_reg: u8) -> &mut Self {
        self.emit(Opcode::Move).byte(src_reg).byte(dst_reg)
    }

    pub fn emit_copy(&mut self, src_reg: u8, dst_reg: u8) -> &mut Self {
        self.emit(Opcode::Copy).byte(src_reg).byte(dst_reg)
    }

    pub fn emit_label(&mut self, label: &str) -> &mut Self {
        self.emit(Opcode::Nop);
        self.label_locations
            .insert(label.to_ascii_lowercase(), self.current_size());
        self
    }

    pub fn emit_jump(&mut self, opcode: Opcode, label: &str, reg: u8) -> &mut Self {
        if !matches!(opcode, Opcode::Jmp | Opcode::JmpIf | Opcode::JmpNot) {
            return self.fail(format!("invalid jump opcode: {opcode:?}"));
        }
        self.emit(opcode);
        if opcode != Opcode::Jmp {
            self.byte(reg);
        }
        let offset = self.current_size();
        self.writer.write_u16_le(0);
        self.jump_locations.insert(offset, label.to_string());
        self
    }

    pub fn emit_call(&mut self, label: &str, register_count: u8) -> &mut Self {
        if register_count == 0 || register_count > Self::MAX_REGISTER_COUNT {
            return self.fail(format!("invalid number of registers: {register_count}"));
        }
        self.emit(Opcode::Call);
        self.byte(register_count);
        let offset = self.current_size();
        self.writer.write_u16_le(0);
        self.jump_locations.insert(offset, label.to_string());
        self
    }

    pub fn emit_conditional_jump(&mut self, opcode: Opcode, src_reg: u8, label: &str) -> &mut Self {
        if !matches!(opcode, Opcode::JmpIf | Opcode::JmpNot) {
            return self.fail(format!("invalid conditional jump opcode: {opcode:?}"));
        }
        self.emit_jump(opcode, label, src_reg)
    }

    pub fn emit_var_bytes(&mut self, value: u64) -> &mut Self {
        self.writer.write_var_uint(value);
        self
    }

    pub fn call_interop<I>(&mut self, method: &str, args: I) -> &mut Self
    where
        I: IntoIterator<Item = ScriptArg>,
    {
        self.insert_method_args(args);
        self.emit_load_string(0, method);
        self.emit(Opcode::ExtCall);
        self.byte(0)
    }

    pub fn call_contract<I>(&mut self, contract_name: &str, method: &str, args: I) -> &mut Self
    where
        I: IntoIterator<Item = ScriptArg>,
    {
        self.insert_method_args(args);
        self.emit_load_string(0, method);
        self.emit_push(0);
        self.emit_load_string(0, contract_name);
        self.emit(Opcode::Ctx);
        self.byte(0);
        self.byte(1);
        self.emit(Opcode::Switch);
        self.byte(1)
    }

    pub fn allow_gas(
        &mut self,
        from: Address,
        to: Address,
        gas_price: u64,
        gas_limit: u64,
    ) -> &mut Self {
        self.call_contract(
            "gas",
            "AllowGas",
            vec![from.into(), to.into(), gas_price.into(), gas_limit.into()],
        )
    }

    pub fn allow_gas_text(
        &mut self,
        from: &str,
        to: &str,
        gas_price: u64,
        gas_limit: u64,
    ) -> &mut Self {
        match (Address::from_text(from), Address::from_text(to)) {
            (Ok(from), Ok(to)) => self.allow_gas(from, to, gas_price, gas_limit),
            (Err(error), _) | (_, Err(error)) => self.fail(error.to_string()),
        }
    }

    pub fn spend_gas(&mut self, address: Address) -> &mut Self {
        self.call_contract("gas", "SpendGas", vec![address.into()])
    }

    pub fn spend_gas_text(&mut self, address: &str) -> &mut Self {
        match Address::from_text(address) {
            Ok(address) => self.spend_gas(address),
            Err(error) => self.fail(error.to_string()),
        }
    }

    pub fn transfer_tokens(
        &mut self,
        symbol: &str,
        from: Address,
        to: Address,
        amount: u64,
    ) -> &mut Self {
        self.call_interop(
            "Runtime.TransferTokens",
            vec![from.into(), to.into(), symbol.into(), amount.into()],
        )
    }

    pub fn transfer_tokens_text(
        &mut self,
        symbol: &str,
        from: &str,
        to: &str,
        amount: u64,
    ) -> &mut Self {
        match (Address::from_text(from), Address::from_text(to)) {
            (Ok(from), Ok(to)) => self.transfer_tokens(symbol, from, to, amount),
            (Err(error), _) | (_, Err(error)) => self.fail(error.to_string()),
        }
    }

    pub fn transfer_tokens_to_text(
        &mut self,
        symbol: &str,
        from: Address,
        to: &str,
        amount: u64,
    ) -> &mut Self {
        match Address::from_text(to) {
            Ok(to) => self.transfer_tokens(symbol, from, to, amount),
            Err(error) => self.fail(error.to_string()),
        }
    }

    pub fn mint_tokens(
        &mut self,
        symbol: &str,
        from: Address,
        to: Address,
        amount: u64,
    ) -> &mut Self {
        self.call_interop(
            "Runtime.MintTokens",
            vec![from.into(), to.into(), symbol.into(), amount.into()],
        )
    }

    pub fn mint_tokens_text(
        &mut self,
        symbol: &str,
        from: &str,
        to: &str,
        amount: u64,
    ) -> &mut Self {
        match (Address::from_text(from), Address::from_text(to)) {
            (Ok(from), Ok(to)) => self.mint_tokens(symbol, from, to, amount),
            (Err(error), _) | (_, Err(error)) => self.fail(error.to_string()),
        }
    }

    pub fn transfer_balance(&mut self, symbol: &str, from: Address, to: Address) -> &mut Self {
        self.call_interop(
            "Runtime.TransferBalance",
            vec![from.into(), to.into(), symbol.into()],
        )
    }

    pub fn transfer_balance_text(&mut self, symbol: &str, from: &str, to: &str) -> &mut Self {
        match (Address::from_text(from), Address::from_text(to)) {
            (Ok(from), Ok(to)) => self.transfer_balance(symbol, from, to),
            (Err(error), _) | (_, Err(error)) => self.fail(error.to_string()),
        }
    }

    pub fn transfer_nft(
        &mut self,
        symbol: &str,
        from: Address,
        to: Address,
        token_id: u64,
    ) -> &mut Self {
        self.call_interop(
            "Runtime.TransferToken",
            vec![from.into(), to.into(), symbol.into(), token_id.into()],
        )
    }

    pub fn transfer_nft_text(
        &mut self,
        symbol: &str,
        from: &str,
        to: &str,
        token_id: u64,
    ) -> &mut Self {
        match (Address::from_text(from), Address::from_text(to)) {
            (Ok(from), Ok(to)) => self.transfer_nft(symbol, from, to, token_id),
            (Err(error), _) | (_, Err(error)) => self.fail(error.to_string()),
        }
    }

    pub fn transfer_nft_to_text(
        &mut self,
        symbol: &str,
        from: Address,
        to: &str,
        token_id: u64,
    ) -> &mut Self {
        match Address::from_text(to) {
            Ok(to) => self.transfer_nft(symbol, from, to, token_id),
            Err(error) => self.fail(error.to_string()),
        }
    }

    pub fn cross_transfer_token(
        &mut self,
        destination_chain: Address,
        symbol: &str,
        from: Address,
        to: Address,
        amount: u64,
    ) -> &mut Self {
        self.call_interop(
            "Runtime.SendTokens",
            vec![
                destination_chain.into(),
                from.into(),
                to.into(),
                symbol.into(),
                amount.into(),
            ],
        )
    }

    pub fn cross_transfer_token_text(
        &mut self,
        destination_chain: &str,
        symbol: &str,
        from: &str,
        to: &str,
        amount: u64,
    ) -> &mut Self {
        match (
            Address::from_text(destination_chain),
            Address::from_text(from),
            Address::from_text(to),
        ) {
            (Ok(destination_chain), Ok(from), Ok(to)) => {
                self.cross_transfer_token(destination_chain, symbol, from, to, amount)
            }
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                self.fail(error.to_string())
            }
        }
    }

    pub fn cross_transfer_token_to_text(
        &mut self,
        destination_chain: Address,
        symbol: &str,
        from: Address,
        to: &str,
        amount: u64,
    ) -> &mut Self {
        match Address::from_text(to) {
            Ok(to) => self.cross_transfer_token(destination_chain, symbol, from, to, amount),
            Err(error) => self.fail(error.to_string()),
        }
    }

    pub fn cross_transfer_nft(
        &mut self,
        destination_chain: Address,
        symbol: &str,
        from: Address,
        to: Address,
        token_id: u64,
    ) -> &mut Self {
        self.call_interop(
            "Runtime.SendToken",
            vec![
                destination_chain.into(),
                from.into(),
                to.into(),
                symbol.into(),
                token_id.into(),
            ],
        )
    }

    pub fn cross_transfer_nft_text(
        &mut self,
        destination_chain: &str,
        symbol: &str,
        from: &str,
        to: &str,
        token_id: u64,
    ) -> &mut Self {
        match (
            Address::from_text(destination_chain),
            Address::from_text(from),
            Address::from_text(to),
        ) {
            (Ok(destination_chain), Ok(from), Ok(to)) => {
                self.cross_transfer_nft(destination_chain, symbol, from, to, token_id)
            }
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                self.fail(error.to_string())
            }
        }
    }

    pub fn cross_transfer_nft_to_text(
        &mut self,
        destination_chain: Address,
        symbol: &str,
        from: Address,
        to: &str,
        token_id: u64,
    ) -> &mut Self {
        match Address::from_text(to) {
            Ok(to) => self.cross_transfer_nft(destination_chain, symbol, from, to, token_id),
            Err(error) => self.fail(error.to_string()),
        }
    }

    pub fn stake(&mut self, address: Address, amount: u64) -> &mut Self {
        self.call_contract("stake", "Stake", vec![address.into(), amount.into()])
    }

    pub fn stake_text(&mut self, address: &str, amount: u64) -> &mut Self {
        match Address::from_text(address) {
            Ok(address) => self.stake(address, amount),
            Err(error) => self.fail(error.to_string()),
        }
    }

    pub fn unstake(&mut self, address: Address, amount: u64) -> &mut Self {
        self.call_contract("stake", "Unstake", vec![address.into(), amount.into()])
    }

    pub fn unstake_text(&mut self, address: &str, amount: u64) -> &mut Self {
        match Address::from_text(address) {
            Ok(address) => self.unstake(address, amount),
            Err(error) => self.fail(error.to_string()),
        }
    }

    pub fn call_nft<I>(&mut self, symbol: &str, series_id: u64, method: &str, args: I) -> &mut Self
    where
        I: IntoIterator<Item = ScriptArg>,
    {
        self.call_contract(&format!("{symbol}#{series_id}"), method, args)
    }

    fn insert_method_args<I>(&mut self, args: I)
    where
        I: IntoIterator<Item = ScriptArg>,
    {
        let args: Vec<ScriptArg> = args.into_iter().collect();
        for arg in args.into_iter().rev() {
            self.load_into_register(0, &arg);
            self.emit_push(0);
        }
    }

    fn load_into_register(&mut self, reg: u8, value: &ScriptArg) {
        match value {
            ScriptArg::Address(address) => {
                self.emit_load(reg, address.prefixed_bytes(), VMType::Bytes);
            }
            ScriptArg::String(value) => {
                self.emit_load_string(reg, value);
            }
            ScriptArg::Bool(value) => {
                self.emit_load_bool(reg, *value);
            }
            ScriptArg::Bytes(value) => {
                self.emit_load(reg, value, VMType::Bytes);
            }
            ScriptArg::Number(value) => {
                self.emit_load_number(reg, value);
            }
            ScriptArg::Timestamp(value) => {
                self.emit_load_timestamp(reg, *value);
            }
            ScriptArg::Array(values) => {
                if reg > Self::MAX_REGISTER_COUNT - 3 {
                    self.fail(format!(
                        "array load needs three registers starting at {reg}"
                    ));
                    return;
                }
                self.emit(Opcode::Cast);
                self.byte(reg).byte(reg).byte(VMType::None as u8);
                for (index, item) in values.iter().enumerate() {
                    self.load_into_register(reg + 1, item);
                    self.load_into_register(reg + 2, &ScriptArg::Number(BigInt::from(index)));
                    self.emit(Opcode::Put);
                    self.byte(reg + 1).byte(reg).byte(reg + 2);
                }
            }
        }
    }

    fn byte(&mut self, value: u8) -> &mut Self {
        self.writer.write_u8(value);
        self
    }

    fn fail(&mut self, message: impl Into<String>) -> &mut Self {
        if self.error.is_none() {
            self.error = Some(PhantasmaError::Builder(message.into()));
        }
        self
    }
}

pub fn script_arg_number_to_i64(value: &ScriptArg) -> Option<i64> {
    match value {
        ScriptArg::Number(value) => value.to_i64(),
        _ => None,
    }
}
