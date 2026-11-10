//! Values of NFT data and series metadata. The builders check each value against the type of its
//! field and convert it. An Int64 or Int256 value can be a string of decimal digits. The byte vectors
//! of converted values are the TypeScript SDK's for the same input.

use num_bigint::BigInt;
use phantasma_sdk::{
    build_phantasma_nft_rom, CarbonReader, PhantasmaError, VMDynamicStruct, VMNamedVariableSchema,
    VMStructSchema, VMType, VMValue,
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
