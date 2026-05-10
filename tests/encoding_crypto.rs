use phantasma_sdk::{
    decode_base58, encode_base58, Address, AddressKind, Ed25519Signature, Hash, PhantasmaError,
    PhantasmaKeys,
};

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
