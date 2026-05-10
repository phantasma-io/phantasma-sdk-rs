use num_bigint::BigInt;
use phantasma_sdk::vm::VMType as ClassicVMType;
use phantasma_sdk::{
    big_int_to_vm_bytes, vm_bytes_to_big_int, Address, BinaryReader, BinaryWriter, Opcode,
    PhantasmaKeys, ScriptArg, ScriptBuilder, Transaction, VMObject, SDK_PAYLOAD, SDK_VERSION,
};
use sha2::{Digest, Sha256};

const EXPECTED_CONSENSUS_SINGLE_VOTE: &str = concat!(
    "0D00030350340303000D000302102703000D000223220000000000000000000000000000000000000000000000000000",
    "000000000000000003000D000223220100AA53BE71FC41BC0889B694F4D6D03F7906A3D9A21705943CAF9632EEAFBB",
    "489503000D000408416C6C6F7747617303000D0004036761732D00012E010D0003010003000D00041D73797374656D",
    "2E6E657875732E70726F746F636F6C2E76657273696F6E03000D00042F50324B464579466576705166536157384734",
    "566A536D6857555A585234517247395951523148624D7054554370434C03000D00040A53696E676C65566F74650300",
    "0D000409636F6E73656E7375732D00012E010D000223220100AA53BE71FC41BC0889B694F4D6D03F7906A3D9A217",
    "05943CAF9632EEAFBB489503000D0004085370656E6447617303000D0004036761732D00012E010B"
);
const SCRIPT_BUILDER_FIXTURE_SHA256: &str =
    "81907a6b1df095b84599d8f8d709623e20dadeca2082ab9dffef114c7d0015e0";

fn script_vector_rows() -> Vec<(String, String, String, String)> {
    std::fs::read_to_string("tests/fixtures/classic_script_builder_vectors.tsv")
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with("case_id\t"))
        .map(|line| {
            let parts = line.split('\t').collect::<Vec<_>>();
            assert_eq!(parts.len(), 4, "bad script vector row: {line}");
            (
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
                parts[3].to_string(),
            )
        })
        .collect()
}

fn finish_script(mut builder: ScriptBuilder) -> String {
    builder.end_script_hex().unwrap()
}

fn script_builder_vector(case_id: &str) -> String {
    let main_keys =
        PhantasmaKeys::from_wif("L5UEVHBjujaR1721aZM5Zm5ayjDyamMZS9W35RE9Y9giRkdf3dVx").unwrap();
    let helper_keys =
        PhantasmaKeys::from_wif("KxMn2TgXukYaNXx7tEdjh7qB2YaMgeuKy47j4rvKigHhBuZWeP3r").unwrap();
    let address = helper_keys.address();
    let null = Address::null();

    let mut builder = ScriptBuilder::begin();
    match case_id {
        "consensus_single_vote" => {
            builder
                .allow_gas(main_keys.address(), null, 10_000, 210_000)
                .call_contract(
                    "consensus",
                    "SingleVote",
                    vec![
                        ScriptArg::String(main_keys.address().to_text()),
                        ScriptArg::String("system.nexus.protocol.version".into()),
                        0u64.into(),
                    ],
                )
                .spend_gas(main_keys.address());
        }
        "gas_transfer_spend" => {
            builder
                .allow_gas(address, null, 100_000, 21_000)
                .transfer_tokens("SOUL", address, null, 100_000_000)
                .spend_gas(address);
        }
        "mint_tokens" => {
            builder.mint_tokens("SOUL", address, null, 1);
        }
        "transfer_balance" => {
            builder.transfer_balance("KCAL", address, null);
        }
        "transfer_nft" => {
            builder.transfer_nft("ART", address, null, 42);
        }
        "cross_transfer_token" => {
            builder.cross_transfer_token(null, "SOUL", address, null, 1);
        }
        "cross_transfer_nft" => {
            builder.cross_transfer_nft(null, "ART", address, null, 7);
        }
        "stake_unstake" => {
            builder.stake(address, 7).unstake(address, 8);
        }
        "call_nft" => {
            builder.call_nft("ART", 7, "mint", vec![address.into()]);
        }
        "runtime_array_timestamp" => {
            builder.call_interop(
                "Runtime.Test",
                vec![
                    ScriptArg::Array(vec!["alpha".into(), 7u64.into()]),
                    ScriptArg::Timestamp(1_778_330_400),
                ],
            );
        }
        _ => panic!("unhandled script vector: {case_id}"),
    }
    finish_script(builder)
}

#[test]
fn script_builder_fixture_hash_is_locked() {
    let data = std::fs::read("tests/fixtures/classic_script_builder_vectors.tsv").unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(data)),
        SCRIPT_BUILDER_FIXTURE_SHA256
    );
}

#[test]
fn script_builder_matches_golden_vectors() {
    for (case_id, source, expected_hex, notes) in script_vector_rows() {
        assert_eq!(source, "csharp_sdk", "{notes}");
        assert_eq!(script_builder_vector(&case_id), expected_hex, "{case_id}");
    }
}

#[test]
fn default_sdk_payload_matches_crate_version() {
    // The classic transaction default payload is visible on-chain, so it must
    // move with the crate version instead of retaining a stale release string.
    assert_eq!(SDK_PAYLOAD, format!("RS-SDK-v{SDK_VERSION}").as_bytes());
}

#[test]
fn var_uint_boundaries_match_reference_sdks() {
    // Compact integer boundaries match C#/TS/C++/Go canonical encoding.
    let cases = [
        (0xFC, "FC"),
        (0xFD, "FDFD00"),
        (0xFFFF, "FDFFFF"),
        (0x10000, "FE00000100"),
        (0xFFFF_FFFF, "FEFFFFFFFF"),
        (0x1_0000_0000, "FF0000000001000000"),
    ];
    for (value, expected) in cases {
        let mut writer = BinaryWriter::new();
        writer.write_var_uint(value);
        assert_eq!(hex::encode_upper(writer.bytes()), expected);
        let mut reader = BinaryReader::new(writer.bytes());
        assert_eq!(reader.read_var_uint().unwrap(), value);
        reader.assert_eof().unwrap();
    }
}

#[test]
fn binary_reader_bounds_fail_closed() {
    // Bounds checks reject truncated and oversized inputs before allocation-heavy reads.
    assert!(BinaryReader::new(&[1]).read_u16_le().is_err());
    let mut writer = BinaryWriter::new();
    writer.write_var_uint(4);
    writer.write(b"abcd");
    assert!(BinaryReader::new(writer.bytes()).read_var_bytes(3).is_err());
}

#[test]
fn vm_big_integer_edges_round_trip() {
    // Classic VM BigInteger storage must preserve sign through the padded VM
    // byte shape, not merely through a script-local LOAD round-trip.
    for value in [
        0i64,
        1,
        -1,
        127,
        128,
        -128,
        -129,
        255,
        256,
        -255,
        i64::MAX,
        i64::MIN,
    ] {
        let value = BigInt::from(value);
        let raw = big_int_to_vm_bytes(&value).unwrap();
        assert_eq!(vm_bytes_to_big_int(&raw), value);
    }
}

#[test]
fn vm_big_integer_matches_gen2_csharp_binary_fixtures() {
    // This fixture was generated from Gen2 C# sources and checks the two
    // distinct number encodings: padded BinaryWriter/VMObject storage and
    // unpadded ScriptBuilder LOAD bytes.
    let data = std::fs::read_to_string("tests/fixtures/gen2_csharp_vm_bigint_binary.tsv").unwrap();
    for line in data.lines() {
        if line.is_empty() || line.starts_with('#') || line.starts_with("case_id\t") {
            continue;
        }
        let parts = line.split('\t').collect::<Vec<_>>();
        assert_eq!(parts.len(), 6, "bad fixture row: {line}");
        let case_id = parts[0];
        let value = parts[1].parse::<BigInt>().unwrap();
        let signed_hex = parts[2];
        let io_write_hex = parts[4];
        let script_load_hex = parts[5];

        let raw = big_int_to_vm_bytes(&value).unwrap();
        assert_eq!(hex::encode(&raw), signed_hex, "{case_id} signed bytes");
        assert_eq!(vm_bytes_to_big_int(&raw), value, "{case_id} round-trip");

        let mut writer = BinaryWriter::new();
        writer.write_big_integer(&value).unwrap();
        assert_eq!(
            hex::encode(writer.bytes()),
            io_write_hex,
            "{case_id} WriteBigInteger"
        );

        let mut object_bytes = vec![ClassicVMType::Number as u8];
        object_bytes.extend_from_slice(writer.bytes());
        assert_eq!(
            VMObject::from_bytes(&object_bytes)
                .unwrap()
                .as_number()
                .unwrap(),
            value,
            "{case_id} VMObject(Number)"
        );

        let mut builder = ScriptBuilder::begin();
        builder.emit_load_number(0, &value);
        assert_eq!(
            hex::encode(builder.to_script().unwrap()),
            script_load_hex,
            "{case_id} ScriptBuilder LOAD"
        );
    }
}

#[test]
fn vm_object_as_number_matches_gen2_csharp_fixtures() {
    // VMObject.AsNumber is part of script-result decoding, so SDK conversion
    // behavior must match Gen2 for numeric strings, byte payloads, and hash
    // objects instead of only for VMType::Number values.
    let data = std::fs::read_to_string("tests/fixtures/gen2_csharp_vmobject_asnumber.tsv").unwrap();
    for line in data.lines() {
        if line.is_empty() || line.starts_with('#') || line.starts_with("case_id\t") {
            continue;
        }
        let parts = line.split('\t').collect::<Vec<_>>();
        assert!(parts.len() >= 6, "bad fixture row: {line}");
        let case_id = parts[0];
        let source_kind = parts[1];
        let payload = parts[3];
        let outcome = parts[4];
        let expected = parts[5];

        let object = match source_kind {
            "empty" => VMObject::None,
            "string" => VMObject::String(payload.to_string()),
            "bytes" => VMObject::Bytes(hex::decode(payload).unwrap()),
            "bool" => VMObject::Bool(payload == "true"),
            "enum" => VMObject::Enum(payload.parse().unwrap()),
            "timestamp" => VMObject::Timestamp(payload.parse().unwrap()),
            "number" => VMObject::Number(payload.parse().unwrap()),
            "object" => VMObject::Object(hex::decode(payload).unwrap()),
            "struct" => VMObject::Struct(vec![
                (
                    VMObject::String("name".into()),
                    VMObject::String("neo".into()),
                ),
                (
                    VMObject::String("count".into()),
                    VMObject::Number(BigInt::from(7)),
                ),
            ]),
            _ => panic!("unsupported fixture source kind: {source_kind}"),
        };

        let result = object.as_number();
        if outcome == "ok" {
            assert_eq!(
                result.unwrap(),
                expected.parse::<BigInt>().unwrap(),
                "{case_id}"
            );
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }
}

#[test]
fn script_builder_matches_shared_consensus_vector() {
    // Shared consensus vector proves address and numeric argument VM encoding.
    let keys =
        PhantasmaKeys::from_wif("L5UEVHBjujaR1721aZM5Zm5ayjDyamMZS9W35RE9Y9giRkdf3dVx").unwrap();
    let mut builder = ScriptBuilder::begin();
    builder
        .allow_gas(keys.address(), Address::null(), 10_000, 210_000)
        .call_contract(
            "consensus",
            "SingleVote",
            vec![
                ScriptArg::String(keys.address().to_text()),
                ScriptArg::String("system.nexus.protocol.version".into()),
                0u64.into(),
            ],
        )
        .spend_gas(keys.address());
    assert_eq!(
        builder.end_script_hex().unwrap(),
        EXPECTED_CONSENSUS_SINGLE_VOTE
    );
}

#[test]
fn script_builder_resolves_labels_per_instance() {
    // Label resolution is scoped to each builder instance.
    let mut builder = ScriptBuilder::begin();
    builder
        .emit_jump(Opcode::Jmp, "done", 0)
        .emit_load_string(0, "unused")
        .emit_label("DONE");
    let script = builder.end_script().unwrap();
    let target = u16::from_le_bytes([script[1], script[2]]) as usize;
    assert_eq!(script[target - 1], Opcode::Nop as u8);
    assert_eq!(script[target], Opcode::Ret as u8);
    assert_eq!(
        ScriptBuilder::begin().end_script().unwrap(),
        vec![Opcode::Ret as u8]
    );
}

#[test]
fn script_builder_runtime_helper_parity() {
    let keys =
        PhantasmaKeys::from_wif("KxMn2TgXukYaNXx7tEdjh7qB2YaMgeuKy47j4rvKigHhBuZWeP3r").unwrap();
    let address = keys.address();
    let null = Address::null();

    let mut left = ScriptBuilder::begin();
    left.mint_tokens("SOUL", address, null, 1);
    let mut right = ScriptBuilder::begin();
    right.call_interop(
        "Runtime.MintTokens",
        vec![address.into(), null.into(), "SOUL".into(), 1u64.into()],
    );
    assert_eq!(left.end_script().unwrap(), right.end_script().unwrap());

    let mut left = ScriptBuilder::begin();
    left.transfer_balance("SOUL", address, null);
    let mut right = ScriptBuilder::begin();
    right.call_interop(
        "Runtime.TransferBalance",
        vec![address.into(), null.into(), "SOUL".into()],
    );
    assert_eq!(left.end_script().unwrap(), right.end_script().unwrap());

    let mut left = ScriptBuilder::begin();
    left.transfer_nft("ART", address, null, 42);
    let mut right = ScriptBuilder::begin();
    right.call_interop(
        "Runtime.TransferToken",
        vec![address.into(), null.into(), "ART".into(), 42u64.into()],
    );
    assert_eq!(left.end_script().unwrap(), right.end_script().unwrap());

    let mut left = ScriptBuilder::begin();
    left.cross_transfer_token(null, "SOUL", address, null, 1);
    let mut right = ScriptBuilder::begin();
    right.call_interop(
        "Runtime.SendTokens",
        vec![
            null.into(),
            address.into(),
            null.into(),
            "SOUL".into(),
            1u64.into(),
        ],
    );
    assert_eq!(left.end_script().unwrap(), right.end_script().unwrap());

    let mut left = ScriptBuilder::begin();
    left.call_nft("ART", 7, "mint", vec![address.into()]);
    let mut right = ScriptBuilder::begin();
    right.call_contract("ART#7", "mint", vec![address.into()]);
    assert_eq!(left.end_script().unwrap(), right.end_script().unwrap());
}

#[test]
fn script_builder_all_text_helpers_match_typed_helpers() {
    let keys =
        PhantasmaKeys::from_wif("KxMn2TgXukYaNXx7tEdjh7qB2YaMgeuKy47j4rvKigHhBuZWeP3r").unwrap();
    let address = keys.address();
    let address_text = address.to_text();
    let null = Address::null();

    macro_rules! assert_same_script {
        ($left:expr, $right:expr) => {{
            let mut left = ScriptBuilder::begin();
            $left(&mut left);
            let mut right = ScriptBuilder::begin();
            $right(&mut right);
            assert_eq!(left.end_script().unwrap(), right.end_script().unwrap());
        }};
    }

    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.allow_gas_text(&address_text, "NULL", 1, 2);
        },
        |b: &mut ScriptBuilder| {
            b.allow_gas(address, null, 1, 2);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.spend_gas_text(&address_text);
        },
        |b: &mut ScriptBuilder| {
            b.spend_gas(address);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.transfer_tokens_text("KCAL", &address_text, "NULL", 3);
        },
        |b: &mut ScriptBuilder| {
            b.transfer_tokens("KCAL", address, null, 3);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.mint_tokens_text("KCAL", &address_text, "NULL", 3);
        },
        |b: &mut ScriptBuilder| {
            b.mint_tokens("KCAL", address, null, 3);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.transfer_tokens_to_text("KCAL", address, "NULL", 3);
        },
        |b: &mut ScriptBuilder| {
            b.transfer_tokens("KCAL", address, null, 3);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.transfer_balance_text("KCAL", &address_text, "NULL");
        },
        |b: &mut ScriptBuilder| {
            b.transfer_balance("KCAL", address, null);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.transfer_nft_text("ART", &address_text, "NULL", 4);
        },
        |b: &mut ScriptBuilder| {
            b.transfer_nft("ART", address, null, 4);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.transfer_nft_to_text("ART", address, "NULL", 4);
        },
        |b: &mut ScriptBuilder| {
            b.transfer_nft("ART", address, null, 4);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.cross_transfer_token_text("NULL", "KCAL", &address_text, "NULL", 5);
        },
        |b: &mut ScriptBuilder| {
            b.cross_transfer_token(null, "KCAL", address, null, 5);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.cross_transfer_token_to_text(null, "KCAL", address, "NULL", 5);
        },
        |b: &mut ScriptBuilder| {
            b.cross_transfer_token(null, "KCAL", address, null, 5);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.cross_transfer_nft_text("NULL", "ART", &address_text, "NULL", 6);
        },
        |b: &mut ScriptBuilder| {
            b.cross_transfer_nft(null, "ART", address, null, 6);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.cross_transfer_nft_to_text(null, "ART", address, "NULL", 6);
        },
        |b: &mut ScriptBuilder| {
            b.cross_transfer_nft(null, "ART", address, null, 6);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.stake_text(&address_text, 7);
        },
        |b: &mut ScriptBuilder| {
            b.stake(address, 7);
        }
    );
    assert_same_script!(
        |b: &mut ScriptBuilder| {
            b.unstake_text(&address_text, 8);
        },
        |b: &mut ScriptBuilder| {
            b.unstake(address, 8);
        }
    );
}

#[test]
fn script_builder_reports_and_rejects_invalid_operations() {
    let mut builder = ScriptBuilder::begin();
    let (script, error) = builder
        .transfer_tokens_text("SOUL", "bad-address", "NULL", 1)
        .end_script_with_error();
    assert!(script.is_empty());
    assert!(error.is_some());

    let mut builder = ScriptBuilder::begin();
    assert!(builder
        .emit_jump(Opcode::Ret, "done", 0)
        .end_script()
        .is_err());
    let mut builder = ScriptBuilder::begin();
    assert!(builder
        .emit_conditional_jump(Opcode::Jmp, 0, "done")
        .end_script()
        .is_err());
    let mut builder = ScriptBuilder::begin();
    assert!(builder.emit_call("done", 0).end_script().is_err());
    let mut builder = ScriptBuilder::begin();
    assert!(builder
        .emit_load(0, vec![0u8; 0x10000], ClassicVMType::Bytes)
        .end_script()
        .is_err());
    let mut builder = ScriptBuilder::begin();
    let (_script, error) = builder
        .call_interop(
            "Runtime.Time",
            vec![ScriptArg::Timestamp(u64::from(u32::MAX) + 1)],
        )
        .end_script_with_error();
    assert!(error
        .unwrap()
        .to_string()
        .contains("timestamp out of VM uint32 range"));
}

#[test]
fn script_builder_array_and_timestamp_argument_paths_are_stable() {
    let mut builder = ScriptBuilder::begin();
    builder.call_interop(
        "Runtime.Test",
        vec![
            ScriptArg::Array(vec!["alpha".into(), 7u64.into()]),
            ScriptArg::Timestamp(1_778_330_400),
        ],
    );
    let script = builder.end_script().unwrap();
    assert!(script.contains(&(Opcode::Cast as u8)));
    assert!(script.contains(&(Opcode::Put as u8)));
    assert!(script.contains(&(Opcode::ExtCall as u8)));
}

#[test]
fn vm_object_decodes_primitives_and_structs() {
    // VMObject decoding is used by RPC script result parsing.
    assert_eq!(
        VMObject::from_bytes(&[0]).unwrap().as_number().unwrap(),
        BigInt::from(0)
    );
    assert_eq!(
        VMObject::from_bytes(&[6, 1]).unwrap().as_string().unwrap(),
        "true"
    );

    let mut writer = BinaryWriter::new();
    writer.write_u8(1);
    writer.write_var_uint(1);
    writer.write_u8(4);
    writer.write_string("name");
    writer.write_u8(4);
    writer.write_string("sdk");
    let VMObject::Struct(items) = VMObject::from_bytes(writer.bytes()).unwrap() else {
        panic!("expected struct");
    };
    let name_key = VMObject::String("name".into());
    let name_value = items
        .iter()
        .find_map(|(key, value)| (key == &name_key).then_some(value))
        .unwrap();
    assert_eq!(name_value.as_string().unwrap(), "sdk");
}

#[test]
fn vm_object_rejects_invalid_or_incompatible_values() {
    assert!(VMObject::from_bytes(&[0xff]).is_err());
    assert!(VMObject::from_bytes(&[6, 1, 0]).is_err());
    assert!(VMObject::Object(vec![0; 34]).as_number().is_err());
    assert!(VMObject::Bytes(vec![0xff]).as_string().is_err());
}

#[test]
fn classic_transaction_signing_round_trip_and_pow() {
    // Classic transaction bytes are the signing payload for VM script broadcasts.
    let keys =
        PhantasmaKeys::from_wif("KxMn2TgXukYaNXx7tEdjh7qB2YaMgeuKy47j4rvKigHhBuZWeP3r").unwrap();
    let mut script = ScriptBuilder::begin();
    script.call_interop("Runtime.Time", []);
    let mut tx = Transaction::new(
        "mainnet",
        "main",
        script.end_script().unwrap(),
        1_754_000_000,
    );
    let signature = tx.sign(&keys);
    assert!(signature.verify(&tx.to_bytes(false), [&keys.address()]));
    assert!(tx.is_signed_by(&keys));
    let decoded = Transaction::from_bytes(&tx.to_bytes(true)).unwrap();
    assert_eq!(decoded.nexus_name, tx.nexus_name);
    assert_eq!(decoded.signatures.len(), 1);

    tx.mine(5);
    assert!(tx.hash().difficulty() >= 5);
}
