use phantasma_sdk::{
    decode_base58, encode_base58, Address, AddressKind, Ed25519Signature, Hash, PhantasmaError,
    PhantasmaKeys,
};
use sha2::{Digest, Sha256};

const ED25519_VECTORS_SHA256: &str =
    "dd747f5c49b49a67f1c63d02351be669558bf9da65571ed7311bcd8cf8d2bd01";

#[test]
fn base58_round_trip_preserves_leading_zeroes() {
    // Address and WIF text depend on Bitcoin Base58 preserving leading zero bytes.
    let payload = b"\0\0hello phantasma";
    assert_eq!(decode_base58(&encode_base58(payload)).unwrap(), payload);
}

#[test]
fn wif_and_address_vectors_match_reference_sdks() {
    // Existing cross-SDK WIF/address vectors must remain byte-stable.
    let cases = [
        (
            "KxMn2TgXukYaNXx7tEdjh7qB2YaMgeuKy47j4rvKigHhBuZWeP3r",
            "P2K9zmyFDNGN6n6hHiTUAz6jqn29s5G1SWLiXwCVQcpHcQb",
        ),
        (
            "L2sTuSzangXQCFxXFXJqfPAKJsstKvQdkGqP9J2VFkFRbEjd1Ez6",
            "P2K65RZhfxZhQcXKGgSPZL6c6hkygXipNxdeuW5FU531Bqc",
        ),
    ];
    for (wif, address) in cases {
        let keys = PhantasmaKeys::from_wif(wif).unwrap();
        assert_eq!(keys.to_wif(), wif);
        assert_eq!(keys.address().to_text(), address);
        assert_eq!(Address::from_text(address).unwrap(), keys.address());
        assert_eq!(keys.address().kind(), AddressKind::User);
    }
}

#[test]
fn signature_verifies_against_derived_address() {
    // Ed25519 signatures validate through the derived Phantasma address.
    let keys =
        PhantasmaKeys::from_wif("KxMn2TgXukYaNXx7tEdjh7qB2YaMgeuKy47j4rvKigHhBuZWeP3r").unwrap();
    let message = b"phantasma-rust-sdk";
    let signature = keys.sign(message);
    assert!(signature.verify(message, [&keys.address()]));
    assert!(!signature.verify(b"bad", [&keys.address()]));
}

#[test]
fn ed25519_matches_shared_golden_vectors() {
    let data = std::fs::read("tests/fixtures/ed25519_vectors.tsv").unwrap();
    let digest = Sha256::digest(&data);
    assert_eq!(hex::encode(digest), ED25519_VECTORS_SHA256);

    let text = std::str::from_utf8(&data).unwrap();
    let mut rows = 0usize;
    for line in text
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .skip(1)
    {
        let parts: Vec<&str> = line.split('\t').collect();
        assert_eq!(parts.len(), 7, "malformed Ed25519 vector row: {line}");

        let case_id = parts[0];
        let seed = hex::decode(parts[2]).unwrap();
        let public_key = hex::decode(parts[3]).unwrap();
        let message = hex::decode(parts[4]).unwrap();
        let signature = hex::decode(parts[5]).unwrap();

        let keys = PhantasmaKeys::try_from_slice(&seed).unwrap();
        assert_eq!(
            keys.public_key().as_slice(),
            public_key.as_slice(),
            "{case_id}"
        );

        let sdk_signature = keys.sign(&message);
        assert_eq!(
            sdk_signature.data().as_slice(),
            signature.as_slice(),
            "{case_id}"
        );

        let expected_signature = Ed25519Signature::try_from_slice(&signature).unwrap();
        assert!(
            expected_signature.verify(&message, [&keys.address()]),
            "{case_id}"
        );

        let mut bad_message = if message.is_empty() {
            vec![0]
        } else {
            message.clone()
        };
        bad_message[0] ^= 0xff;
        assert!(
            !expected_signature.verify(&bad_message, [&keys.address()]),
            "{case_id}"
        );

        rows += 1;
    }
    assert!(rows > 0, "Ed25519 fixture did not contain data rows");
}

#[test]
fn hash_difficulty_matches_phantasma_little_endian_pow() {
    // VM transaction PoW uses the validator/Go/TS little-endian hash convention.
    assert_eq!(Hash::new([0xFF; 32]).difficulty(), 0);
    let mut high = [0xFF; 32];
    high[31] = 0;
    assert_eq!(Hash::new(high).difficulty(), 8);
    assert_eq!(Hash::new([0; 32]).difficulty(), 256);
}

#[test]
fn invalid_crypto_inputs_fail_closed() {
    // User-provided key/address material should return errors instead of panicking.
    assert!(matches!(
        PhantasmaKeys::from_wif("KxMn2TgXukYaNXx7tEdjh7qB2YaMgeuKy47j4rvKigHhBuZWeP3s"),
        Err(PhantasmaError::Crypto(_)) | Err(PhantasmaError::Encoding(_))
    ));
    assert!(Address::from_public_key(b"short").is_err());
    assert!(Address::try_from_slice(&[1]).is_err());
    assert!(Address::from_text("Z111").is_err());
    assert!(Address::null().public_key().is_err());
    assert!(Hash::try_from_slice(&[0]).is_err());
    assert!(Ed25519Signature::try_from_slice(&[0]).is_err());
    assert!(PhantasmaKeys::from_wif("").is_err());
    assert!(PhantasmaKeys::try_from_slice(&[0]).is_err());
}

#[test]
fn null_address_text_round_trip() {
    // NULL is a public API spelling, not a Base58 encoded address.
    assert_eq!(Address::from_text("NULL").unwrap().to_text(), "NULL");
    assert_eq!(Address::from_text("").unwrap().kind(), AddressKind::System);
}
