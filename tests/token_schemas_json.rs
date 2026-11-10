//! Token schemas JSON. A field is `{ "name", "type" }`, and the builders turn each field into its
//! schema.

use phantasma_sdk::{
    build_token_schemas_from_fields, token_schemas_from_json, PhantasmaError, TokenSchemaField,
    VMType,
};

/// The fields every NFT token schema needs, so that `token_schemas_from_json` accepts the schemas.
const STANDARD_ROM_FIELDS: &str = r#"
    { "name": "name", "type": "String" },
    { "name": "description", "type": "String" },
    { "name": "imageURL", "type": "String" },
    { "name": "infoURL", "type": "String" },
    { "name": "royalties", "type": "Int32" }"#;

/// Returns the token schemas JSON with the standard ROM fields and `extra_fields` in `rom`.
fn schemas_json(extra_fields: &str) -> String {
    format!(
        r#"{{ "seriesMetadata": [], "rom": [{STANDARD_ROM_FIELDS}, {extra_fields}], "ram": [] }}"#
    )
}

// A field name is a SmallString of at most 255 UTF-8 bytes. A longer name comes back as the crate's
// SmallString error.
#[test]
fn takes_a_field_name_of_255_bytes_and_refuses_a_longer_one_with_an_error() {
    let field = |name: &str| format!(r#"{{ "name": "{name}", "type": "String" }}"#);
    let longest = "k".repeat(255);
    let too_long = "k".repeat(256);
    let schemas = token_schemas_from_json(&schemas_json(&field(&longest))).unwrap();
    assert_eq!(schemas.rom.fields.last().unwrap().name.as_str(), longest);

    let err = token_schemas_from_json(&schemas_json(&field(&too_long))).unwrap_err();
    assert!(
        matches!(&err, PhantasmaError::Serialization(message) if message == "SmallString exceeds 255 UTF-8 bytes"),
        "{err:?}"
    );
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
