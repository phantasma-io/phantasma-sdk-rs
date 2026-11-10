//! Token schemas JSON with struct fields. A Struct or Array_Struct field carries the schema of its
//! struct, and the chain writes the field's values by it. The JSON of token schemas names that
//! struct's fields under "fields", in the same form as the other fields.

use phantasma_sdk::{
    build_phantasma_nft_rom, build_token_schemas_from_fields, parse_token_schemas_json,
    serialize_token_schemas, token_schemas_from_json, PhantasmaError, TokenSchemaField,
    VMDynamicStruct, VMNamedDynamicVariable, VMNamedVariableSchema, VMStructArray, VMStructSchema,
    VMType, VMValue,
};

/// The fields every NFT token schema needs, so that `token_schemas_from_json` accepts the schemas.
const STANDARD_ROM_FIELDS: &str = r#"
    { "name": "name", "type": "String" },
    { "name": "description", "type": "String" },
    { "name": "imageURL", "type": "String" },
    { "name": "infoURL", "type": "String" },
    { "name": "royalties", "type": "Int32" }"#;

const STATS: &str = r#"{
    "name": "stats",
    "type": "Struct",
    "fields": [
        { "name": "level", "type": "Int32" },
        { "name": "class", "type": "String" }
    ]
}"#;

const ITEMS: &str =
    r#"{ "name": "items", "type": "Array_Struct", "fields": [{ "name": "id", "type": "Int64" }] }"#;

/// Returns the token schemas JSON with the standard ROM fields and `extra_fields` in `rom`.
fn schemas_json(extra_fields: &str) -> String {
    format!(
        r#"{{ "seriesMetadata": [], "rom": [{STANDARD_ROM_FIELDS}, {extra_fields}], "ram": [] }}"#
    )
}

/// Returns the message with which `token_schemas_from_json` refuses `extra_fields` in `rom`.
fn refusal(extra_fields: &str) -> String {
    match token_schemas_from_json(&schemas_json(extra_fields)).unwrap_err() {
        PhantasmaError::Builder(message) => message,
        other => panic!("expected a builder error, got {other:?}"),
    }
}

fn stats_by_hand() -> VMNamedVariableSchema {
    VMNamedVariableSchema::with_struct(
        "stats",
        VMType::Struct,
        VMStructSchema::new(vec![
            VMNamedVariableSchema::new("level", VMType::Int32),
            VMNamedVariableSchema::new("class", VMType::String),
        ]),
    )
}

// The schemas the TypeScript SDK serializes for the same JSON. A struct schema is its field count,
// each field as a length-prefixed name and a type byte, then a flags byte. A Struct (0x04) or
// Array_Struct (0x05) field follows its type byte with the schema of its struct.
#[test]
fn serializes_struct_and_array_struct_fields_with_the_schemas_of_their_structs() {
    let schemas = token_schemas_from_json(&schemas_json(&format!("{STATS}, {ITEMS}"))).unwrap();
    let expected = concat!(
        // seriesMetadata: _i, mode and rom, which the SDK adds, and no flags.
        "03000000",
        "025F690E",
        "046D6F646506",
        "03726F6D02",
        "00",
        // rom: _i and rom, the standard fields, stats and items.
        "09000000",
        "025F690E",
        "03726F6D02",
        "046E616D6516",
        "0B6465736372697074696F6E16",
        "08696D61676555524C16",
        "07696E666F55524C16",
        "09726F79616C746965730A",
        // stats is a Struct (0x04). Its struct has two fields and no flags.
        "057374617473",
        "04",
        "02000000",
        "056C6576656C0A",
        "05636C61737316",
        "00",
        // items is an Array_Struct (0x05). Its struct has one field and no flags.
        "056974656D73",
        "05",
        "01000000",
        "0269640C",
        "00",
        // The flags of rom.
        "00",
        // ram: no fields, and the flag for fields that are not in the schema.
        "00000000",
        "01",
    );
    assert_eq!(
        hex::encode_upper(serialize_token_schemas(&schemas).unwrap()),
        expected
    );
}

// NFT data built by the ROM schema from JSON holds the values only, in the order of the schema, the
// struct fields by the schemas of their structs. The bytes are the TypeScript SDK's for the same
// input. The fields of a struct can come in any order.
#[test]
fn builds_the_nft_data_of_struct_fields_by_the_schema_from_json() {
    let schemas = token_schemas_from_json(&schemas_json(&format!("{STATS}, {ITEMS}"))).unwrap();
    let id = |value: VMValue| {
        VMDynamicStruct::new(vec![VMNamedDynamicVariable::make(
            "id",
            VMType::Int64,
            value,
        )])
    };
    let rom = build_phantasma_nft_rom(
        &schemas.rom,
        &[
            ("name", VMValue::String("Hero".into())),
            (
                "description",
                VMValue::String("An NFT with a struct field".into()),
            ),
            (
                "imageURL",
                VMValue::String("https://example.invalid/hero.png".into()),
            ),
            (
                "infoURL",
                VMValue::String("https://example.invalid/hero".into()),
            ),
            ("royalties", VMValue::Int(0)),
            (
                "stats",
                VMValue::Struct(VMDynamicStruct::new(vec![
                    VMNamedDynamicVariable::make(
                        "class",
                        VMType::String,
                        VMValue::String("mage".into()),
                    ),
                    VMNamedDynamicVariable::make("level", VMType::Int32, VMValue::Int(3)),
                ])),
            ),
            (
                "items",
                VMValue::ArrayStruct(VMStructArray {
                    schema: VMStructSchema::new(vec![VMNamedVariableSchema::new(
                        "id",
                        VMType::Int64,
                    )]),
                    structs: vec![id(VMValue::Int(9_007_199_254_740_993)), id(VMValue::Int(2))],
                }),
            ),
        ],
    )
    .unwrap();
    let expected = concat!(
        // The strings are zero-terminated, royalties is four little-endian bytes.
        "4865726F00",
        "416E204E46542077697468206120737472756374206669656C6400",
        "68747470733A2F2F6578616D706C652E696E76616C69642F6865726F2E706E6700",
        "68747470733A2F2F6578616D706C652E696E76616C69642F6865726F00",
        "00000000",
        // stats: level, then class, as the struct's schema orders them.
        "03000000",
        "6D61676500",
        // items: the count, then each id in eight little-endian bytes.
        "02000000",
        "0100000000002000",
        "0200000000000000",
    );
    assert_eq!(hex::encode_upper(rom), expected);
}

// The struct of a Struct field can hold a Struct field itself. Its schema is built at every level.
#[test]
fn builds_a_struct_field_inside_a_struct_field() {
    let outer = format!(r#"{{ "name": "outer", "type": "Struct", "fields": [{STATS}] }}"#);
    let schemas = token_schemas_from_json(&schemas_json(&outer)).unwrap();
    assert_eq!(
        schemas.rom.fields.last().unwrap(),
        &VMNamedVariableSchema::with_struct(
            "outer",
            VMType::Struct,
            VMStructSchema::new(vec![stats_by_hand()]),
        )
    );
}

// The parsed JSON keeps the fields of each struct, so `build_token_schemas_from_fields` builds the
// same schemas from it.
#[test]
fn parses_the_fields_of_a_struct_field_into_the_json_shape() {
    let json = schemas_json(STATS);
    let parsed = parse_token_schemas_json(&json).unwrap();
    assert_eq!(
        parsed.rom.last().unwrap(),
        &TokenSchemaField::with_fields(
            "stats",
            VMType::Struct,
            vec![
                TokenSchemaField::new("level", VMType::Int32),
                TokenSchemaField::new("class", VMType::String),
            ],
        )
    );
    assert_eq!(
        build_token_schemas_from_fields(&parsed.series_metadata, &parsed.rom, &parsed.ram).unwrap(),
        token_schemas_from_json(&json).unwrap()
    );
}

#[test]
fn refuses_a_struct_field_without_fields() {
    assert_eq!(
        refusal(r#"{ "name": "stats", "type": "Struct" }"#),
        "rom.stats has the type Struct and needs a fields array"
    );
    assert_eq!(
        refusal(r#"{ "name": "stats", "type": "Struct", "fields": {} }"#),
        "rom.stats has the type Struct and needs a fields array"
    );
}

#[test]
fn refuses_an_array_struct_field_whose_struct_has_no_fields() {
    assert_eq!(
        refusal(r#"{ "name": "items", "type": "Array_Struct", "fields": [] }"#),
        "rom.items has the type Array_Struct and needs at least one field"
    );
}

#[test]
fn refuses_fields_on_a_field_of_another_type() {
    assert_eq!(
        refusal(r#"{ "name": "title", "type": "String", "fields": [] }"#),
        "rom.title has the type String and cannot have fields"
    );
}

// A message names the list a field stands in, then each struct field around it.
#[test]
fn names_the_struct_field_when_one_of_its_fields_is_malformed() {
    assert_eq!(refusal("7"), "rom field must be an object");
    let malformed = |field: &str| {
        refusal(&format!(
            r#"{{ "name": "stats", "type": "Struct", "fields": [{field}] }}"#
        ))
    };
    assert_eq!(malformed("7"), "rom.stats field must be an object");
    assert_eq!(
        malformed(r#"{ "name": 5, "type": "Int32" }"#),
        "rom.stats field name must be string"
    );
    assert_eq!(
        malformed(r#"{ "name": "level", "type": 5 }"#),
        "rom.stats field type must be string"
    );
    let deeper = r#"{ "name": "outer", "type": "Struct", "fields": [{ "name": "inner", "type": "Struct" }] }"#;
    assert_eq!(
        refusal(deeper),
        "rom.outer.inner has the type Struct and needs a fields array"
    );
}

// A field name is a SmallString of at most 255 UTF-8 bytes, at every level. A longer name comes
// back as the crate's SmallString error.
#[test]
fn takes_a_field_name_of_255_bytes_and_refuses_a_longer_one_with_an_error() {
    let field = |name: &str| format!(r#"{{ "name": "{name}", "type": "String" }}"#);
    let longest = "k".repeat(255);
    let too_long = "k".repeat(256);
    let schemas = token_schemas_from_json(&schemas_json(&field(&longest))).unwrap();
    assert_eq!(schemas.rom.fields.last().unwrap().name.as_str(), longest);

    let nested = format!(
        r#"{{ "name": "stats", "type": "Struct", "fields": [{}] }}"#,
        field(&too_long)
    );
    for json in [schemas_json(&field(&too_long)), schemas_json(&nested)] {
        let err = token_schemas_from_json(&json).unwrap_err();
        assert!(
            matches!(&err, PhantasmaError::Serialization(message) if message == "SmallString exceeds 255 UTF-8 bytes"),
            "{err:?}"
        );
    }
    let err = build_token_schemas_from_fields(
        &[],
        &[],
        &[TokenSchemaField::new(too_long, VMType::String)],
    )
    .unwrap_err();
    assert!(
        matches!(&err, PhantasmaError::Serialization(message) if message == "SmallString exceeds 255 UTF-8 bytes"),
        "{err:?}"
    );
}
