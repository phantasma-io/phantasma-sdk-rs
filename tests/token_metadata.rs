//! Token metadata: the contract seal at creation and as an update, and the Token.UpdateTokenMetadata
//! builder.
//!
//! The seal is `_sealed_c` holding the Int64 1, and the chain refuses it with any other type or value.
//! An update removes a field with an empty Array_Dynamic, the only value the chain reads as a
//! removal. These tests hold the builders of the SDK to the same rules.

mod common;

use phantasma_sdk::{
    build_contract_seal_update, build_token_metadata, build_token_metadata_with_options,
    build_update_token_metadata_tx, build_update_token_metadata_tx_and_sign,
    build_update_token_metadata_tx_and_sign_hex, deserialize, plan_fees, Bytes32, FeePlanOptions,
    ModuleId, NativeFeeKind, PhantasmaError, PlanAndSignOptions, SignedTxMsg, TokenContractMethod,
    TokenMetadataOptions, TxLimits, TxMsg, TxMsgCall, TxPayload, TxType, UpdateTokenMetadataArgs,
    VMDynamicStruct, VMDynamicVariable, VMNamedDynamicVariable, VMType, VMValue,
};

use common::{address_of, keys, v2_config};

const OWNER: Bytes32 = Bytes32([3; 32]);

/// Returns the creation fields of the C# SDK's seal tests, so that the bytes below compare across
/// SDKs.
fn creation_fields() -> [(&'static str, &'static str); 4] {
    [
        ("name", "Sealed token"),
        ("icon", "data:image/png;base64,iVBORw0KGgo="),
        ("url", "https://example.invalid/sealed"),
        ("description", "A token without a contract"),
    ]
}

/// The fields of `creation_fields` as the C# and TypeScript SDKs write them. Each field is its
/// length-prefixed name, the String type 0x16 and the zero-terminated value. A struct is written
/// sorted by field name.
const CREATION_STRING_FIELDS_HEX: &str = concat!(
    "0B6465736372697074696F6E16",
    "4120746F6B656E20776974686F7574206120636F6E747261637400",
    "0469636F6E16",
    "646174613A696D6167652F706E673B6261736536342C6956424F5277304B47676F3D00",
    "046E616D6516",
    "5365616C656420746F6B656E00",
    "0375726C16",
    "68747470733A2F2F6578616D706C652E696E76616C69642F7365616C656400",
);

/// `_sealed_c` as the chain reads the seal: the length-prefixed name, the Int64 type 0x0C and the
/// value 1 in eight little-endian bytes.
const SEAL_FIELD_HEX: &str = "095F7365616C65645F630C0100000000000000";

/// Returns an empty Array_Dynamic, the only value the chain reads as the removal of a field.
fn removal() -> VMDynamicVariable {
    VMDynamicVariable::new(VMType::ArrayDynamic, VMValue::ArrayDynamic(Vec::new()))
}

/// Returns the message of a builder error. Any other error fails the test.
fn builder_message(err: PhantasmaError) -> String {
    match err {
        PhantasmaError::Builder(message) => message,
        other => panic!("expected a builder error, got {other:?}"),
    }
}

/// Returns the message with which the update builder refuses `fields` for token 42.
fn update_refusal(fields: Vec<VMNamedDynamicVariable>) -> String {
    builder_message(
        build_update_token_metadata_tx(
            42,
            VMDynamicStruct::new(fields),
            OWNER,
            TxLimits::default(),
        )
        .unwrap_err(),
    )
}

/// Returns the arguments the builder writes for an update of token 42 with `fields`.
fn update_args(fields: Vec<VMNamedDynamicVariable>) -> Vec<u8> {
    let msg = build_update_token_metadata_tx(
        42,
        VMDynamicStruct::new(fields),
        OWNER,
        TxLimits::default(),
    )
    .unwrap();
    call_of(&msg).args.clone()
}

/// Returns the call a message carries.
fn call_of(msg: &TxMsg) -> &TxMsgCall {
    let TxPayload::Call(call) = &msg.msg else {
        panic!("expected a call, got {:?}", msg.msg);
    };
    call
}

// The option adds `_sealed_c` as the Int64 1 to the string fields. The bytes are the TypeScript
// SDK's for the same input. The seal comes first because the struct is written sorted by name.
#[test]
fn seals_a_token_at_creation_with_the_int64_1_next_to_the_string_fields() {
    let bytes = build_token_metadata_with_options(
        &creation_fields(),
        TokenMetadataOptions {
            seal_contract: true,
        },
    )
    .unwrap();
    assert_eq!(
        hex::encode_upper(&bytes),
        format!("05000000{SEAL_FIELD_HEX}{CREATION_STRING_FIELDS_HEX}")
    );

    let metadata: VMDynamicStruct = deserialize(&bytes).unwrap();
    assert_eq!(
        metadata.get("_sealed_c"),
        Some(&VMDynamicVariable::int64(1))
    );
    assert_eq!(metadata.fields.len(), 5);
    assert_eq!(metadata.get("name").unwrap().vm_type, VMType::String);
}

// Without the option the metadata holds the string fields alone, and the default options write the
// same bytes as the builder without options.
#[test]
fn writes_no_seal_without_the_option() {
    let bytes = build_token_metadata(&creation_fields()).unwrap();
    assert_eq!(
        hex::encode_upper(&bytes),
        format!("04000000{CREATION_STRING_FIELDS_HEX}")
    );
    assert_eq!(
        build_token_metadata_with_options(&creation_fields(), TokenMetadataOptions::default())
            .unwrap(),
        bytes
    );
    let metadata: VMDynamicStruct = deserialize(&bytes).unwrap();
    assert!(metadata.get("_sealed_c").is_none());
}

// A string field cannot carry the Int64 the chain takes for the seal, so the key is refused with a
// message that names the option. The option does not make such a field acceptable.
#[test]
fn refuses_the_seal_passed_as_a_string_field() {
    let mut fields = creation_fields().to_vec();
    fields.push(("_sealed_c", "1"));
    for options in [
        TokenMetadataOptions::default(),
        TokenMetadataOptions {
            seal_contract: true,
        },
    ] {
        let err = build_token_metadata_with_options(&fields, options).unwrap_err();
        assert_eq!(
            builder_message(err),
            "pass TokenMetadataOptions { seal_contract: true } to seal a token against contracts: _sealed_c holds the Int64 1, which a string field cannot carry"
        );
    }
}

// The seal update holds the seal alone, so the chain keeps every other field of the token.
#[test]
fn builds_a_seal_update_that_holds_the_seal_alone() {
    let update = build_contract_seal_update();
    assert_eq!(update.fields.len(), 1);
    assert_eq!(update.fields[0].name.as_str(), "_sealed_c");
    assert_eq!(update.fields[0].value.vm_type, VMType::Int64);
    assert_eq!(update.fields[0].value.data, VMValue::Int(1));
}

// The chain reads a u64 token id, then a dynamic struct: a u32 field count, and per field a
// length-prefixed name, a type byte and the value. The arguments are the C# SDK's vector for the
// same update. The message carries the owner and the limits given.
#[test]
fn builds_an_update_call_of_the_token_id_then_the_update() {
    let limits = TxLimits {
        max_gas: 1,
        max_data: 2,
        expiry: 3,
    };
    let msg =
        build_update_token_metadata_tx(42, build_contract_seal_update(), OWNER, limits).unwrap();
    assert_eq!(msg.tx_type, TxType::Call);
    assert_eq!(msg.gas_from, OWNER);
    assert_eq!((msg.max_gas, msg.max_data, msg.expiry), (1, 2, 3));
    let call = call_of(&msg);
    assert_eq!(call.module_id, ModuleId::Token as u32);
    assert_eq!(
        call.method_id,
        TokenContractMethod::UpdateTokenMetadata as u32
    );
    assert_eq!(
        hex::encode_upper(&call.args),
        format!("2A0000000000000001000000{SEAL_FIELD_HEX}")
    );
}

// An ordinary key takes any value, and an update may remove it. Neither is a chain setting, so the
// builder passes both on as they are.
#[test]
fn takes_ordinary_fields_with_any_value_and_their_removal() {
    let fields = vec![
        VMNamedDynamicVariable::new(
            "description",
            VMDynamicVariable::string("A new description"),
        ),
        VMNamedDynamicVariable::new("website", removal()),
    ];
    let args: UpdateTokenMetadataArgs = deserialize(update_args(fields.clone())).unwrap();
    assert_eq!(args.token_id, 42);
    assert_eq!(args.metadata, VMDynamicStruct::new(fields));
}

// The chain assigns token ids from 1, so an update of token 0 is refused.
#[test]
fn refuses_token_id_0_which_the_chain_never_assigns() {
    let err =
        build_update_token_metadata_tx(0, build_contract_seal_update(), OWNER, TxLimits::default())
            .unwrap_err();
    assert_eq!(
        builder_message(err),
        "token_id must be between 1 and 2^64 - 1: the chain assigns no token id 0"
    );
}

// An update without fields would change nothing and still pay the fee.
#[test]
fn refuses_an_update_without_fields() {
    assert_eq!(
        update_refusal(Vec::new()),
        "a metadata update needs at least one field"
    );
}

// The seal is only ever the Int64 1. A string, another Int64, the Int256 1 and a removal are
// refused, because the chain refuses them and a seal cannot be removed. A struct can hold a name
// twice, so a wrong seal next to a right one is refused too.
#[test]
fn refuses_a_seal_of_any_other_type_or_value_than_the_int64_1() {
    for value in [
        VMDynamicVariable::string("1"),
        VMDynamicVariable::int64(2),
        VMDynamicVariable::int256(1),
        removal(),
    ] {
        assert_eq!(
            update_refusal(vec![VMNamedDynamicVariable::new("_sealed_c", value)]),
            "_sealed_c must be the Int64 1: build it with build_contract_seal_update"
        );
    }
    assert_eq!(
        update_refusal(vec![
            VMNamedDynamicVariable::new("_sealed_c", VMDynamicVariable::int64(1)),
            VMNamedDynamicVariable::new("_sealed_c", VMDynamicVariable::string("1")),
        ]),
        "_sealed_c must be the Int64 1: build it with build_contract_seal_update"
    );
}

// The planner has no formula for this call, so the signing variant plans it with the script budget,
// an upper bound. The SDK's own reader decodes the envelope back into the message that was planned,
// signed by the owner.
#[test]
fn signs_a_planned_update_into_an_envelope_the_sdk_reads_back() {
    let (owner_keys, _) = keys();
    let owner = address_of(&owner_keys);
    let config = v2_config();
    let signed = build_update_token_metadata_tx_and_sign(
        9,
        build_contract_seal_update(),
        &owner_keys,
        Some(&config),
        &PlanAndSignOptions::default(),
    )
    .unwrap();
    let decoded: SignedTxMsg = deserialize(&signed).unwrap();

    let same_expiry = TxLimits {
        expiry: decoded.msg.expiry,
        ..TxLimits::default()
    };
    let unsigned =
        build_update_token_metadata_tx(9, build_contract_seal_update(), owner, same_expiry)
            .unwrap();
    let plan = plan_fees(
        &unsigned,
        &config,
        &FeePlanOptions {
            witness_count: Some(1),
            ..FeePlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(plan.kinds, vec![NativeFeeKind::Script]);
    assert!(!plan.exact, "a budgeted call is planned as an upper bound");
    assert_eq!(decoded.msg, plan.apply(&unsigned));
    assert_eq!(signed.len() as u32, plan.envelope_bytes);
    assert_eq!(decoded.witnesses.len(), 1);
    assert_eq!(decoded.witnesses[0].address, owner);

    let as_hex = build_update_token_metadata_tx_and_sign_hex(
        9,
        build_contract_seal_update(),
        &owner_keys,
        Some(&config),
        &PlanAndSignOptions {
            limits: same_expiry,
            ..PlanAndSignOptions::default()
        },
    )
    .unwrap();
    assert_eq!(as_hex, hex::encode(&signed));
}

// An offer the caller fixed is signed as it stands and needs no gas config. The envelope is the
// TypeScript SDK's for the same key, update and limits.
#[test]
fn signs_an_update_whose_offer_the_caller_fixed_as_it_stands() {
    let (owner_keys, _) = keys();
    let signed = build_update_token_metadata_tx_and_sign_hex(
        9,
        build_contract_seal_update(),
        &owner_keys,
        None,
        &PlanAndSignOptions {
            limits: TxLimits {
                max_gas: 999_000_000,
                max_data: 7_000_000,
                expiry: 1_787_000_000_000,
            },
            ..PlanAndSignOptions::default()
        },
    )
    .unwrap();
    let owner_hex = "F94A8E45BDF1E37A8466B951849E92D1BAF870F49D1D04CD204D0BC9FE430896";
    let expected = [
        // Call, the expiry, the offer, the storage ceiling, the owner as the gas payer and an empty
        // payload.
        "00000E8011A0010000C0878B3B00000000C0CF6A0000000000",
        owner_hex,
        "00",
        // Module 1 (Token), method 23 (UpdateTokenMetadata), then the 31 argument bytes.
        "01000000170000001F000000",
        "090000000000000001000000",
        SEAL_FIELD_HEX,
        // One witness: the owner and the signature.
        "01000000",
        owner_hex,
        "29EC6B7778930B032BD8F46F44631CB2F4923B72742583FB8FFB3857553D3AEE",
        "CC1CBE2A99D5F0CA4B9C31AD3838B498F6E5F24A950C83677DF6D8F828E5CE0F",
    ]
    .concat();
    assert_eq!(signed.to_uppercase(), expected);
}
