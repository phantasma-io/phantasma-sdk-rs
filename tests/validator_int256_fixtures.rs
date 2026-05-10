use std::collections::HashMap;

use num_bigint::BigInt;
use phantasma_sdk::{
    deserialize, serialize, Bytes32, CarbonReader, CarbonWriter, IntX, SeriesInfo, SmallString,
    TokenFlags, TokenInfo, VMDynamicStruct, VMDynamicVariable, VMNamedDynamicVariable,
    VMStructSchema, VMType, VMValue,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureBundle {
    int256: Vec<Int256Fixture>,
    intx: Vec<IntXFixture>,
    vm_dynamic_int256: Vec<VmDynamicInt256Fixture>,
    vm_dynamic_int256_array: Vec<VmDynamicInt256ArrayFixture>,
    metadata_structs: Vec<MetadataStructFixture>,
    token_info: Vec<TokenInfoFixture>,
    series_info: Vec<SeriesInfoFixture>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Int256Fixture {
    id: String,
    source_dec: String,
    read_back_signed_dec: String,
    wire_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IntXFixture {
    id: String,
    source_dec: String,
    read_back_dec: String,
    wire_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VmDynamicInt256Fixture {
    id: String,
    source_dec: String,
    wire_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VmDynamicInt256ArrayFixture {
    id: String,
    values: Vec<String>,
    wire_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MetadataStructFixture {
    id: String,
    shape: String,
    #[serde(rename = "_iDec")]
    meta_id_dec: String,
    mode: Option<i64>,
    rom_hex: String,
    wire_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenInfoFixture {
    id: String,
    max_supply_dec: String,
    flags: u8,
    decimals: u8,
    symbol: String,
    metadata_hex: String,
    wire_hex: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SeriesInfoFixture {
    id: String,
    max_mint: u32,
    max_supply: u32,
    metadata_hex: String,
    wire_hex: String,
}

#[test]
fn raw_int256_matches_validator_fixtures() {
    let fixtures = load_fixtures();
    for fixture in fixtures.int256 {
        let mut writer = CarbonWriter::new();
        writer
            .write_big_int(&parse_bigint(&fixture.source_dec))
            .unwrap();
        assert_eq!(
            hex::encode_upper(writer.into_bytes()),
            fixture.wire_hex,
            "{} encode",
            fixture.id
        );

        let raw = hex::decode(&fixture.wire_hex).unwrap();
        let mut reader = CarbonReader::new(&raw);
        assert_eq!(
            reader.read_big_int().unwrap().to_string(),
            fixture.read_back_signed_dec,
            "{} decode",
            fixture.id
        );
        reader.assert_eof().unwrap();
    }
}

#[test]
fn intx_matches_validator_fixtures() {
    let fixtures = load_fixtures();
    for fixture in fixtures.intx {
        let value = IntX::new(parse_bigint(&fixture.source_dec));
        assert_eq!(
            hex::encode_upper(serialize(&value).unwrap()),
            fixture.wire_hex,
            "{} encode",
            fixture.id
        );

        let decoded: IntX = deserialize(hex::decode(&fixture.wire_hex).unwrap()).unwrap();
        assert_eq!(
            decoded.to_string(),
            fixture.read_back_dec,
            "{} decode",
            fixture.id
        );
    }
}

#[test]
fn vm_dynamic_int256_matches_validator_fixtures() {
    let fixtures = load_fixtures();
    let readback = int256_readback_by_source(&fixtures);
    for fixture in fixtures.vm_dynamic_int256 {
        let value = VMDynamicVariable::int256(parse_bigint(&fixture.source_dec));
        assert_eq!(
            hex::encode_upper(serialize(&value).unwrap()),
            fixture.wire_hex,
            "{} encode",
            fixture.id
        );

        let decoded: VMDynamicVariable =
            deserialize(hex::decode(&fixture.wire_hex).unwrap()).unwrap();
        assert_eq!(decoded.vm_type, VMType::Int256, "{} type", fixture.id);
        let VMValue::Int256(value) = decoded.data else {
            panic!("{} expected Int256", fixture.id);
        };
        assert_eq!(
            value.to_string(),
            readback[&fixture.source_dec],
            "{} decode",
            fixture.id
        );
    }
}

#[test]
fn vm_dynamic_int256_array_matches_validator_fixtures() {
    let fixtures = load_fixtures();
    let readback = int256_readback_by_source(&fixtures);
    for fixture in fixtures.vm_dynamic_int256_array {
        let values = fixture
            .values
            .iter()
            .map(|value| parse_bigint(value))
            .collect::<Vec<_>>();
        let value = VMDynamicVariable::new(VMType::ArrayInt256, VMValue::ArrayInt256(values));
        assert_eq!(
            hex::encode_upper(serialize(&value).unwrap()),
            fixture.wire_hex,
            "{} encode",
            fixture.id
        );

        let decoded: VMDynamicVariable =
            deserialize(hex::decode(&fixture.wire_hex).unwrap()).unwrap();
        assert_eq!(decoded.vm_type, VMType::ArrayInt256, "{} type", fixture.id);
        let VMValue::ArrayInt256(values) = decoded.data else {
            panic!("{} expected ArrayInt256", fixture.id);
        };
        let actual = values.iter().map(ToString::to_string).collect::<Vec<_>>();
        let expected = fixture
            .values
            .iter()
            .map(|value| readback[value].clone())
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{} decode", fixture.id);
    }
}

#[test]
fn metadata_structs_match_validator_fixtures() {
    let fixtures = load_fixtures();
    for fixture in fixtures.metadata_structs {
        let value = build_metadata_struct(&fixture);
        assert_eq!(
            hex::encode_upper(serialize(&value).unwrap()),
            fixture.wire_hex,
            "{} encode",
            fixture.id
        );

        let decoded: VMDynamicStruct =
            deserialize(hex::decode(&fixture.wire_hex).unwrap()).unwrap();
        let field_names = decoded
            .fields
            .iter()
            .map(|field| field.name.0.as_str())
            .collect::<Vec<_>>();
        if fixture.shape == "nft-default" {
            assert_eq!(field_names, ["_i", "rom"], "{} fields", fixture.id);
        } else {
            assert_eq!(field_names, ["_i", "mode", "rom"], "{} fields", fixture.id);
        }
        let VMValue::Int256(meta_id) = &decoded.get("_i").unwrap().data else {
            panic!("{} expected _i Int256", fixture.id);
        };
        assert_eq!(
            meta_id.to_string(),
            fixture.meta_id_dec,
            "{} _i",
            fixture.id
        );

        let VMValue::Bytes(rom) = &decoded.get("rom").unwrap().data else {
            panic!("{} expected rom bytes", fixture.id);
        };
        assert_eq!(
            hex::encode_upper(rom),
            fixture.rom_hex,
            "{} rom",
            fixture.id
        );

        if let Some(mode) = fixture.mode {
            let VMValue::Int(actual_mode) = decoded.get("mode").unwrap().data else {
                panic!("{} expected mode Int8", fixture.id);
            };
            assert_eq!(actual_mode, mode, "{} mode", fixture.id);
        }
    }
}

#[test]
fn token_info_matches_validator_fixtures() {
    let fixtures = load_fixtures();
    for fixture in fixtures.token_info {
        let value = build_token_info(&fixture);
        assert_eq!(
            hex::encode_upper(serialize(&value).unwrap()),
            fixture.wire_hex,
            "{} encode",
            fixture.id
        );

        let decoded: TokenInfo = deserialize(hex::decode(&fixture.wire_hex).unwrap()).unwrap();
        assert_eq!(
            decoded.max_supply.to_string(),
            fixture.max_supply_dec,
            "{} supply",
            fixture.id
        );
        assert_eq!(decoded.flags.bits(), fixture.flags, "{} flags", fixture.id);
        assert_eq!(
            decoded.decimals, fixture.decimals,
            "{} decimals",
            fixture.id
        );
        assert_eq!(
            decoded.owner,
            expected_token_owner(&fixture.id),
            "{} owner",
            fixture.id
        );
        assert_eq!(decoded.symbol.0, fixture.symbol, "{} symbol", fixture.id);
        assert_eq!(
            hex::encode_upper(decoded.metadata),
            fixture.metadata_hex,
            "{} metadata",
            fixture.id
        );
    }
}

#[test]
fn series_info_matches_validator_fixtures() {
    let fixtures = load_fixtures();
    for fixture in fixtures.series_info {
        let value = build_series_info(&fixture);
        assert_eq!(
            hex::encode_upper(serialize(&value).unwrap()),
            fixture.wire_hex,
            "{} encode",
            fixture.id
        );

        let decoded: SeriesInfo = deserialize(hex::decode(&fixture.wire_hex).unwrap()).unwrap();
        assert_eq!(
            decoded.max_mint, fixture.max_mint,
            "{} max_mint",
            fixture.id
        );
        assert_eq!(
            decoded.max_supply, fixture.max_supply,
            "{} max_supply",
            fixture.id
        );
        assert_eq!(
            decoded.owner,
            expected_series_owner(&fixture.id),
            "{} owner",
            fixture.id
        );
        assert_eq!(
            hex::encode_upper(&decoded.metadata),
            fixture.metadata_hex,
            "{} metadata",
            fixture.id
        );
        assert!(decoded.rom.fields.is_empty(), "{} rom schema", fixture.id);
        assert!(decoded.ram.fields.is_empty(), "{} ram schema", fixture.id);

        let metadata: VMDynamicStruct = deserialize(decoded.metadata).unwrap();
        let VMValue::Int256(meta_id) = &metadata.get("_i").unwrap().data else {
            panic!("{} expected metadata _i Int256", fixture.id);
        };
        assert_eq!(
            meta_id.to_string(),
            expected_series_metadata_id(&fixture.id),
            "{} metadata _i",
            fixture.id
        );
    }
}

fn load_fixtures() -> FixtureBundle {
    serde_json::from_str(
        &std::fs::read_to_string("tests/fixtures/validator_int256_fixtures.json").unwrap(),
    )
    .unwrap()
}

fn parse_bigint(value: &str) -> BigInt {
    BigInt::parse_bytes(value.as_bytes(), 10).unwrap()
}

fn int256_readback_by_source(fixtures: &FixtureBundle) -> HashMap<String, String> {
    fixtures
        .int256
        .iter()
        .map(|fixture| {
            (
                fixture.source_dec.clone(),
                fixture.read_back_signed_dec.clone(),
            )
        })
        .collect()
}

fn build_metadata_struct(fixture: &MetadataStructFixture) -> VMDynamicStruct {
    let mut fields = vec![
        VMNamedDynamicVariable::make(
            "_i",
            VMType::Int256,
            VMValue::Int256(parse_bigint(&fixture.meta_id_dec)),
        ),
        VMNamedDynamicVariable::make(
            "rom",
            VMType::Bytes,
            VMValue::Bytes(hex::decode(&fixture.rom_hex).unwrap()),
        ),
    ];
    if let Some(mode) = fixture.mode {
        fields.push(VMNamedDynamicVariable::make(
            "mode",
            VMType::Int8,
            VMValue::Int(mode),
        ));
    }
    VMDynamicStruct::new(fields)
}

fn build_token_info(fixture: &TokenInfoFixture) -> TokenInfo {
    TokenInfo {
        max_supply: IntX::new(parse_bigint(&fixture.max_supply_dec)),
        flags: TokenFlags::from_bits_retain(fixture.flags),
        decimals: fixture.decimals,
        owner: expected_token_owner(&fixture.id),
        symbol: SmallString::new(fixture.symbol.clone()).unwrap(),
        metadata: hex::decode(&fixture.metadata_hex).unwrap(),
        token_schemas: Vec::new(),
    }
}

fn build_series_info(fixture: &SeriesInfoFixture) -> SeriesInfo {
    SeriesInfo {
        max_mint: fixture.max_mint,
        max_supply: fixture.max_supply,
        owner: expected_series_owner(&fixture.id),
        metadata: hex::decode(&fixture.metadata_hex).unwrap(),
        rom: VMStructSchema::default(),
        ram: VMStructSchema::default(),
    }
}

fn expected_token_owner(id: &str) -> Bytes32 {
    match id {
        "fungible_zero_supply" => pattern_bytes32(0x10),
        "big_fungible_u64max_supply" => pattern_bytes32(0x20),
        _ => panic!("unknown token fixture id: {id}"),
    }
}

fn expected_series_owner(id: &str) -> Bytes32 {
    match id {
        "series_zero_metaid" => pattern_bytes32(0x30),
        "series_problematic_metaid" => pattern_bytes32(0x40),
        _ => panic!("unknown series fixture id: {id}"),
    }
}

fn expected_series_metadata_id(id: &str) -> &'static str {
    match id {
        "series_zero_metaid" => "0",
        "series_problematic_metaid" => {
            "342701406799689386264365071881606655601301200422094937311139938246178500459"
        }
        _ => panic!("unknown series fixture id: {id}"),
    }
}

fn pattern_bytes32(seed: u8) -> Bytes32 {
    let mut bytes = [0; 32];
    for (index, value) in bytes.iter_mut().enumerate() {
        *value = seed + index as u8;
    }
    Bytes32(bytes)
}
