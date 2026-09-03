//! Envelope tests: the layout the chain reads for each witness form, and the size prediction the
//! gas-model-v2 fee planning is built on (every envelope byte is billed, so the size the planner
//! prices must be the size that goes on the wire).

use phantasma_sdk::{
    bytes32_from_public_key, deserialize, envelope_bytes, required_witnesses, serialize, Bytes32,
    Bytes64, IntX, PhantasmaError, PhantasmaKeys, SignedTxMsg, SmallString, TxMsg,
    TxMsgBurnFungibleGasPayer, TxMsgBurnNonFungibleGasPayer, TxMsgCall, TxMsgTransferFungible,
    TxMsgTransferFungibleGasPayer, TxMsgTransferNonFungibleMultiGasPayer,
    TxMsgTransferNonFungibleSingleGasPayer, TxPayload, TxType, Witness,
};

// Deterministic, non-funded keys: the gas payer and the token owner of a gas-payer transaction.
fn keys() -> (PhantasmaKeys, PhantasmaKeys) {
    (
        PhantasmaKeys::from_wif("KwPpBSByydVKqStGHAnZzQofCqhDmD2bfRgc9BmZqM3ZmsdWJw4d").unwrap(),
        PhantasmaKeys::from_wif("KwVG94yjfVg1YKFyRxAGtug93wdRbmLnqqrFV6Yd2CiA9KZDAp4H").unwrap(),
    )
}

fn address_of(keys: &PhantasmaKeys) -> Bytes32 {
    bytes32_from_public_key(&keys.public_key()).unwrap()
}

fn base_tx(tx_type: TxType, gas_from: Bytes32, msg: TxPayload) -> TxMsg {
    TxMsg {
        tx_type,
        expiry: 1_759_711_416_000,
        max_gas: 10_000_000,
        max_data: 1_000,
        gas_from,
        payload: SmallString::new("p").unwrap(),
        msg,
    }
}

// One message of every gas-payer type, each with `owner` as the token owner and `payer` paying gas.
fn gas_payer_messages(payer: Bytes32, owner: Bytes32, receiver: Bytes32) -> Vec<TxMsg> {
    vec![
        base_tx(
            TxType::TransferFungibleGasPayer,
            payer,
            TxPayload::TransferFungibleGasPayer(TxMsgTransferFungibleGasPayer {
                to: receiver,
                from_address: owner,
                token_id: 1,
                amount: 100_000_000,
            }),
        ),
        base_tx(
            TxType::TransferNonFungibleSingleGasPayer,
            payer,
            TxPayload::TransferNonFungibleSingleGasPayer(TxMsgTransferNonFungibleSingleGasPayer {
                to: receiver,
                from_address: owner,
                token_id: 7,
                instance_id: 42,
            }),
        ),
        base_tx(
            TxType::TransferNonFungibleMultiGasPayer,
            payer,
            TxPayload::TransferNonFungibleMultiGasPayer(TxMsgTransferNonFungibleMultiGasPayer {
                to: receiver,
                from_address: owner,
                token_id: 7,
                instance_ids: vec![42, 43],
            }),
        ),
        base_tx(
            TxType::BurnFungibleGasPayer,
            payer,
            TxPayload::BurnFungibleGasPayer(TxMsgBurnFungibleGasPayer {
                token_id: 1,
                from_address: owner,
                amount: IntX::from(5i64),
            }),
        ),
        base_tx(
            TxType::BurnNonFungibleGasPayer,
            payer,
            TxPayload::BurnNonFungibleGasPayer(TxMsgBurnNonFungibleGasPayer {
                token_id: 7,
                from_address: owner,
                instance_id: 42,
            }),
        ),
    ]
}

fn raw_signature(keys: &PhantasmaKeys, msg: &TxMsg) -> Bytes64 {
    Bytes64(*keys.sign(serialize(msg).unwrap()).data())
}

fn witness(keys: &PhantasmaKeys, msg: &TxMsg) -> Witness {
    Witness {
        address: address_of(keys),
        signature: raw_signature(keys, msg),
    }
}

fn serialization_message(err: PhantasmaError) -> String {
    match err {
        PhantasmaError::Serialization(message) => message,
        other => panic!("expected a serialization error, got {other:?}"),
    }
}

// The chain reads the gas payer's signature first and the owner's second, so the envelope is the
// unsigned message followed by exactly those two bare signatures in that order. A wrong order is
// not a decoding error - both signatures verify against addresses the chain resolves itself - so
// only the byte layout can catch it.
#[test]
fn gas_payer_envelopes_write_the_gas_signature_before_the_from_signature() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    for msg in gas_payer_messages(payer, owner, Bytes32([0x33; 32])) {
        let unsigned = serialize(&msg).unwrap();
        let gas_sig = raw_signature(&payer_keys, &msg);
        let from_sig = raw_signature(&owner_keys, &msg);
        let signed = SignedTxMsg {
            msg: msg.clone(),
            witnesses: vec![witness(&payer_keys, &msg), witness(&owner_keys, &msg)],
        };
        let encoded = serialize(&signed).unwrap();

        assert_eq!(encoded.len(), unsigned.len() + 128, "{:?}", msg.tx_type);
        assert_eq!(&encoded[..unsigned.len()], &unsigned[..]);
        assert_eq!(
            &encoded[unsigned.len()..unsigned.len() + 64],
            &gas_sig.0[..]
        );
        assert_eq!(&encoded[unsigned.len() + 64..], &from_sig.0[..]);
        assert!(payer_keys
            .sign(&unsigned)
            .verify(&unsigned, [&payer_keys.address()]));

        // The envelope does not carry the second witness's address; the reader must recover it
        // from the payload's `from_address`, exactly as the node does.
        let decoded: SignedTxMsg = deserialize(&encoded).unwrap();
        assert_eq!(decoded.witnesses.len(), 2);
        assert_eq!(decoded.witnesses[0].address, payer);
        assert_eq!(decoded.witnesses[1].address, owner);
        assert_eq!(serialize(&decoded).unwrap(), encoded);
    }
}

#[test]
fn envelopes_refuse_witness_sets_that_do_not_match_the_message() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let msg = gas_payer_messages(payer, owner, Bytes32([0x33; 32])).remove(0);
    let gas = witness(&payer_keys, &msg);
    let from = witness(&owner_keys, &msg);

    let cases: Vec<(Vec<Witness>, &str)> = vec![
        (vec![gas.clone()], "expects 2 witnesses"),
        (
            vec![from.clone(), gas.clone()],
            "gas witness address mismatch",
        ),
        (
            vec![gas.clone(), gas.clone()],
            "from witness address mismatch",
        ),
    ];
    for (witnesses, expected) in cases {
        let signed = SignedTxMsg {
            msg: msg.clone(),
            witnesses,
        };
        let message = serialization_message(serialize(&signed).unwrap_err());
        assert!(message.contains(expected), "{message}");
    }

    // A witness-array envelope that omits the gas payer would be rejected by the node as "not
    // signed by gas payer"; the SDK refuses to produce it.
    let call = base_tx(
        TxType::Call,
        payer,
        TxPayload::Call(TxMsgCall {
            module_id: 1,
            method_id: 2,
            args: vec![0; 10],
            sections: None,
        }),
    );
    let without = SignedTxMsg {
        msg: call.clone(),
        witnesses: vec![witness(&owner_keys, &call)],
    };
    let message = serialization_message(serialize(&without).unwrap_err());
    assert!(message.contains("gas payer must be one of the witnesses"));
}

// The envelope size decides the v2 gas bill, and a wallet must know it before anyone signs: the
// placeholder-witness size must equal the size of the really signed transaction.
#[test]
fn envelope_bytes_predicts_the_signed_size() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let receiver = Bytes32([0x33; 32]);

    for msg in gas_payer_messages(payer, owner, receiver) {
        let signed = SignedTxMsg {
            msg: msg.clone(),
            witnesses: vec![witness(&payer_keys, &msg), witness(&owner_keys, &msg)],
        };
        let want = serialize(&signed).unwrap().len() as u32;
        assert_eq!(envelope_bytes(&msg, None).unwrap(), want);
        assert_eq!(envelope_bytes(&msg, Some(2)).unwrap(), want);
        assert!(envelope_bytes(&msg, Some(1))
            .unwrap_err()
            .to_string()
            .contains("carries 2 witness(es)"));
    }

    let native = base_tx(
        TxType::TransferFungible,
        payer,
        TxPayload::TransferFungible(TxMsgTransferFungible {
            to: receiver,
            token_id: 1,
            amount: 1,
        }),
    );
    let native_signed = SignedTxMsg {
        msg: native.clone(),
        witnesses: vec![witness(&payer_keys, &native)],
    };
    assert_eq!(
        envelope_bytes(&native, None).unwrap(),
        serialize(&native_signed).unwrap().len() as u32
    );

    let call = base_tx(
        TxType::Call,
        payer,
        TxPayload::Call(TxMsgCall {
            module_id: 1,
            method_id: 2,
            args: vec![0; 10],
            sections: None,
        }),
    );
    let third = PhantasmaKeys::generate();
    let signers = [&payer_keys, &owner_keys, &third];
    for count in 1..=3usize {
        let signed = SignedTxMsg {
            msg: call.clone(),
            witnesses: signers[..count].iter().map(|k| witness(k, &call)).collect(),
        };
        assert_eq!(
            envelope_bytes(&call, Some(count as u32)).unwrap(),
            serialize(&signed).unwrap().len() as u32
        );
    }
    assert_eq!(
        envelope_bytes(&call, None).unwrap(),
        envelope_bytes(&call, Some(1)).unwrap()
    );
    assert!(envelope_bytes(&call, Some(0)).is_err());
}

#[test]
fn required_witnesses_names_the_envelope_order() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    for msg in gas_payer_messages(payer, owner, Bytes32([0x33; 32])) {
        assert_eq!(required_witnesses(&msg), Some(vec![payer, owner]));
    }
    let native = base_tx(
        TxType::TransferFungible,
        payer,
        TxPayload::TransferFungible(TxMsgTransferFungible {
            to: owner,
            token_id: 1,
            amount: 1,
        }),
    );
    assert_eq!(required_witnesses(&native), Some(vec![payer]));
    // Witness-array types leave the witness set to the caller; a raw Phantasma transaction has none.
    let call = base_tx(
        TxType::Call,
        payer,
        TxPayload::Call(TxMsgCall {
            module_id: 1,
            method_id: 2,
            args: vec![],
            sections: None,
        }),
    );
    assert_eq!(required_witnesses(&call), None);
    let raw = base_tx(
        TxType::PhantasmaRaw,
        payer,
        TxPayload::PhantasmaRaw(phantasma_sdk::TxMsgPhantasmaRaw {
            transaction: vec![1],
        }),
    );
    assert_eq!(required_witnesses(&raw), Some(Vec::new()));
}
