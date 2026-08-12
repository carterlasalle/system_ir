# serde
> https://github.com/serde-rs/serde | Rust | rust cli (lib) | ~43k LOC

## components
- `Serialize` — core trait in serde_core/src/ser/mod.rs:234; implemented by types that can be serialized
- `Deserialize` — core trait in serde_core/src/de/mod.rs:554; implemented by types that can be deserialized
- `Serializer` — trait in serde_core/src/ser/mod.rs:355; data-format side of serialization
- `Deserializer` — trait in serde_core/src/de/mod.rs:945; data-format side of deserialization
- `Visitor` — trait in serde_core/src/de/mod.rs:1317; receives deserialized values from a Deserializer
- `SerializeSeq` — compound serializer trait (ser/mod.rs:1518/1624/1811/1907: `SerializeTuple`/`SerializeMap`/`SerializeStruct`) returned by `serialize_seq` etc.
- `SeqAccess` — accessor trait the Visitor consumes (de/mod.rs:1749/1837/2035/2088: `MapAccess`/`EnumAccess`/`VariantAccess`)
- `serde_derive` — proc-macro crate; `#[proc_macro_derive(Serialize, attributes(serde))]` and Deserialize at serde_derive/src/lib.rs:113/121
- `serde_core` — no_std crate mirroring serde_core/src, re-exported by `serde` (workspace member)
- `serde_derive_internals` — internal AST/attr analysis consumed by serde_derive (workspace member)
- `IgnoredAny` — de/ignored_any.rs; deserializer that accepts and discards any value
- `de::value` — module (serde_core/src/de/value.rs) with `IntoDeserializer` impls and primitive-value deserializers

## entrypoints
- `#[derive(Serialize)]` — serde_derive/src/lib.rs:113; generates the `Serialize` impl from a struct/enum
- `#[derive(Deserialize)]` — serde_derive/src/lib.rs:121; generates the `Deserialize` impl
- `Serialize::serialize` — entry method called by format code (ser/mod.rs:234 trait)
- `Deserialize::deserialize` — entry method receiving a `Deserializer` (de/mod.rs:554 trait)
- `serde_json::to_string` — example format usage in README.md:63 demonstrating `Serialize`
- `serde_json::from_str` — README.md:69; `Deserialize` usage with `Point`

## flows
- `Serialize.serialize` — struct serialization chain (ser/mod.rs:1220): Serialize.serialize -> Serializer.serialize_struct -> SerializeStruct.serialize_field -> end
- `Deserializer.deserialize_struct` — deserialization chain (de/mod.rs:1152): Deserializer.deserialize_struct -> Visitor.visit_seq -> SeqAccess.next_element
- `Serialize` — derive expansion pipeline: #[derive(Serialize)] -> serde_derive_internals AST -> generated serialize impl
- `serde_json::to_string` — README roundtrip flow (README.md:63-69): serde_json::to_string -> <Point as Serialize>::serialize -> JSON string
- `Deserializer.deserialize_enum` — enum deserialization (de/mod.rs:1163): Deserializer.deserialize_enum -> EnumAccess.variant_seed -> VariantAccess

## ownership
- `serde/src/private` — internal module gating crate-private hooks used by serde_derive_internals (private/mod.rs, private/de.rs, private/ser.rs)
- `serde_derive_internals` — owns the parsed AST and serde-attribute analysis for derive generation
- `serde_core/src` — the shared no_std core implementation, re-exported through serde/src/lib.rs
- `Content` — owns buffered token data for buffered deserialization (serde_core/src/private/content.rs)
- `serde_core/src/format.rs` — format-related internal helpers shared by ser and de

## contracts
- `serialize_struct` — serializer contract: serialize_struct(name, len) (struct name + field count) (ser/mod.rs:1220)
- `deserialize_struct` — deserializer contract: deserialize_struct(name, fields, visitor) (field-name list) (de/mod.rs:1152)
- `#[serde(tag = "t", content = "c")]` — adjacently-tagged enum attribute exercised in test_suite/tests/test_enum_adjacently_tagged.rs
- `Error::custom` — format error escape hatch on Serializer/Deserializer Error associated types
- `serde_test::Token` — token-stream contract (serde_test crate) used to assert serialized/de tokens in test_suite
- `serde_json::to_string` — JSON wire contract shown in README (README.md:63)

## tests
- `test_suite/tests/test_ser.rs` — Serialize impls via `assert_ser_tokens`
- `test_suite/tests/test_de.rs` — Deserialize impls via `assert_de_tokens`/`Configure`
- `test_suite/tests/test_roundtrip.rs` — round-trip checks with `assert_tokens`
- `test_suite/tests/test_annotations.rs` — serde attribute coverage (rename, skip, default, ...)
- `test_suite/tests/test_enum_adjacently_tagged.rs` — tagged-enum derive behavior
- `test_suite/tests/regression/issue2565.rs` — regression fixtures per reported issue
- `test_suite/tests/ui` — compile-fail UI tests for derive diagnostics
