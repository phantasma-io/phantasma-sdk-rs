use num_bigint::BigInt;
use phantasma_sdk::vm::VMType as ClassicVMType;
use phantasma_sdk::{
    big_int_to_vm_bytes, vm_bytes_to_big_int, BinaryWriter, ScriptBuilder, VMObject,
};
use sha2::{Digest, Sha256};

const UNIT_COVERED_GEN2_FIXTURES: &[&str] = &[
    "gen2_csharp_vm_bigint_binary.tsv",
    "gen2_csharp_vm_bigint_decimal.tsv",
    "gen2_csharp_vmobject_arraytype.tsv",
    "gen2_csharp_vmobject_asbool.tsv",
    "gen2_csharp_vmobject_asbytes.tsv",
    "gen2_csharp_vmobject_asnumber.tsv",
    "gen2_csharp_vmobject_asstring.tsv",
    "gen2_csharp_vmobject_cast_struct.tsv",
    "gen2_csharp_vmobject_serde.tsv",
];

const LIVE_RUNNER_COVERED_GEN2_FIXTURES: &[&str] = &[
    "gen2_csharp_vm_scriptcontext_ops.tsv",
    "gen2_csharp_vm_scriptcontext_unary.tsv",
];

const NOT_SDK_UNIT_APPLICABLE_GEN2_FIXTURES: &[&str] = &[
    "gen2_csharp_vm_bigint_narrow_int.tsv",
    "gen2_csharp_vm_bigint_ops.tsv",
    "gen2_csharp_vm_bigint_unary_ops.tsv",
];

const GEN2_FIXTURE_SHA256: &[(&str, &str)] = &[
    (
        "gen2_csharp_vm_bigint_binary.tsv",
        "a5be05751b35de8b7b3578577bb2769073ac7a2ddea3eaf9503d76d0302fa464",
    ),
    (
        "gen2_csharp_vm_bigint_decimal.tsv",
        "1bede4198883018817d94eceefe4e7b70a9f5c96c9d60d57481990ad21b027a9",
    ),
    (
        "gen2_csharp_vm_bigint_narrow_int.tsv",
        "b82315b4483c23ee7e3e9943b5c41cf8daf12c627c6e12f30b735ad7dbde1445",
    ),
    (
        "gen2_csharp_vm_bigint_ops.tsv",
        "997f3a935393358a89c7be785176e8528535111994bc5193c7d7ddc2429aa3d3",
    ),
    (
        "gen2_csharp_vm_bigint_unary_ops.tsv",
        "53719de8a1528897a083401aaad251cdb3e9e201f8639d29cd3708beeda93ea7",
    ),
    (
        "gen2_csharp_vm_scriptcontext_ops.tsv",
        "c87e4a5ec075b8efc0abe88a551ae8fe505df04167cb0e4f2714768c0a1e917f",
    ),
    (
        "gen2_csharp_vm_scriptcontext_unary.tsv",
        "7198d33a84bd61c671dc1871f2b56e232748c41d69e957e8f994cd2dc9b5922c",
    ),
    (
        "gen2_csharp_vmobject_arraytype.tsv",
        "f6b7ce9cd92f464d260018ffb1a0ab01202ca908cf915dead3295e8270ddf532",
    ),
    (
        "gen2_csharp_vmobject_asbool.tsv",
        "a2979cc7eccd22760de82f8401de4b8b41c45fedf09b91a94871d3a3051c85d5",
    ),
    (
        "gen2_csharp_vmobject_asbytes.tsv",
        "dd326e18c94e2e116705893f742c708cfb1cd7b96c8a40a2ab6637b39ae409b9",
    ),
    (
        "gen2_csharp_vmobject_asnumber.tsv",
        "986cfc21658c66b04c1ffaaa7bb9fa08bc9a3acd929276d0d2496ba43c43bf69",
    ),
    (
        "gen2_csharp_vmobject_asstring.tsv",
        "eb14408b7e65fc417bf1bbfe4fb1e87c3d06d28734c7c25514a806f41fceede6",
    ),
    (
        "gen2_csharp_vmobject_cast_struct.tsv",
        "1580a9ec312619a7e2632076073ae80d57dcfc3defc0ef7b4876da34c0e231af",
    ),
    (
        "gen2_csharp_vmobject_serde.tsv",
        "0c74c90e83c5c20bed48b1d52ca5489d15a7c4f67874184c1d0a4f708ce5e42f",
    ),
];

#[test]
fn phantasma_bigint_vectors_match_vm_writer_and_script_builder() {
    let data = std::fs::read_to_string("tests/fixtures/phantasma_bigint_vectors.tsv").unwrap();
    for (line_number, line) in data.lines().enumerate() {
        if line_number == 0 || line.is_empty() {
            continue;
        }
        let parts = line.split('\t').collect::<Vec<_>>();
        assert_eq!(parts.len(), 3, "bad fixture row: {line}");
        let value = parts[0].parse::<BigInt>().unwrap();
        let expected_vm_bytes = parse_decimal_bytes(parts[1]);
        let expected_csharp_bytes = parse_decimal_bytes(parts[2]);

        let vm_bytes = big_int_to_vm_bytes(&value).unwrap();
        assert_eq!(vm_bytes, expected_vm_bytes, "VM bytes for {}", parts[0]);
        assert_eq!(
            vm_bytes_to_big_int(&vm_bytes),
            value,
            "VM roundtrip {}",
            parts[0]
        );

        let mut writer = BinaryWriter::new();
        writer.write_big_integer(&value).unwrap();
        let mut expected_writer = BinaryWriter::new();
        expected_writer.write_var_bytes(&expected_vm_bytes);
        assert_eq!(
            writer.bytes(),
            expected_writer.bytes(),
            "writer {}",
            parts[0]
        );

        let mut builder = ScriptBuilder::begin();
        builder.emit_load_number(0, &value);
        let script = builder.to_script().unwrap();
        assert_eq!(script[0], 13, "LOAD opcode {}", parts[0]);
        assert_eq!(script[1], 0, "LOAD register {}", parts[0]);
        assert_eq!(
            script[2],
            ClassicVMType::Number as u8,
            "LOAD type {}",
            parts[0]
        );
        assert_eq!(
            usize::from(script[3]),
            expected_csharp_bytes.len(),
            "LOAD length {}",
            parts[0]
        );
        assert_eq!(
            &script[4..],
            expected_csharp_bytes.as_slice(),
            "LOAD bytes {}",
            parts[0]
        );
    }
}

#[test]
fn gen2_fixture_manifest_is_explicit() {
    let mut discovered = std::fs::read_dir("tests/fixtures")
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| name.starts_with("gen2_csharp_") && name.ends_with(".tsv"))
        .collect::<Vec<_>>();
    discovered.sort();

    let mut classified = UNIT_COVERED_GEN2_FIXTURES
        .iter()
        .chain(LIVE_RUNNER_COVERED_GEN2_FIXTURES)
        .chain(NOT_SDK_UNIT_APPLICABLE_GEN2_FIXTURES)
        .map(|name| (*name).to_string())
        .collect::<Vec<_>>();
    classified.sort();

    assert_eq!(discovered, classified);
}

#[test]
fn gen2_fixture_hashes_are_locked() {
    let mut classified = UNIT_COVERED_GEN2_FIXTURES
        .iter()
        .chain(LIVE_RUNNER_COVERED_GEN2_FIXTURES)
        .chain(NOT_SDK_UNIT_APPLICABLE_GEN2_FIXTURES)
        .copied()
        .collect::<Vec<_>>();
    classified.sort();

    let mut hashed = GEN2_FIXTURE_SHA256
        .iter()
        .map(|(name, _hash)| *name)
        .collect::<Vec<_>>();
    hashed.sort();
    assert_eq!(hashed, classified);

    for (name, expected) in GEN2_FIXTURE_SHA256 {
        let data = std::fs::read(format!("tests/fixtures/{name}")).unwrap();
        let digest = Sha256::digest(&data);
        assert_eq!(hex::encode(digest), *expected, "{name}");
    }
}

#[test]
fn vmobject_as_string_matches_gen2_csharp_fixtures() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_asstring.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        assert_eq!(parts[4], "ok", "{case_id}");
        assert_eq!(object.as_string().unwrap(), parts[5], "{case_id}");
    }
}

#[test]
fn vmobject_string_as_number_matches_gen2_csharp_decimal_fixtures() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vm_bigint_decimal.tsv") {
        let case_id = &parts[0];
        let object = VMObject::String(parts[1].clone());
        let result = object.as_number();
        if parts[2] == "ok" {
            assert_eq!(
                result.unwrap(),
                parts[3].parse::<BigInt>().unwrap(),
                "{case_id}"
            );
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }
}

#[test]
fn vmobject_as_bytes_matches_gen2_csharp_fixtures() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_asbytes.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        let result = object.as_bytes();
        if parts[4] == "ok" {
            assert_eq!(hex::encode(result.unwrap()), parts[5], "{case_id}");
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }
}

#[test]
fn vmobject_as_bool_matches_gen2_csharp_fixtures() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_asbool.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        let result = object.as_bool();
        if parts[4] == "ok" {
            assert_eq!(result.unwrap().to_string(), parts[5], "{case_id}");
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }
}

#[test]
fn vmobject_array_type_matches_gen2_csharp_fixtures() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_arraytype.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        assert_eq!(format!("{:?}", object.array_type()), parts[4], "{case_id}");
    }
}

#[test]
fn vmobject_serde_matches_gen2_csharp_fixtures() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_serde.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        assert_eq!(
            hex::encode(object.to_bytes().unwrap()),
            parts[4],
            "{case_id}"
        );

        let roundtrip = VMObject::from_bytes(&hex::decode(&parts[4]).unwrap()).unwrap();
        assert_eq!(
            format!("{:?}", roundtrip.vm_type()),
            parts[5],
            "{case_id} type"
        );
        assert_eq!(
            object_descriptor(&roundtrip),
            parts[6],
            "{case_id} descriptor"
        );
    }
}

#[test]
fn vmobject_cast_struct_matches_gen2_csharp_fixtures() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_cast_struct.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        let result = object.cast_to(ClassicVMType::Struct);
        if parts[4] == "ok" {
            let result = result.unwrap();
            assert_eq!(format!("{:?}", result.vm_type()), parts[5], "{case_id}");
            assert_eq!(object_descriptor(&result), parts[6], "{case_id}");
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }
}

#[test]
fn vmobject_cast_to_common_targets_matches_gen2_conversion_fixtures() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_asstring.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        let result = object.cast_to(ClassicVMType::String);
        if parts[4] == "ok" {
            assert_eq!(
                result.unwrap(),
                VMObject::String(parts[5].clone()),
                "{case_id}"
            );
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }

    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_asbytes.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        let result = object.cast_to(ClassicVMType::Bytes);
        if parts[4] == "ok" {
            assert_eq!(
                result.unwrap(),
                VMObject::Bytes(hex::decode(&parts[5]).unwrap()),
                "{case_id}"
            );
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }

    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_asnumber.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        let result = object.cast_to(ClassicVMType::Number);
        if parts[4] == "ok" {
            assert_eq!(
                result.unwrap(),
                VMObject::Number(parts[5].parse().unwrap()),
                "{case_id}"
            );
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }

    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_asbool.tsv") {
        let case_id = &parts[0];
        let object = object_from_fixture(&parts[1], &parts[3]);
        let result = object.cast_to(ClassicVMType::Bool);
        if parts[4] == "ok" {
            assert_eq!(
                result.unwrap(),
                VMObject::Bool(parts[5] == "true"),
                "{case_id}"
            );
        } else {
            assert!(result.is_err(), "{case_id}");
        }
    }
}

#[test]
fn vmobject_serde_fixture_payloads_reject_truncation() {
    for parts in fixture_rows("tests/fixtures/gen2_csharp_vmobject_serde.tsv") {
        let case_id = &parts[0];
        let payload = hex::decode(&parts[4]).unwrap();
        assert!(!payload.is_empty(), "{case_id}");
        assert!(
            VMObject::from_bytes(&payload[..payload.len() - 1]).is_err(),
            "{case_id}"
        );
    }
}

fn parse_decimal_bytes(value: &str) -> Vec<u8> {
    if value.trim().is_empty() {
        return Vec::new();
    }
    value
        .split_whitespace()
        .map(|part| part.parse::<u8>().unwrap())
        .collect()
}

fn fixture_rows(path: &str) -> Vec<Vec<String>> {
    let mut width = 0usize;
    let mut rows = Vec::new();
    for line in std::fs::read_to_string(path).unwrap().lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line
            .split('\t')
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        if line.starts_with("case_id\t") {
            width = parts.len();
            continue;
        }
        if width != 0 && parts.len() < width {
            parts.resize(width, String::new());
        }
        rows.push(parts);
    }
    rows
}

fn object_from_fixture(source_kind: &str, payload: &str) -> VMObject {
    match source_kind {
        "serialized_vmobject" => VMObject::from_bytes(&hex::decode(payload).unwrap()).unwrap(),
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
        other => panic!("unsupported fixture source kind: {other}"),
    }
}

fn object_descriptor(object: &VMObject) -> String {
    match object {
        VMObject::None => "None".to_string(),
        VMObject::Struct(_) => format!("Struct:{}", hex::encode(object.to_bytes().unwrap())),
        VMObject::Bytes(value) => format!("Bytes:{}", hex::encode(value)),
        VMObject::Number(value) => format!("Number:{value}"),
        VMObject::String(value) => format!("String:{value}"),
        VMObject::Timestamp(value) => format!("Timestamp:{value}"),
        VMObject::Bool(value) => format!("Bool:{value}"),
        VMObject::Enum(value) => format!("Enum:{value}"),
        VMObject::Object(value) if value.len() == 34 => {
            format!("Object.Address:{}", hex::encode(value))
        }
        VMObject::Object(value) if value.len() == 32 => {
            format!("Object.Hash:{}", hex::encode(value))
        }
        VMObject::Object(value) => format!("Object:{}", hex::encode(value)),
    }
}
