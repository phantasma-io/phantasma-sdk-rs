use num_bigint::BigInt;
use phantasma_sdk::{
    build_and_serialize_token_schemas, build_mint_phantasma_non_fungible_single_tx,
    build_mint_phantasma_non_fungible_tx, build_token_schemas_from_fields, default_market_config,
    deserialize, parse_token_schemas_json, serialize, serialize_token_schemas,
    serialize_token_schemas_hex, token_schemas_from_json, vm_type_from_string, vm_type_name,
    BurnFungibleArgs, BurnNonFungibleArgs, Bytes32, ChainConfig, CreateMintedTokenSeriesArgs,
    CreateSeriesFeeOptions, CreateTokenSeriesArgs, GasConfig, IntX, MarketConfig,
    MarketConfigFlags, MarketSellTokenByIdArgs, MintFungibleArgs, MintNFTFeeOptions,
    MintPhantasmaNonFungibleArgs, ModuleId, PhantasmaNFTMintInfo, PhantasmaNFTMintResult,
    SeriesInfo, SmallString, TokenContractMethod, TokenListing, TokenSchemaField, TokensConfig,
    TokensConfigFlags, TransferFungibleArgs, TransferNonFungibleArgs, TxMsgCall, TxPayload, TxType,
    UpdateSeriesMetadataArgs, UpdateTokenMetadataArgs, VMDynamicStruct, VMDynamicVariable,
    VMNamedDynamicVariable, VMStructFlags, VMStructSchema, VMType,
};

fn repeated_bytes32(value: u8) -> Bytes32 {
    Bytes32([value; 32])
}

fn vector_series_info() -> SeriesInfo {
    SeriesInfo {
        max_mint: 3,
        max_supply: 9,
        owner: repeated_bytes32(0x33),
        metadata: vec![0xaa, 0xbb],
        rom: VMStructSchema::default(),
        ram: VMStructSchema::default(),
    }
}

fn assert_blob_vector<T>(expected_hex: &str, value: T)
where
    T: phantasma_sdk::CarbonSerializable + PartialEq + std::fmt::Debug,
{
    let raw = hex::decode(expected_hex).unwrap();
    assert_eq!(hex::encode_upper(serialize(&value).unwrap()), expected_hex);
    let decoded: T = deserialize(raw).unwrap();
    assert_eq!(decoded, value);
    assert_eq!(
        hex::encode_upper(serialize(&decoded).unwrap()),
        expected_hex
    );
}

#[test]
fn token_call_args_vectors_match_go_reference() {
    let one = repeated_bytes32(0x11);
    let two = repeated_bytes32(0x22);
    let four = repeated_bytes32(0x44);

    assert_blob_vector(
        "010000000000000011111111111111111111111111111111111111111111111111111111111111110800E1F50500000000",
        MintFungibleArgs {
            token_id: 1,
            to: one,
            amount: IntX::from(100_000_000i64),
        },
    );
    assert_blob_vector(
        concat!(
            "1111111111111111111111111111111111111111111111111111111111111111",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "0100000000000000",
            "0800E1F50500000000"
        ),
        TransferFungibleArgs {
            to: one,
            from_address: two,
            token_id: 1,
            amount: IntX::from(100_000_000i64),
        },
    );
    assert_blob_vector(
        concat!(
            "1111111111111111111111111111111111111111111111111111111111111111",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "0100000000000000",
            "0200000007000000000000000800000000000000"
        ),
        TransferNonFungibleArgs {
            to: one,
            from_address: two,
            token_id: 1,
            instance_ids: vec![7, 8],
        },
    );
    assert_blob_vector(
        "010000000000000022222222222222222222222222222222222222222222222222222222222222220800E1F50500000000",
        BurnFungibleArgs {
            token_id: 1,
            from_address: two,
            amount: IntX::from(100_000_000i64),
        },
    );
    assert_blob_vector(
        concat!(
            "0100000000000000",
            "2222222222222222222222222222222222222222222222222222222222222222",
            "0200000007000000000000000800000000000000"
        ),
        BurnNonFungibleArgs {
            token_id: 1,
            from_address: two,
            instance_ids: vec![7, 8],
        },
    );
    assert_blob_vector(
        concat!(
            "0900000000000000",
            "0300000009000000",
            "3333333333333333333333333333333333333333333333333333333333333333",
            "02000000AABB",
            "0000000000",
            "0000000000"
        ),
        CreateTokenSeriesArgs {
            token_id: 9,
            info: vector_series_info(),
        },
    );
    assert_blob_vector(
        concat!(
            "0900000000000000",
            "0300000009000000",
            "3333333333333333333333333333333333333333333333333333333333333333",
            "02000000AABB",
            "0000000000",
            "0000000000",
            "4444444444444444444444444444444444444444444444444444444444444444",
            "0200000002000000010200000000",
            "010000000100000003"
        ),
        CreateMintedTokenSeriesArgs {
            token_id: 9,
            info: vector_series_info(),
            address: four,
            roms: vec![vec![1, 2], vec![]],
            rams: vec![vec![3]],
        },
    );
    assert_blob_vector(
        "090000000000000001000000016E16616C70686100",
        UpdateTokenMetadataArgs {
            token_id: 9,
            metadata: VMDynamicStruct::new(vec![VMNamedDynamicVariable::new(
                "n",
                VMDynamicVariable::string("alpha"),
            )]),
        },
    );
    assert_blob_vector(
        "09000000000000000700000004000000DEADBEEF",
        UpdateSeriesMetadataArgs {
            token_id: 9,
            series_id: 7,
            metadata: hex::decode("DEADBEEF").unwrap(),
        },
    );
    assert_blob_vector(
        concat!(
            "0900000000000000",
            "4444444444444444444444444444444444444444444444444444444444444444",
            "02000000",
            "082A0000000000000002000000AABB01000000CC",
            "082B000000000000000000000002000000DDEE"
        ),
        MintPhantasmaNonFungibleArgs {
            token_id: 9,
            address: four,
            tokens: vec![
                PhantasmaNFTMintInfo {
                    phantasma_series_id: IntX::from(42i64),
                    rom: vec![0xaa, 0xbb],
                    ram: vec![0xcc],
                },
                PhantasmaNFTMintInfo {
                    phantasma_series_id: IntX::from(43i64),
                    rom: vec![],
                    ram: vec![0xdd, 0xee],
                },
            ],
        },
    );
    assert_blob_vector(
        "55555555555555555555555555555555555555555555555555555555555555557B00000000000000",
        PhantasmaNFTMintResult {
            phantasma_nft_id: repeated_bytes32(0x55),
            carbon_instance_id: 123,
        },
    );
}

#[test]
fn chain_gas_token_and_market_config_wire_formats_match_python() {
    let chain = ChainConfig {
        version: 1,
        reserved1: 2,
        reserved2: 3,
        reserved3: 4,
        allowed_tx_types: 0xAABBCCDD,
        expiry_window: 60_000,
        block_rate_target: 1_000,
    };
    assert_eq!(
        hex::encode(serialize(&chain).unwrap()),
        "01020304ddccbbaa60ea0000e8030000"
    );
    let decoded: ChainConfig = deserialize(serialize(&chain).unwrap()).unwrap();
    assert_eq!(decoded, chain);

    // version=0: the 113-byte image is exactly the version-0 layout; version >= 1 configs
    // append the gas-model-v2 tail on the wire (see tests/gas_config_fee.rs).
    let gas = GasConfig {
        version: 0,
        max_name_length: 2,
        max_token_symbol_length: 3,
        fee_shift: 4,
        max_structure_size: 5,
        fee_multiplier: 6,
        gas_token_id: 7,
        data_token_id: 8,
        minimum_gas_offer: 9,
        data_escrow_per_row: 10,
        gas_fee_transfer: 11,
        gas_fee_query: 12,
        gas_fee_create_token_base: 13,
        gas_fee_create_token_symbol: 14,
        gas_fee_create_token_series: 15,
        gas_fee_per_byte: 16,
        gas_fee_register_name: 17,
        gas_burn_ratio_mul: 18,
        gas_burn_ratio_shift: 19,
        ..GasConfig::default()
    };
    let raw = serialize(&gas).unwrap();
    assert_eq!(raw.len(), 113);
    assert_eq!(&hex::encode(&raw[..8]), "0002030405000000");
    assert_eq!(&hex::encode(&raw[raw.len() - 9..]), "120000000000000013");
    let decoded: GasConfig = deserialize(raw).unwrap();
    assert_eq!(decoded, gas);

    let tokens_config = TokensConfig {
        flags: TokensConfigFlags::REQUIRE_METADATA
            | TokensConfigFlags::ALLOW_EXPLICIT_NFT_META_ID_MINT,
    };
    assert_eq!(hex::encode(serialize(&tokens_config).unwrap()), "11");

    let default_market = default_market_config();
    assert_eq!(
        default_market.flags,
        MarketConfigFlags::PRICE_REQUIRED | MarketConfigFlags::ENFORCE_ROYALTIES
    );
    let decoded: MarketConfig = deserialize(serialize(&default_market).unwrap()).unwrap();
    assert_eq!(decoded, default_market);

    let seller = Bytes32(std::array::from_fn(|index| index as u8));
    let listing = TokenListing {
        listing_type: phantasma_sdk::ListingType::FixedPrice,
        seller,
        quote_token_id: 2,
        price: IntX::from(123i64),
        start_date: 10,
        end_date: 20,
    };
    assert_eq!(
        hex::encode(serialize(&listing).unwrap()),
        concat!(
            "00",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "0200000000000000",
            "087b00000000000000",
            "0a00000000000000",
            "1400000000000000"
        )
    );
}

#[test]
fn market_by_id_args_and_schema_json_helpers_match_python() {
    let market_args = MarketSellTokenByIdArgs {
        from_address: repeated_bytes32(0x22),
        symbol: SmallString::new("ART").unwrap(),
        instance_id: VMDynamicVariable::int64(7),
        quote_symbol: SmallString::new("SOUL").unwrap(),
        price: IntX::from(12i64),
        end_date: 13,
    };
    let decoded: MarketSellTokenByIdArgs = deserialize(serialize(&market_args).unwrap()).unwrap();
    assert_eq!(decoded, market_args);

    let schema_json = r#"
        {
          "seriesMetadata": [{"name": "collection", "type": "String"}],
          "rom": [
            {"name": "name", "type": "String"},
            {"name": "description", "type": "String"},
            {"name": "imageURL", "type": "String"},
            {"name": "infoURL", "type": "String"},
            {"name": "royalties", "type": "Int32"},
            {"name": "rarity", "type": "Int32"}
          ],
          "ram": []
        }
        "#;
    let json_shape = parse_token_schemas_json(schema_json).unwrap();
    assert_eq!(json_shape.series_metadata[0].name, "collection");
    assert_eq!(json_shape.series_metadata[0].vm_type, VMType::String);
    assert_eq!(json_shape.rom.last().unwrap().name, "rarity");
    assert_eq!(json_shape.ram.len(), 0);

    let parsed = token_schemas_from_json(schema_json).unwrap();
    assert_eq!(
        parsed.series_metadata.fields.last().unwrap().name.as_str(),
        "collection"
    );
    assert_eq!(
        parsed.rom.fields.last().unwrap().schema.vm_type,
        VMType::Int32
    );
    assert_eq!(parsed.ram.flags, VMStructFlags::DYNAMIC_EXTRAS);
    assert_eq!(
        serialize_token_schemas_hex(&parsed).unwrap(),
        hex::encode_upper(serialize_token_schemas(&parsed).unwrap())
    );
    assert_eq!(
        build_and_serialize_token_schemas(None).unwrap(),
        serialize_token_schemas(&phantasma_sdk::prepare_standard_token_schemas(false)).unwrap()
    );
    assert_eq!(
        build_token_schemas_from_fields(
            &[TokenSchemaField::new("collection", VMType::String)],
            &[
                TokenSchemaField::new("name", VMType::String),
                TokenSchemaField::new("description", VMType::String),
                TokenSchemaField::new("imageURL", VMType::String),
                TokenSchemaField::new("infoURL", VMType::String),
                TokenSchemaField::new("royalties", VMType::Int32),
                TokenSchemaField::new("rarity", VMType::Int32),
            ],
            &[],
        )
        .unwrap(),
        parsed
    );
    assert_eq!(
        vm_type_from_string("ArrayBytes32").unwrap(),
        VMType::ArrayBytes32
    );
    assert_eq!(vm_type_name(VMType::ArrayBytes32).unwrap(), "Array_Bytes32");
    assert!(token_schemas_from_json(r#"{"seriesMetadata": [], "rom": []}"#).is_err());
    assert!(token_schemas_from_json(
        r#"{"seriesMetadata": [], "rom": [], "ram": [{"name": "bad", "type": "Nope"}]}"#
    )
    .is_err());
}

#[test]
fn phantasma_nft_tx_helper_uses_call_payload_and_fee_defaults() {
    let sender = repeated_bytes32(0x11);
    let receiver = repeated_bytes32(0x22);
    let tx = build_mint_phantasma_non_fungible_single_tx(
        42,
        BigInt::from(777u32),
        sender,
        receiver,
        vec![0xaa, 0xbb],
        vec![],
        Some(MintNFTFeeOptions::default()),
        123,
        999,
    )
    .unwrap();
    assert_eq!(tx.tx_type, TxType::Call);
    assert_eq!(tx.expiry, 999);
    assert_eq!(tx.max_data, 123);
    assert_eq!(tx.gas_from, sender);
    let TxPayload::Call(TxMsgCall {
        module_id,
        method_id,
        args,
        ..
    }) = tx.msg
    else {
        panic!("expected call payload");
    };
    assert_eq!(module_id, ModuleId::Token as u32);
    assert_eq!(
        method_id,
        TokenContractMethod::MintPhantasmaNonFungible as u32
    );
    let decoded: MintPhantasmaNonFungibleArgs = deserialize(args).unwrap();
    assert_eq!(decoded.token_id, 42);
    assert_eq!(decoded.address, receiver);
    assert!(CreateSeriesFeeOptions::default().calculate_max_gas() > 0);
    assert!(MintNFTFeeOptions::default().calculate_max_gas() > 0);
}

#[test]
fn fee_options_scale_only_count_sensitive_mint_fees() {
    let mint_fees = MintNFTFeeOptions {
        gas_fee_base: 10,
        fee_multiplier: 1_000,
    };
    assert_eq!(mint_fees.calculate_max_gas(), 10_000);
    assert_eq!(mint_fees.calculate_max_gas_for_count(3).unwrap(), 30_000);
    assert!(mint_fees.calculate_max_gas_for_count(0).is_err());

    let series_fees = CreateSeriesFeeOptions {
        gas_fee_base: 10,
        fee_multiplier: 30,
        gas_fee_create_series_base: 20,
    };
    assert_eq!(series_fees.calculate_max_gas(), 900);

    let sender = repeated_bytes32(0x11);
    let receiver = repeated_bytes32(0x22);
    let tx = build_mint_phantasma_non_fungible_tx(
        42,
        sender,
        receiver,
        vec![
            PhantasmaNFTMintInfo {
                phantasma_series_id: IntX(BigInt::from(1u32)),
                rom: vec![0x01],
                ram: vec![],
            },
            PhantasmaNFTMintInfo {
                phantasma_series_id: IntX(BigInt::from(2u32)),
                rom: vec![0x02],
                ram: vec![],
            },
            PhantasmaNFTMintInfo {
                phantasma_series_id: IntX(BigInt::from(3u32)),
                rom: vec![0x03],
                ram: vec![],
            },
        ],
        Some(mint_fees),
        123,
        999,
    )
    .unwrap();
    assert_eq!(tx.max_gas, 30_000);
    assert!(build_mint_phantasma_non_fungible_tx(
        42,
        sender,
        receiver,
        vec![],
        Some(MintNFTFeeOptions::default()),
        123,
        999,
    )
    .is_err());
}
