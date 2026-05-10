use std::fs;

use num_bigint::BigInt;
use phantasma_sdk::{
    build_and_serialize_token_schemas, build_create_token_series_tx, build_create_token_tx,
    build_mint_non_fungible_tx, build_mint_non_fungible_tx_and_sign,
    build_mint_non_fungible_tx_and_sign_hex, build_mint_phantasma_non_fungible_single_tx,
    build_nft_rom, build_phantasma_nft_rom, build_series_info, build_token_info,
    build_token_metadata, build_token_series_metadata, bytes32_from_phantasma_address,
    bytes32_from_public_key, check_token_symbol, deserialize, get_nft_address,
    parse_create_token_result, parse_create_token_series_result, parse_mint_non_fungible_result,
    parse_mint_phantasma_non_fungible_result, prepare_standard_token_schemas, serialize,
    serialize_token_schemas_hex, sign_and_serialize_tx_msg_hex, unpack_nft_instance_id, Bytes32,
    CallArgSection, CarbonReader, CarbonWriter, CreateSeriesFeeOptions, CreateTokenFeeOptions,
    FeeOptions, IntX, MintPhantasmaNonFungibleArgs, MsgCallArgSections, PhantasmaKeys, SmallString,
    TokenFlags, TxMsg, TxMsgBurnFungibleGasPayer, TxMsgCall, TxMsgMintFungible,
    TxMsgTransferFungible, TxMsgTransferFungibleGasPayer, TxPayload, TxType, VMDynamicStruct,
    VMNamedDynamicVariable, VMNamedVariableSchema, VMStructArray, VMStructSchema, VMType, VMValue,
};
use sha2::{Digest, Sha256};

const SAMPLE_PNG_ICON_DATA_URI: &str =
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==";
const SAMPLE_JPEG_ICON_DATA_URI: &str = "data:image/jpeg;base64,/9j/";
const SAMPLE_WEBP_ICON_DATA_URI: &str = "data:image/webp;base64,UklGRg==";
const CARBON_TX_BUILDER_FIXTURE_SHA256: &str =
    "efcb2d237ffd2ca3178b8c3b3106c7d035bc0f5e05959abb135163d637c3b11d";

fn repeated_bytes32(value: u8) -> Bytes32 {
    Bytes32([value; 32])
}

fn metadata_fields(icon: &str) -> [(&str, &str); 4] {
    [
        ("name", "My token"),
        ("icon", icon),
        ("url", "https://example.com"),
        ("description", "Demo"),
    ]
}

fn carbon_builder_rows() -> Vec<(String, String, String, String)> {
    fs::read_to_string("tests/fixtures/carbon_tx_builder_vectors.tsv")
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with("case_id\t"))
        .map(|line| {
            let parts = line.split('\t').collect::<Vec<_>>();
            assert_eq!(parts.len(), 4, "bad Carbon builder vector row: {line}");
            (
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
                parts[3].to_string(),
            )
        })
        .collect()
}

fn sample_token_metadata() -> [(&'static str, &'static str); 4] {
    [
        ("name", "My test token!"),
        (
            "icon",
            "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==",
        ),
        ("url", "http://example.com"),
        ("description", "My test token description"),
    ]
}

fn sample_nft_metadata(include_nested_rom: bool) -> Vec<(&'static str, VMValue)> {
    let mut fields = vec![
        ("name", VMValue::String("My NFT #1".into())),
        (
            "description",
            VMValue::String("This is my first NFT!".into()),
        ),
        (
            "imageURL",
            VMValue::String("images-assets.nasa.gov/image/PIA13227/PIA13227~orig.jpg".into()),
        ),
        (
            "infoURL",
            VMValue::String("https://images.nasa.gov/details/PIA13227".into()),
        ),
        ("royalties", VMValue::Int(10_000_000)),
    ];
    if include_nested_rom {
        fields.push(("rom", VMValue::Bytes(vec![0x01, 0x42])));
    }
    fields
}

#[test]
fn carbon_tx_builder_fixture_hash_is_locked() {
    let data = fs::read("tests/fixtures/carbon_tx_builder_vectors.tsv").unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(data)),
        CARBON_TX_BUILDER_FIXTURE_SHA256
    );
}

#[test]
fn token_result_parsers_match_reference_behaviour() {
    // Carbon result helpers decode RPC blobs and reject truncated payloads.
    assert_eq!(parse_create_token_result("0900000000000000").unwrap(), 9);
    assert_eq!(parse_create_token_series_result("07000000").unwrap(), 7);
    assert_eq!(
        parse_mint_non_fungible_result(9, "0200000007000000000000000800000000000000").unwrap(),
        vec![get_nft_address(9, 7), get_nft_address(9, 8)]
    );
    let payload = format!(
        "02000000{}0700000000000000{}0800000000000000",
        "55".repeat(32),
        "AA".repeat(32)
    );
    let parsed = parse_mint_phantasma_non_fungible_result(&payload).unwrap();
    assert_eq!(parsed[0].carbon_instance_id, 7);
    assert_eq!(parsed[1].phantasma_nft_id, repeated_bytes32(0xAA));
    assert!(parse_create_token_result("01020304").is_err());
    assert!(parse_mint_non_fungible_result(9, "01000000").is_err());
}

#[test]
fn token_metadata_and_symbol_validation_match_reference_sdks() {
    // Token builders reject malformed public inputs before invalid Carbon payloads are produced.
    check_token_symbol("SOUL").unwrap();
    for symbol in ["", "A1", "AbC"] {
        assert!(check_token_symbol(symbol).is_err());
    }
    assert!(build_token_metadata(&[
        ("name", "My token"),
        ("icon", SAMPLE_PNG_ICON_DATA_URI),
        ("url", "https://example.com"),
        ("description", "Demo"),
    ])
    .is_ok());
    assert!(build_token_metadata(&[
        ("name", "My token"),
        ("icon", "data:image/svg+xml;base64,PHN2Zy8+"),
        ("url", "https://example.com"),
        ("description", "Demo"),
    ])
    .is_err());
}

#[test]
fn token_metadata_icon_validation_matches_reference_sdk_matrix() {
    for icon in [
        SAMPLE_PNG_ICON_DATA_URI,
        SAMPLE_JPEG_ICON_DATA_URI,
        SAMPLE_WEBP_ICON_DATA_URI,
    ] {
        build_token_metadata(&metadata_fields(icon)).unwrap();
    }

    for icon in [
        "data:image/svg+xml;base64,PHN2Zy8+",
        "data:image/svg+xml,%3Csvg%2F%3E",
        "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAAAAACH5BAAAAAAALAAAAAABAAEAAAICRAEAOw==",
        "data:image/png;base64,",
        "data:image/jpeg;base64,@@@",
    ] {
        assert!(build_token_metadata(&metadata_fields(icon)).is_err());
    }
}

#[test]
fn token_info_flags_and_schema_serialization_are_stable() {
    // Fungible/NFT token info chooses the Carbon token flag set from supply and schema rules.
    let schemas = prepare_standard_token_schemas(false);
    let metadata = build_token_metadata(&[
        ("name", "My token"),
        ("icon", SAMPLE_PNG_ICON_DATA_URI),
        ("url", "https://example.com"),
        ("description", "Demo"),
    ])
    .unwrap();
    let fungible = build_token_info(
        "FUNGIBLE",
        IntX::from(0i64),
        false,
        8,
        Bytes32::default(),
        metadata.clone(),
        vec![],
    )
    .unwrap();
    assert_eq!(fungible.flags, TokenFlags::NONE);
    let nft = build_token_info(
        "NFT",
        IntX::from(100i64),
        true,
        0,
        Bytes32::default(),
        metadata,
        phantasma_sdk::serialize_token_schemas(&schemas).unwrap(),
    )
    .unwrap();
    assert!(nft.flags.contains(TokenFlags::NON_FUNGIBLE));
    assert_eq!(
        serialize_token_schemas_hex(&schemas).unwrap(),
        hex::encode_upper(phantasma_sdk::serialize_token_schemas(&schemas).unwrap())
    );
}

#[test]
fn phantasma_nft_public_rom_and_tx_helpers_work() {
    // Deterministic Phantasma NFT minting omits chain-owned `_i` and nested `rom` caller fields.
    let schemas = prepare_standard_token_schemas(false);
    let rom = build_phantasma_nft_rom(
        &schemas.rom,
        &[
            ("name", VMValue::String("My NFT #1".into())),
            ("description", VMValue::String("Demo".into())),
            (
                "imageURL",
                VMValue::String("https://example.com/i.png".into()),
            ),
            ("infoURL", VMValue::String("https://example.com".into())),
            ("royalties", VMValue::Int(10_000_000)),
        ],
    )
    .unwrap();
    let public_schema = phantasma_sdk::build_phantasma_nft_public_mint_schema(&schemas.rom);
    let decoded =
        VMDynamicStruct::read_with_schema(&public_schema, &mut CarbonReader::new(&rom)).unwrap();
    assert!(decoded.get("_i").is_none());
    assert!(decoded.get("rom").is_none());

    let sender = repeated_bytes32(0x11);
    let receiver = repeated_bytes32(0x22);
    let tx = build_mint_phantasma_non_fungible_single_tx(
        42,
        BigInt::from(777u32),
        sender,
        receiver,
        rom,
        vec![],
        Some(FeeOptions::default()),
        123,
        999,
    )
    .unwrap();
    assert_eq!(tx.tx_type, TxType::Call);
    assert_eq!(tx.expiry, 999);
    assert_eq!(tx.max_data, 123);
    assert_eq!(tx.gas_from, sender);
    let TxPayload::Call(call) = tx.msg else {
        panic!("expected call payload");
    };
    let args: MintPhantasmaNonFungibleArgs = deserialize(call.args).unwrap();
    assert_eq!(args.token_id, 42);
    assert_eq!(args.address, receiver);
    assert_eq!(
        args.tokens[0].phantasma_series_id,
        IntX(BigInt::from(777u32))
    );
}

#[test]
fn generic_carbon_integer_array_helpers_match_python_vectors() {
    let mut writer = CarbonWriter::new();
    writer.write_int_array(&[-1, 0, 127], 1, true).unwrap();
    assert_eq!(hex::encode(writer.bytes()), "03000000ff007f");
    let mut reader = CarbonReader::new(writer.bytes());
    assert_eq!(reader.read_int_array(1, true).unwrap(), vec![-1, 0, 127]);

    let mut writer = CarbonWriter::new();
    writer
        .write_int_array(&[0, i128::from(u64::MAX)], 8, false)
        .unwrap();
    assert_eq!(
        hex::encode(writer.bytes()),
        "020000000000000000000000ffffffffffffffff"
    );
    let mut reader = CarbonReader::new(writer.bytes());
    assert_eq!(
        reader.read_int_array(8, false).unwrap(),
        vec![0, i128::from(u64::MAX)]
    );
}

#[test]
fn intx_eight_byte_safety_boundaries_match_reference_sdks() {
    let min = BigInt::from(i64::MIN);
    let max = BigInt::from(i64::MAX);

    assert!(IntX(min.clone()).is_8_byte_safe());
    assert!(IntX(max.clone()).is_8_byte_safe());
    assert!(IntX::from(0i64).is_8_byte_safe());
    assert!(!IntX(max + 1).is_8_byte_safe());
    assert!(!IntX(min - 1).is_8_byte_safe());
}

#[test]
fn tx_msg_call_arg_sections_match_reference_sdks() {
    let call = TxMsgCall {
        module_id: 1,
        method_id: 2,
        args: Vec::new(),
        sections: Some(MsgCallArgSections {
            sections: vec![
                CallArgSection {
                    register_offset: -1,
                    args: Vec::new(),
                },
                CallArgSection {
                    register_offset: 0,
                    args: vec![0x0A, 0x0B],
                },
            ],
        }),
    };
    let expected_hex = "0100000002000000FEFFFFFFFFFFFFFF020000000A0B";

    assert_eq!(hex::encode_upper(serialize(&call).unwrap()), expected_hex);

    let decoded: TxMsgCall = deserialize(hex::decode(expected_hex).unwrap()).unwrap();
    assert_eq!(decoded.module_id, 1);
    assert_eq!(decoded.method_id, 2);
    assert!(decoded.args.is_empty());
    let sections = decoded.sections.unwrap().sections;
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].register_offset, -1);
    assert!(sections[0].args.is_empty());
    assert_eq!(sections[1].register_offset, 0);
    assert_eq!(sections[1].args, vec![0x0A, 0x0B]);
}

#[test]
fn metadata_builders_validate_schema_inputs_like_python_sdk() {
    let schemas = prepare_standard_token_schemas(false);
    assert!(build_nft_rom(&schemas.rom, 1, &[]).is_err());
    assert!(build_nft_rom(
        &schemas.rom,
        1,
        &[("Name", VMValue::String("wrong case".into()))],
    )
    .is_err());
    assert!(
        build_token_series_metadata(&schemas.series_metadata, 1, &[("rom", VMValue::Int(7))])
            .is_err()
    );

    let nested_schema = VMStructSchema::new(vec![VMNamedVariableSchema::make(
        "innerName",
        VMType::String,
    )]);
    let custom_schema = VMStructSchema::new(vec![
        VMNamedVariableSchema::make("_i", VMType::Int256),
        VMNamedVariableSchema::make("rom", VMType::Bytes),
        VMNamedVariableSchema::make_with_struct("details", VMType::Struct, nested_schema.clone()),
        VMNamedVariableSchema::make("roots", VMType::ArrayBytes32),
    ]);
    let nested_with_extra = VMDynamicStruct::new(vec![
        VMNamedDynamicVariable::make("innerName", VMType::String, VMValue::String("demo".into())),
        VMNamedDynamicVariable::make("extra", VMType::String, VMValue::String("oops".into())),
    ]);
    assert!(build_nft_rom(
        &custom_schema,
        1,
        &[
            ("details", VMValue::Struct(nested_with_extra)),
            ("roots", VMValue::ArrayBytes32(vec![])),
        ],
    )
    .is_err());

    let scalar_schema = VMStructSchema::new(vec![
        VMNamedVariableSchema::make("_i", VMType::Int256),
        VMNamedVariableSchema::make("rom", VMType::Bytes),
        VMNamedVariableSchema::make("payload", VMType::Bytes),
        VMNamedVariableSchema::make("royalties", VMType::Int32),
        VMNamedVariableSchema::make("roots", VMType::ArrayBytes32),
    ]);
    let rom = build_nft_rom(
        &scalar_schema,
        1,
        &[
            ("payload", VMValue::String("0x0a0b".into())),
            ("royalties", VMValue::Int(0xFFFF_FFFF)),
            ("roots", VMValue::ArrayBytes32(vec![Bytes32([0x11; 32])])),
        ],
    )
    .unwrap();
    let decoded =
        VMDynamicStruct::read_with_schema(&scalar_schema, &mut CarbonReader::new(&rom)).unwrap();
    assert_eq!(
        decoded.get("payload").unwrap().data,
        VMValue::Bytes(vec![0x0a, 0x0b])
    );
    assert_eq!(decoded.get("royalties").unwrap().data, VMValue::Int(-1));
    assert_eq!(
        decoded.get("roots").unwrap().data,
        VMValue::ArrayBytes32(vec![Bytes32([0x11; 32])])
    );
    assert!(build_nft_rom(
        &scalar_schema,
        1,
        &[
            ("payload", VMValue::String("xyz".into())),
            ("royalties", VMValue::Int(0)),
            ("roots", VMValue::ArrayBytes32(vec![])),
        ],
    )
    .is_err());
    assert!(build_nft_rom(
        &scalar_schema,
        1,
        &[
            ("payload", VMValue::Bytes(vec![])),
            ("royalties", VMValue::Int(1i64 << 32)),
            ("roots", VMValue::ArrayBytes32(vec![])),
        ],
    )
    .is_err());
}

#[test]
fn array_struct_metadata_round_trips_with_schema() {
    let item_schema =
        VMStructSchema::new(vec![VMNamedVariableSchema::make("name", VMType::String)]);
    let schema = VMStructSchema::new(vec![
        VMNamedVariableSchema::make("_i", VMType::Int256),
        VMNamedVariableSchema::make("rom", VMType::Bytes),
        VMNamedVariableSchema::make_with_struct("items", VMType::ArrayStruct, item_schema.clone()),
    ]);
    let items = VMStructArray {
        schema: item_schema,
        structs: vec![
            VMDynamicStruct::new(vec![VMNamedDynamicVariable::make(
                "name",
                VMType::String,
                VMValue::String("one".into()),
            )]),
            VMDynamicStruct::new(vec![VMNamedDynamicVariable::make(
                "name",
                VMType::String,
                VMValue::String("two".into()),
            )]),
        ],
    };
    let rom = build_nft_rom(&schema, 1, &[("items", VMValue::ArrayStruct(items))]).unwrap();
    let decoded = VMDynamicStruct::read_with_schema(&schema, &mut CarbonReader::new(&rom)).unwrap();
    let VMValue::ArrayStruct(value) = &decoded.get("items").unwrap().data else {
        panic!("expected array struct");
    };
    let names: Vec<_> = value
        .structs
        .iter()
        .map(|item| match &item.get("name").unwrap().data {
            VMValue::String(value) => value.as_str(),
            _ => panic!("expected string"),
        })
        .collect();
    assert_eq!(names, vec!["one", "two"]);
}

#[test]
fn carbon_address_and_signing_helpers_match_shared_vector() {
    // Carbon signing helper uses the same public-key Bytes32 address as the other SDKs.
    let keys =
        PhantasmaKeys::from_wif("KwPpBSByydVKqStGHAnZzQofCqhDmD2bfRgc9BmZqM3ZmsdWJw4d").unwrap();
    let receiver =
        PhantasmaKeys::from_wif("KwVG94yjfVg1YKFyRxAGtug93wdRbmLnqqrFV6Yd2CiA9KZDAp4H").unwrap();
    let sender_bytes = bytes32_from_public_key(&keys.public_key()).unwrap();
    assert_eq!(
        bytes32_from_phantasma_address(&keys.address()).unwrap(),
        sender_bytes
    );
    let msg = TxMsg {
        tx_type: TxType::TransferFungible,
        expiry: 1_759_711_416_000,
        max_gas: 10_000_000,
        max_data: 1_000,
        gas_from: sender_bytes,
        payload: SmallString::new("test-payload").unwrap(),
        msg: TxPayload::TransferFungible(TxMsgTransferFungible {
            to: bytes32_from_public_key(&receiver.public_key()).unwrap(),
            token_id: 1,
            amount: 100_000_000,
        }),
    };
    let vector = std::fs::read_to_string("tests/fixtures/carbon_vectors.tsv")
        .unwrap()
        .lines()
        .find(|line| line.starts_with("TX2\t"))
        .unwrap()
        .split('\t')
        .nth(2)
        .unwrap()
        .to_string();
    assert_eq!(
        sign_and_serialize_tx_msg_hex(&msg, &keys)
            .unwrap()
            .to_uppercase(),
        vector
    );
}

#[test]
fn carbon_tx_builders_match_golden_vectors() {
    // Golden vectors exercise public transaction builders, not only raw Carbon serde.
    for (case_id, source, expected_hex, notes) in carbon_builder_rows() {
        assert!(
            matches!(source.as_str(), "csharp_sdk" | "go_sdk"),
            "{notes}"
        );
        assert_eq!(
            carbon_tx_builder_vector(&case_id),
            expected_hex,
            "{case_id}"
        );
        if !case_id.starts_with("signed_") {
            let decoded: TxMsg = deserialize(hex::decode(&expected_hex).unwrap()).unwrap();
            assert_eq!(
                hex::encode_upper(serialize(&decoded).unwrap()),
                expected_hex,
                "{case_id} round-trip"
            );
        }
    }
}

fn carbon_tx_builder_vector(case_id: &str) -> String {
    let keys =
        PhantasmaKeys::from_wif("KwPpBSByydVKqStGHAnZzQofCqhDmD2bfRgc9BmZqM3ZmsdWJw4d").unwrap();
    let receiver =
        PhantasmaKeys::from_wif("KwVG94yjfVg1YKFyRxAGtug93wdRbmLnqqrFV6Yd2CiA9KZDAp4H").unwrap();
    let sender_bytes = bytes32_from_public_key(&keys.public_key()).unwrap();
    let receiver_bytes = bytes32_from_public_key(&receiver.public_key()).unwrap();

    match case_id {
        "signed_transfer_fungible" => {
            let msg = TxMsg {
                tx_type: TxType::TransferFungible,
                expiry: 1_759_711_416_000,
                max_gas: 10_000_000,
                max_data: 1_000,
                gas_from: sender_bytes,
                payload: SmallString::new("test-payload").unwrap(),
                msg: TxPayload::TransferFungible(TxMsgTransferFungible {
                    to: receiver_bytes,
                    token_id: 1,
                    amount: 100_000_000,
                }),
            };
            sign_and_serialize_tx_msg_hex(&msg, &keys)
                .unwrap()
                .to_uppercase()
        }
        "transfer_fungible_gas_payer" => hex::encode_upper(
            serialize(&TxMsg {
                tx_type: TxType::TransferFungibleGasPayer,
                expiry: 1_759_711_416_000,
                max_gas: 10_000_000,
                max_data: 1_000,
                gas_from: sender_bytes,
                payload: SmallString::new("test-payload").unwrap(),
                msg: TxPayload::TransferFungibleGasPayer(TxMsgTransferFungibleGasPayer {
                    to: receiver_bytes,
                    from_address: sender_bytes,
                    token_id: 1,
                    amount: 100_000_000,
                }),
            })
            .unwrap(),
        ),
        "burn_fungible_gas_payer" => hex::encode_upper(
            serialize(&TxMsg {
                tx_type: TxType::BurnFungibleGasPayer,
                expiry: 1_759_711_416_000,
                max_gas: 10_000_000,
                max_data: 1_000,
                gas_from: sender_bytes,
                payload: SmallString::new("test-payload").unwrap(),
                msg: TxPayload::BurnFungibleGasPayer(TxMsgBurnFungibleGasPayer {
                    token_id: 1,
                    from_address: sender_bytes,
                    amount: IntX::from(100_000_000i64),
                }),
            })
            .unwrap(),
        ),
        "mint_fungible" => hex::encode_upper(
            serialize(&TxMsg {
                tx_type: TxType::MintFungible,
                expiry: 1_759_711_416_000,
                max_gas: 10_000_000,
                max_data: 1_000,
                gas_from: sender_bytes,
                payload: SmallString::new("test-payload").unwrap(),
                msg: TxPayload::MintFungible(TxMsgMintFungible {
                    token_id: 1,
                    to: receiver_bytes,
                    amount: IntX::from(100_000_000i64),
                }),
            })
            .unwrap(),
        ),
        "create_token_nft" => {
            let token_info = build_token_info(
                "MYNFT",
                IntX::from(0i64),
                true,
                0,
                sender_bytes,
                build_token_metadata(&sample_token_metadata()).unwrap(),
                build_and_serialize_token_schemas(None).unwrap(),
            )
            .unwrap();
            hex::encode_upper(
                serialize(
                    &build_create_token_tx(
                        token_info,
                        sender_bytes,
                        Some(CreateTokenFeeOptions::default()),
                        100_000_000,
                        1_759_711_416_000,
                    )
                    .unwrap(),
                )
                .unwrap(),
            )
        }
        "create_token_series_u256_id" => {
            let series_id = (BigInt::from(1u8) << 256) - BigInt::from(1u8);
            let series_info = build_series_info(series_id, 0, 0, sender_bytes).unwrap();
            hex::encode_upper(
                serialize(
                    &build_create_token_series_tx(
                        u64::MAX,
                        series_info,
                        sender_bytes,
                        Some(CreateSeriesFeeOptions::default()),
                        100_000_000,
                        1_759_711_416_000,
                    )
                    .unwrap(),
                )
                .unwrap(),
            )
        }
        "mint_non_fungible_u256_nft_id" => {
            let schemas = prepare_standard_token_schemas(false);
            let nft_id = (BigInt::from(1u8) << 256) - BigInt::from(1u8);
            let rom = build_nft_rom(&schemas.rom, nft_id, &sample_nft_metadata(true)).unwrap();
            hex::encode_upper(
                serialize(&build_mint_non_fungible_tx(
                    u64::MAX,
                    u32::MAX,
                    sender_bytes,
                    sender_bytes,
                    rom,
                    vec![],
                    Some(FeeOptions::default()),
                    100_000_000,
                    1_759_711_416_000,
                ))
                .unwrap(),
            )
        }
        "mint_phantasma_nft_single_u255_series" => {
            let schemas = prepare_standard_token_schemas(false);
            let series_id = (BigInt::from(1u8) << 255) - BigInt::from(1u8);
            let public_rom =
                build_phantasma_nft_rom(&schemas.rom, &sample_nft_metadata(false)).unwrap();
            hex::encode_upper(
                serialize(
                    &build_mint_phantasma_non_fungible_single_tx(
                        42,
                        series_id,
                        sender_bytes,
                        receiver_bytes,
                        public_rom,
                        vec![],
                        Some(FeeOptions::default()),
                        123,
                        1_759_711_416_000,
                    )
                    .unwrap(),
                )
                .unwrap(),
            )
        }
        _ => panic!("unhandled Carbon builder vector: {case_id}"),
    }
}

#[test]
fn mint_nft_signing_hex_helper_matches_raw_helper() {
    // Hex convenience wrappers are thin views over the same signed Carbon bytes.
    let keys =
        PhantasmaKeys::from_wif("KwPpBSByydVKqStGHAnZzQofCqhDmD2bfRgc9BmZqM3ZmsdWJw4d").unwrap();
    let receiver = bytes32_from_public_key(
        &PhantasmaKeys::from_wif("KwVG94yjfVg1YKFyRxAGtug93wdRbmLnqqrFV6Yd2CiA9KZDAp4H")
            .unwrap()
            .public_key(),
    )
    .unwrap();
    let raw = build_mint_non_fungible_tx_and_sign(
        9,
        7,
        &keys,
        receiver,
        vec![0xAA],
        vec![],
        Some(FeeOptions::default()),
        0,
        1_759_711_416_000,
    )
    .unwrap();
    let encoded = build_mint_non_fungible_tx_and_sign_hex(
        9,
        7,
        &keys,
        receiver,
        vec![0xAA],
        vec![],
        Some(FeeOptions::default()),
        0,
        1_759_711_416_000,
    )
    .unwrap();
    assert_eq!(encoded, hex::encode(raw));
}

#[test]
fn unpack_nft_instance_id_matches_reference_helper() {
    // Carbon instance ids pack mint and series ordinals into one u64.
    assert_eq!(unpack_nft_instance_id(0x0000000800000007), (7, 8));
}
