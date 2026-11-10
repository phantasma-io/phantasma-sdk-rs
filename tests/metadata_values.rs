//! Values of NFT data and series metadata. The builders check each value against the type of its
//! field and convert it. An Int64 or Int256 value can be a string of decimal digits, and a Dynamic
//! or Array_Dynamic field takes typed values, `MetadataDynamicValue`. The SDK's own VM dynamic
//! variables are still written as they are. The byte vectors of converted values are the
//! TypeScript SDK's for the same input.

use num_bigint::BigInt;
use phantasma_sdk::{
    build_phantasma_nft_rom, build_update_token_metadata_tx, serialize, Bytes32, CarbonReader,
    CarbonWriter, MetadataDynamicValue, PhantasmaError, TxLimits, VMDynamicStruct,
    VMDynamicVariable, VMNamedDynamicVariable, VMNamedVariableSchema, VMStructSchema, VMType,
    VMValue,
};

/// Returns a schema of the given fields, with no flags.
fn schema(fields: &[(&str, VMType)]) -> VMStructSchema {
    VMStructSchema::new(
        fields
            .iter()
            .map(|(name, vm_type)| VMNamedVariableSchema::new(*name, *vm_type))
            .collect(),
    )
}

/// Returns the NFT data the SDK builds by `schema` for `metadata`, and that data read back by the
/// schema.
fn build_and_read(
    schema: &VMStructSchema,
    metadata: &[(&str, VMValue)],
) -> (String, VMDynamicStruct) {
    let bytes = build_phantasma_nft_rom(schema, metadata).unwrap();
    let read = VMDynamicStruct::read_with_schema(schema, &mut CarbonReader::new(&bytes)).unwrap();
    (hex::encode_upper(bytes), read)
}

/// Returns the message with which the builder refuses `value` for a field named "value".
fn refusal(vm_type: VMType, value: VMValue) -> String {
    let schema = schema(&[("value", vm_type)]);
    match build_phantasma_nft_rom(&schema, &[("value", value)]).unwrap_err() {
        PhantasmaError::Builder(message) => message,
        other => panic!("expected a builder error, got {other:?}"),
    }
}

/// Returns a typed value for a Dynamic field: `value` given as for a field of `vm_type`.
fn typed(vm_type: VMType, value: VMValue) -> VMValue {
    VMValue::MetadataDynamic(Box::new(MetadataDynamicValue::new(vm_type, value)))
}

fn text(value: &str) -> VMValue {
    VMValue::String(value.into())
}

/// Returns 2 to the power of `exponent`.
fn power_of_two(exponent: usize) -> BigInt {
    BigInt::from(1u8) << exponent
}

// A digit string takes the range of a number: the signed range, or the unsigned range, which is
// written as its two's complement. The items of an Array_Int64 or Array_Int256 field can be digit
// strings too.
#[test]
fn writes_digit_strings_for_int64_and_int256_at_the_range_edges() {
    let schema = schema(&[
        ("low", VMType::Int64),
        ("high", VMType::Int64),
        ("wideLow", VMType::Int256),
        ("wideHigh", VMType::Int256),
        ("ids", VMType::ArrayInt64),
        ("wideIds", VMType::ArrayInt256),
    ]);
    let (bytes, read) = build_and_read(
        &schema,
        &[
            ("low", text("-9223372036854775808")),
            ("high", text("18446744073709551615")),
            ("wideLow", text(&(-power_of_two(255)).to_string())),
            ("wideHigh", text(&(power_of_two(256) - 1u8).to_string())),
            (
                "ids",
                VMValue::ArrayString(vec![
                    "9223372036854775807".into(),
                    "-1".into(),
                    "9223372036854775808".into(),
                ]),
            ),
            (
                "wideIds",
                VMValue::ArrayString(vec!["1".into(), (power_of_two(255) - 1u8).to_string()]),
            ),
        ],
    );
    let expected = concat!(
        // low and high: eight little-endian bytes each.
        "0000000000000080",
        "FFFFFFFFFFFFFFFF",
        // wideLow: a header byte (0x80 for a negative number, plus the count of bytes, 32), then
        // the bytes.
        "A0",
        "0000000000000000000000000000000000000000000000000000000000000080",
        // wideHigh: the two's complement of the unsigned maximum is -1, a header without bytes.
        "80",
        // ids: the item count, then the items.
        "03000000",
        "FFFFFFFFFFFFFF7F",
        "FFFFFFFFFFFFFFFF",
        "0000000000000080",
        // wideIds: the item count, then 1 and the signed maximum.
        "02000000",
        "0101",
        "20",
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF7F",
    );
    assert_eq!(bytes, expected);
    assert_eq!(read.get("low").unwrap().data, VMValue::Int(i64::MIN));
    assert_eq!(read.get("high").unwrap().data, VMValue::Int(-1));
    assert_eq!(
        read.get("ids").unwrap().data,
        VMValue::ArrayInt64(vec![i64::MAX, -1, i64::MIN])
    );
}

// A digit string outside both ranges is refused with the message a number outside them gets.
#[test]
fn refuses_digit_strings_outside_the_ranges() {
    let int64_range = "between -9223372036854775808 and 9223372036854775807 or between 0 and 18446744073709551615";
    for digits in ["-9223372036854775809", "18446744073709551616"] {
        assert_eq!(
            refusal(VMType::Int64, text(digits)),
            format!("metadata field \"value\" must be {int64_range}")
        );
    }
    assert_eq!(
        refusal(
            VMType::ArrayInt64,
            VMValue::ArrayString(vec!["1".into(), "18446744073709551616".into()])
        ),
        format!("metadata field \"value[1]\" must be {int64_range}")
    );
    let int256_range = format!(
        "between {} and {} or between 0 and {}",
        -power_of_two(255),
        power_of_two(255) - 1u8,
        power_of_two(256) - 1u8
    );
    for number in [-power_of_two(255) - 1u8, power_of_two(256)] {
        assert_eq!(
            refusal(VMType::Int256, text(&number.to_string())),
            format!("metadata field \"value\" must be {int256_range}")
        );
    }
}

// Only the whole string counts: no space, no plus sign, no hex and no line break. Int8, Int16 and
// Int32 take no strings at all.
#[test]
fn refuses_a_string_that_is_not_only_decimal_digits() {
    for value in ["0x10", "12\n", " 12", "+12", "", "-", "1_000"] {
        assert_eq!(
            refusal(VMType::Int64, text(value)),
            "metadata field \"value\" must be an integer or a string of decimal digits",
            "{value:?}"
        );
    }
    assert_eq!(
        refusal(VMType::Int256, text("0x10")),
        "metadata field \"value\" must be an integer or a string of decimal digits"
    );
    assert_eq!(
        refusal(
            VMType::ArrayInt256,
            VMValue::ArrayString(vec!["1".into(), "x".into()])
        ),
        "metadata field \"value[1]\" must be an integer or a string of decimal digits"
    );
    assert_eq!(
        refusal(VMType::Int32, text("12")),
        "metadata field \"value\" must be an integer"
    );
}

// A Dynamic field is written as its value's type byte and then the value. An Array_Dynamic field is
// a count and then each item in the same way. A Dynamic field reads back as a Dynamic around the
// value, which keeps its own type, so the data read writes back to the same bytes. The bytes are
// also the C# SDK's layout for the same values.
#[test]
fn writes_typed_dynamic_values_with_their_types_and_reads_them_back_wrapped() {
    let schema = schema(&[
        ("extra", VMType::Dynamic),
        ("extras", VMType::ArrayDynamic),
        ("note", VMType::String),
    ]);
    let (bytes, read) = build_and_read(
        &schema,
        &[
            ("extra", typed(VMType::Int64, text("9007199254740993"))),
            (
                "extras",
                VMValue::ArrayMetadataDynamic(vec![
                    MetadataDynamicValue::new(VMType::String, text("a")),
                    MetadataDynamicValue::new(VMType::Int32, VMValue::Int(7)),
                ]),
            ),
            ("note", text("after the dynamic values")),
        ],
    );
    let expected = concat!(
        // extra: the type byte of Int64 (0x0C), then the value.
        "0C",
        "0100000000002000",
        // extras: the count, then a String (0x16) and an Int32 (0x0A) with their values.
        "02000000",
        "16",
        "6100",
        "0A",
        "07000000",
        // note
        "6166746572207468652064796E616D69632076616C75657300",
    );
    assert_eq!(bytes, expected);
    assert_eq!(
        read.get("extra").unwrap(),
        &VMDynamicVariable::new(
            VMType::Dynamic,
            VMValue::Dynamic(Box::new(VMDynamicVariable::int64(9_007_199_254_740_993)))
        )
    );
    assert_eq!(
        read.get("extras").unwrap().data,
        VMValue::ArrayDynamic(vec![
            VMDynamicVariable::string("a"),
            VMDynamicVariable::int32(7)
        ])
    );
    let mut writer = CarbonWriter::new();
    assert!(read.write_with_schema(&schema, &mut writer).unwrap());
    assert_eq!(hex::encode_upper(writer.into_bytes()), expected);
}

// A Dynamic value is written with its own type and without a schema, so it cannot be a struct. A
// typed value inside a typed value has no known use. Array has the code of Array_Dynamic.
#[test]
fn refuses_a_typed_value_of_a_struct_type_or_of_a_dynamic_type() {
    for (vm_type, name) in [
        (VMType::Dynamic, "Dynamic"),
        (VMType::ArrayDynamic, "Array_Dynamic"),
        (VMType::Array, "Array"),
        (VMType::Struct, "Struct"),
        (VMType::ArrayStruct, "Array_Struct"),
    ] {
        assert_eq!(
            refusal(VMType::Dynamic, typed(vm_type, VMValue::None)),
            format!("metadata field \"value\" cannot hold a Dynamic value of type {name}")
        );
    }
    assert_eq!(
        refusal(
            VMType::ArrayDynamic,
            VMValue::ArrayMetadataDynamic(vec![
                MetadataDynamicValue::new(VMType::Int32, VMValue::Int(1)),
                MetadataDynamicValue::new(VMType::Struct, VMValue::None),
            ])
        ),
        "metadata field \"value[1]\" cannot hold a Dynamic value of type Struct"
    );
}

// A typed value is checked as a field of its own type would be.
#[test]
fn checks_each_typed_value_against_its_own_type() {
    assert_eq!(
        refusal(VMType::Dynamic, typed(VMType::Bytes32, text("0011"))),
        "metadata field \"value\" must be exactly 32 bytes"
    );
    assert_eq!(
        refusal(
            VMType::ArrayDynamic,
            VMValue::ArrayMetadataDynamic(vec![MetadataDynamicValue::new(
                VMType::Bytes32,
                text("0011")
            )])
        ),
        "metadata field \"value[0]\" must be exactly 32 bytes"
    );
}

// A Dynamic field takes a typed value or the SDK's own VM dynamic variable, and nothing else.
#[test]
fn refuses_a_value_that_is_neither_form_for_a_dynamic_field() {
    assert_eq!(
        refusal(VMType::Dynamic, text("text")),
        "metadata field \"value\" is Dynamic and must be given as a MetadataDynamicValue"
    );
    assert_eq!(
        refusal(VMType::ArrayDynamic, VMValue::ArrayString(vec!["a".into()])),
        "metadata field \"value\" is Array_Dynamic and must be given as a list of MetadataDynamicValue"
    );
}

// The SDK's own VM dynamic variables are written as they are, also with a type that a typed value
// cannot have. The bytes are the ones the SDK wrote for these inputs before typed values existed.
// For the same values both forms give the same bytes.
#[test]
fn still_writes_the_sdk_vm_dynamic_variables_as_they_are() {
    let schema = schema(&[("extra", VMType::Dynamic), ("extras", VMType::ArrayDynamic)]);
    let (bytes, _) = build_and_read(
        &schema,
        &[
            (
                "extra",
                VMValue::Dynamic(Box::new(VMDynamicVariable::int64(5))),
            ),
            (
                "extras",
                VMValue::ArrayDynamic(vec![
                    VMDynamicVariable::string("a"),
                    VMDynamicVariable::int32(7),
                ]),
            ),
        ],
    );
    assert_eq!(
        bytes,
        concat!(
            "0C",
            "0500000000000000",
            "02000000",
            "16",
            "6100",
            "0A",
            "07000000",
        )
    );

    let inner = VMDynamicStruct::new(vec![VMNamedDynamicVariable::new(
        "a",
        VMDynamicVariable::int64(1),
    )]);
    let (bytes, _) = build_and_read(
        &schema,
        &[
            (
                "extra",
                VMValue::Dynamic(Box::new(VMDynamicVariable::new(
                    VMType::Struct,
                    VMValue::Struct(inner),
                ))),
            ),
            ("extras", VMValue::ArrayDynamic(Vec::new())),
        ],
    );
    // The type byte of Struct (0x04), then the struct with its field names and types, then an empty
    // Array_Dynamic.
    assert_eq!(
        bytes,
        concat!(
            "04",
            "01000000",
            "0161",
            "0C",
            "0100000000000000",
            "00000000",
        )
    );
}

// Inside a struct field, a digit string and a typed value are converted as in a field of their
// own: the count from its digits, and the bytes from the hex string, written after the type byte
// of Bytes (0x02) and their count.
#[test]
fn writes_a_digit_string_and_a_typed_value_inside_a_struct_child_as_converted_values() {
    let schema = VMStructSchema::new(vec![VMNamedVariableSchema::with_struct(
        "stats",
        VMType::Struct,
        schema(&[("count", VMType::Int64), ("extra", VMType::Dynamic)]),
    )]);
    let stats = VMDynamicStruct::new(vec![
        VMNamedDynamicVariable::make("count", VMType::Int64, text("9007199254740993")),
        VMNamedDynamicVariable::make(
            "extra",
            VMType::Dynamic,
            typed(VMType::Bytes, text("0xdead")),
        ),
    ]);
    let (bytes, read) = build_and_read(&schema, &[("stats", VMValue::Struct(stats))]);
    assert_eq!(bytes, concat!("0100000000002000", "02", "02000000", "DEAD"));
    let VMValue::Struct(stats) = &read.get("stats").unwrap().data else {
        panic!("expected a struct");
    };
    assert_eq!(
        stats.get("count").unwrap().data,
        VMValue::Int(9_007_199_254_740_993)
    );
    assert_eq!(
        stats.get("extra").unwrap().data,
        VMValue::Dynamic(Box::new(VMDynamicVariable::bytes(vec![0xde, 0xad])))
    );
}

// A typed value is an input of the metadata builders. Written as it is, it would lose its value: a
// Dynamic field would hold an empty Array_Dynamic, and an Array_Dynamic field would be an empty
// Array_Dynamic, which a token metadata update reads as the removal of the field. So the writer
// refuses both, also in an update.
#[test]
fn refuses_to_write_a_typed_value_outside_the_metadata_builders() {
    let message = "a MetadataDynamicValue is converted by the metadata builders and cannot be written as it is";
    let fields = [
        VMNamedDynamicVariable::new(
            "level",
            VMDynamicVariable::new(VMType::Dynamic, typed(VMType::Int64, VMValue::Int(1))),
        ),
        VMNamedDynamicVariable::new(
            "tags",
            VMDynamicVariable::new(
                VMType::ArrayDynamic,
                VMValue::ArrayMetadataDynamic(vec![MetadataDynamicValue::new(
                    VMType::String,
                    text("rare"),
                )]),
            ),
        ),
    ];
    for field in fields {
        let err = serialize(&VMDynamicStruct::new(vec![field.clone()])).unwrap_err();
        assert!(
            matches!(&err, PhantasmaError::Serialization(got) if got == message),
            "{err:?}"
        );
        let err = build_update_token_metadata_tx(
            42,
            VMDynamicStruct::new(vec![field]),
            Bytes32([3; 32]),
            TxLimits::default(),
        )
        .unwrap_err();
        assert!(
            matches!(&err, PhantasmaError::Serialization(got) if got == message),
            "{err:?}"
        );
    }
}
