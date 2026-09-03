//! Multi-witness signing: the signer set is matched to the envelope slots the message fixes, or
//! taken in the caller's order for the witness-array types, and every witness signs the same bytes.

use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use phantasma_sdk::{
    bytes32_from_public_key, deserialize, serialize, sign_and_serialize_tx_msg,
    sign_and_serialize_tx_msg_with, sign_and_serialize_tx_msg_with_keys, sign_tx_msg,
    sign_tx_msg_with, sign_tx_msg_with_keys, Bytes32, PhantasmaError, PhantasmaKeys, Result,
    SignedTxMsg, SmallString, TxMsg, TxMsgCall, TxMsgPhantasmaRaw, TxMsgTransferFungible,
    TxMsgTransferFungibleGasPayer, TxPayload, TxSigner, TxType, PUBLIC_KEY_LENGTH,
    SIGNATURE_LENGTH,
};

fn keys() -> (PhantasmaKeys, PhantasmaKeys) {
    (
        PhantasmaKeys::from_wif("KwPpBSByydVKqStGHAnZzQofCqhDmD2bfRgc9BmZqM3ZmsdWJw4d").unwrap(),
        PhantasmaKeys::from_wif("KwVG94yjfVg1YKFyRxAGtug93wdRbmLnqqrFV6Yd2CiA9KZDAp4H").unwrap(),
    )
}

fn address_of(keys: &PhantasmaKeys) -> Bytes32 {
    bytes32_from_public_key(&keys.public_key()).unwrap()
}

fn gas_payer_transfer(payer: Bytes32, owner: Bytes32, to: Bytes32) -> TxMsg {
    TxMsg {
        tx_type: TxType::TransferFungibleGasPayer,
        expiry: 1_759_711_416_000,
        max_gas: 10_000_000,
        max_data: 0,
        gas_from: payer,
        payload: SmallString::default(),
        msg: TxPayload::TransferFungibleGasPayer(TxMsgTransferFungibleGasPayer {
            to,
            from_address: owner,
            token_id: 1,
            amount: 5,
        }),
    }
}

fn call_tx(gas_from: Bytes32) -> TxMsg {
    TxMsg {
        tx_type: TxType::Call,
        expiry: 1_759_711_416_000,
        max_gas: 10_000_000,
        max_data: 0,
        gas_from,
        payload: SmallString::default(),
        msg: TxPayload::Call(TxMsgCall {
            module_id: 1,
            method_id: 1,
            args: vec![1, 2, 3],
            sections: None,
        }),
    }
}

fn native_transfer(from: Bytes32, to: Bytes32, max_gas: u64) -> TxMsg {
    TxMsg {
        tx_type: TxType::TransferFungible,
        expiry: 1,
        max_gas,
        max_data: 0,
        gas_from: from,
        payload: SmallString::default(),
        msg: TxPayload::TransferFungible(TxMsgTransferFungible {
            to,
            token_id: 1,
            amount: 1,
        }),
    }
}

fn builder_message(err: PhantasmaError) -> String {
    match err {
        PhantasmaError::Builder(message) => message,
        other => panic!("expected a builder error, got {other:?}"),
    }
}

// A signer that answers through the trait the way a hardware wallet or a signing service would,
// and counts how often it was asked.
struct CountingSigner {
    keys: PhantasmaKeys,
    calls: AtomicUsize,
}

impl CountingSigner {
    fn new(keys: PhantasmaKeys) -> Self {
        Self {
            keys,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl TxSigner for CountingSigner {
    fn public_key(&self) -> [u8; PUBLIC_KEY_LENGTH] {
        self.keys.public_key()
    }

    async fn sign_message(&self, message: &[u8]) -> Result<[u8; SIGNATURE_LENGTH]> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(*self.keys.sign(message).data())
    }
}

#[test]
fn keys_are_ordered_into_the_envelope_order() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let msg = gas_payer_transfer(payer, owner, Bytes32([0x33; 32]));

    // Keys are given owner first; the envelope wants the gas payer first, as the node reads them.
    let signed = sign_tx_msg_with_keys(&msg, &[&owner_keys, &payer_keys]).unwrap();
    assert_eq!(signed.witnesses[0].address, payer);
    assert_eq!(signed.witnesses[1].address, owner);
    let message = serialize(&msg).unwrap();
    assert!(payer_keys
        .sign(&message)
        .verify(&message, [&payer_keys.address()]));
    assert_eq!(
        signed.witnesses[0].signature.0,
        *payer_keys.sign(&message).data()
    );
    assert_eq!(
        signed.witnesses[1].signature.0,
        *owner_keys.sign(&message).data()
    );
    let encoded = serialize(&signed).unwrap();
    let decoded: SignedTxMsg = deserialize(&encoded).unwrap();
    assert_eq!(serialize(&decoded).unwrap(), encoded);
}

#[tokio::test]
async fn signers_sign_through_the_trait() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let msg = gas_payer_transfer(payer, owner, Bytes32([0x33; 32]));
    let payer_signer = CountingSigner::new(payer_keys.clone());
    let owner_signer = CountingSigner::new(owner_keys.clone());

    let through_signers = sign_and_serialize_tx_msg_with(&msg, &[&owner_signer, &payer_signer])
        .await
        .unwrap();
    let through_keys =
        sign_and_serialize_tx_msg_with_keys(&msg, &[&payer_keys, &owner_keys]).unwrap();
    assert_eq!(through_signers, through_keys);
    assert_eq!(payer_signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(owner_signer.calls.load(Ordering::SeqCst), 1);

    // In-memory keys are signers too.
    let direct = sign_and_serialize_tx_msg_with(&msg, &[&payer_keys, &owner_keys])
        .await
        .unwrap();
    assert_eq!(direct, through_keys);
}

// The same account pays the gas and owns the tokens: two envelope slots, one signature.
#[tokio::test]
async fn a_double_slot_signer_is_asked_once() {
    let (payer_keys, _) = keys();
    let payer = address_of(&payer_keys);
    let msg = gas_payer_transfer(payer, payer, Bytes32([0x33; 32]));
    let signer = CountingSigner::new(payer_keys);

    let signed = sign_tx_msg_with(&msg, &[&signer]).await.unwrap();
    assert_eq!(signer.calls.load(Ordering::SeqCst), 1);
    assert_eq!(signed.witnesses.len(), 2);
    assert_eq!(signed.witnesses[0].signature, signed.witnesses[1].signature);
}

#[test]
fn signer_sets_that_do_not_match_the_message_are_refused() {
    let (payer_keys, owner_keys) = keys();
    let stranger = PhantasmaKeys::generate();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let msg = gas_payer_transfer(payer, owner, Bytes32([0x33; 32]));

    let missing = builder_message(sign_tx_msg_with_keys(&msg, &[&payer_keys]).unwrap_err());
    assert!(
        missing.contains(&format!("no signer for witness {owner}")),
        "{missing}"
    );
    let stray = builder_message(
        sign_tx_msg_with_keys(&msg, &[&payer_keys, &owner_keys, &stranger]).unwrap_err(),
    );
    assert!(
        stray.contains("is not a witness of this transaction"),
        "{stray}"
    );

    let native = native_transfer(payer, owner, 1);
    let wrong = builder_message(sign_tx_msg_with_keys(&native, &[&owner_keys]).unwrap_err());
    assert!(wrong.contains("no signer for witness"), "{wrong}");

    // A raw Phantasma transaction carries no witnesses at all.
    let raw = TxMsg {
        tx_type: TxType::PhantasmaRaw,
        expiry: 1,
        max_gas: 1,
        max_data: 0,
        gas_from: payer,
        payload: SmallString::default(),
        msg: TxPayload::PhantasmaRaw(TxMsgPhantasmaRaw {
            transaction: vec![1],
        }),
    };
    let with_keys = builder_message(sign_tx_msg_with_keys(&raw, &[&payer_keys]).unwrap_err());
    assert!(with_keys.contains("carry no witnesses"), "{with_keys}");
    assert!(sign_tx_msg_with_keys(&raw, &[])
        .unwrap()
        .witnesses
        .is_empty());
}

#[test]
fn witness_array_transactions_keep_the_caller_order() {
    let (payer_keys, owner_keys) = keys();
    let stranger = PhantasmaKeys::generate();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));

    let signed =
        sign_tx_msg_with_keys(&call_tx(payer), &[&owner_keys, &payer_keys, &stranger]).unwrap();
    assert_eq!(
        signed
            .witnesses
            .iter()
            .map(|witness| witness.address)
            .collect::<Vec<_>>(),
        vec![owner, payer, address_of(&stranger)]
    );
    let decoded: SignedTxMsg = deserialize(serialize(&signed).unwrap()).unwrap();
    assert_eq!(decoded.witnesses.len(), 3);

    // The node rejects a call the gas payer did not sign; the SDK refuses to build one.
    let without_payer =
        builder_message(sign_tx_msg_with_keys(&call_tx(payer), &[&owner_keys]).unwrap_err());
    assert!(without_payer.contains("gas payer"), "{without_payer}");
    let nobody = builder_message(sign_tx_msg_with_keys(&call_tx(payer), &[]).unwrap_err());
    assert!(nobody.contains("at least one witness"), "{nobody}");
}

// A zero offer can never be admitted, so it marks a message that was built but not planned;
// signing it would only produce a rejection.
#[test]
fn an_unplanned_message_is_refused() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let msg = native_transfer(payer, owner, 0);
    let message = builder_message(sign_tx_msg(&msg, &payer_keys).unwrap_err());
    assert!(message.contains("no gas offer"), "{message}");
}

// The single-witness path keeps its historical behaviour and is the same signature the signer
// interface produces.
#[tokio::test]
async fn the_single_witness_path_still_signs_the_historical_way() {
    let (payer_keys, owner_keys) = keys();
    let (payer, owner) = (address_of(&payer_keys), address_of(&owner_keys));
    let native = native_transfer(payer, owner, 1);
    let via_keys = sign_and_serialize_tx_msg(&native, &payer_keys).unwrap();
    let via_signer = sign_and_serialize_tx_msg_with(&native, &[&payer_keys])
        .await
        .unwrap();
    assert_eq!(via_keys, via_signer);
}
