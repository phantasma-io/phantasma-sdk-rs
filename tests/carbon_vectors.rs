use std::fs;

use num_bigint::BigInt;
use phantasma_sdk::{
    deserialize, serialize, Bytes16, Bytes32, Bytes64, CarbonReader, CarbonSerializable,
    CarbonWriter, IntX, SignedTxMsg, TokenSchemas, TxMsg, VMDynamicStruct,
};

fn parse_byte_arrays(value: &str) -> Vec<Vec<u8>> {
    let value = value.trim_start_matches("[[").trim_end_matches("]]");
    if value.is_empty() {
        return Vec::new();
    }
    value
        .split("],[")
        .map(|part| hex::decode(part.replace(',', "")).unwrap())
        .collect()
}

#[test]
fn carbon_shared_vectors_match_reference_sdks() {
    // Shared TSV vectors catch byte-order, signedness, and Carbon packing drift.
    let data = fs::read_to_string("tests/fixtures/carbon_vectors.tsv").unwrap();
    for line in data.lines().filter(|line| !line.trim().is_empty()) {
        let parts: Vec<_> = line.split('\t').collect();
        let kind = parts[0];
        let value = parts[1];
        let expected = parts[2];
        let expected_read_value = if parts.len() >= 5 { parts[4] } else { value };
        let mut writer = CarbonWriter::new();
        match kind {
            "U8" => {
                let value: u8 = value.parse().unwrap();
                writer.write1(value);
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read1()
                        .unwrap(),
                    value
                );
            }
            "I16" => {
                let value: i16 = value.parse().unwrap();
                writer.write2(value);
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read2()
                        .unwrap(),
                    value
                );
            }
            "I32" => {
                let value: i32 = value.parse().unwrap();
                writer.write4(value);
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read4()
                        .unwrap(),
                    value
                );
            }
            "U32" => {
                let value: u32 = value.parse().unwrap();
                writer.write4u(value);
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read4u()
                        .unwrap(),
                    value
                );
            }
            "I64" => {
                let value: i64 = value.parse().unwrap();
                writer.write8(value);
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read8()
                        .unwrap(),
                    value
                );
            }
            "U64" => {
                let value: u64 = value.parse().unwrap();
                writer.write8u(value);
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read8u()
                        .unwrap(),
                    value
                );
            }
            "FIX16" => {
                writer.write16(Bytes16::from_hex(value).unwrap());
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read16()
                        .unwrap(),
                    Bytes16::from_hex(value).unwrap()
                );
            }
            "FIX32" => {
                writer.write32(Bytes32::from_hex(value).unwrap());
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read32()
                        .unwrap(),
                    Bytes32::from_hex(value).unwrap()
                );
            }
            "FIX64" => {
                writer.write64(Bytes64::from_hex(value).unwrap());
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read64()
                        .unwrap(),
                    Bytes64::from_hex(value).unwrap()
                );
            }
            "SZ" => {
                writer.write_string_z(value).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_string_z()
                        .unwrap(),
                    value
                );
            }
            "ARRSZ" => {
                let values: Vec<String> = value.split(',').map(str::to_string).collect();
                writer.write_string_z_array(&values).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_string_z_array()
                        .unwrap(),
                    values
                );
            }
            "ARR8" => {
                let values: Vec<i8> = value.split(',').map(|item| item.parse().unwrap()).collect();
                writer.write_i8_array(&values).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_i8_array()
                        .unwrap(),
                    values
                );
            }
            "ARR16" => {
                let values: Vec<i16> = value.split(',').map(|item| item.parse().unwrap()).collect();
                writer.write_i16_array(&values).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_i16_array()
                        .unwrap(),
                    values
                );
            }
            "ARR32" => {
                let values: Vec<i32> = value.split(',').map(|item| item.parse().unwrap()).collect();
                writer.write_i32_array(&values).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_i32_array()
                        .unwrap(),
                    values
                );
            }
            "ARR64" => {
                let values: Vec<i64> = value.split(',').map(|item| item.parse().unwrap()).collect();
                writer.write_i64_array(&values).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_i64_array()
                        .unwrap(),
                    values
                );
            }
            "ARRU64" => {
                let values: Vec<u64> = value.split(',').map(|item| item.parse().unwrap()).collect();
                writer.write_u64_array(&values).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_u64_array()
                        .unwrap(),
                    values
                );
            }
            "ARRBYTES-1D" => {
                writer
                    .write_byte_array(hex::decode(value).unwrap())
                    .unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_byte_array()
                        .unwrap(),
                    hex::decode(value).unwrap()
                );
            }
            "ARRBYTES-2D" => {
                let values = parse_byte_arrays(value);
                writer.write_byte_arrays(&values).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_byte_arrays()
                        .unwrap(),
                    values
                );
            }
            "BI" => {
                let value: BigInt = value.parse().unwrap();
                let expected_read_value: BigInt = expected_read_value.parse().unwrap();
                writer.write_big_int(&value).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_big_int()
                        .unwrap(),
                    expected_read_value
                );
            }
            "INTX" => {
                let value: BigInt = value.parse().unwrap();
                let expected_read_value: BigInt = expected_read_value.parse().unwrap();
                IntX(value.clone()).write_carbon(&mut writer).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    IntX::read_carbon(&mut CarbonReader::new(&hex::decode(expected).unwrap()))
                        .unwrap()
                        .0,
                    expected_read_value
                );
            }
            "ARRBI" => {
                let values: Vec<BigInt> =
                    value.split(',').map(|item| item.parse().unwrap()).collect();
                writer.write_big_int_array(&values).unwrap();
                assert_eq!(hex::encode_upper(writer.bytes()), expected);
                assert_eq!(
                    CarbonReader::new(&hex::decode(expected).unwrap())
                        .read_big_int_array()
                        .unwrap(),
                    values
                );
            }
            "TX1" | "TX-CREATE-TOKEN" | "TX-CREATE-TOKEN-SERIES" | "TX-MINT-NON-FUNGIBLE" => {
                let msg: TxMsg = deserialize(hex::decode(expected).unwrap()).unwrap();
                assert_eq!(hex::encode_upper(serialize(&msg).unwrap()), expected);
            }
            "TX2" => {
                let msg: SignedTxMsg = deserialize(hex::decode(expected).unwrap()).unwrap();
                assert_eq!(hex::encode_upper(serialize(&msg).unwrap()), expected);
            }
            "VMSTRUCT01" => {
                let value: TokenSchemas = deserialize(hex::decode(expected).unwrap()).unwrap();
                assert_eq!(hex::encode_upper(serialize(&value).unwrap()), expected);
            }
            "VMSTRUCT02" => {
                let value: VMDynamicStruct = deserialize(hex::decode(expected).unwrap()).unwrap();
                assert_eq!(hex::encode_upper(serialize(&value).unwrap()), expected);
            }
            other => panic!("unhandled vector kind: {other}"),
        }
    }
}
