# pydantic
> https://github.com/pydantic/pydantic | Python | python service (lib) | ~173k LOC

## components
- BaseModel — the model base class in pydantic/main.py (line 133), core of the library
- FieldInfo — field metadata class in pydantic/fields.py (line 107), holds default/alias/constraints
- Field — function building FieldInfo in pydantic/fields.py (line 1175)
- ConfigDict — TypedDict of model configuration options in pydantic/config.py (line 34)
- TypeAdapter — validation/serialization for non-model types in pydantic/type_adapter.py (line 70)
- dataclass — pydantic dataclass decorator in pydantic/dataclasses.py (line 66)
- GenerateJsonSchema — JSON-schema generator class in pydantic/json_schema.py (line 223)
- ValidationError — re-exported from pydantic_core in pydantic/__init__.py (line 66)
- PydanticUserError — user-misuse error class in pydantic/errors.py (line 100)
- AfterValidator — field "after" validator marker in pydantic/functional_validators.py (line 30)
- BeforeValidator — field "before" validator marker in pydantic/functional_validators.py (line 91)
- field_validator — decorator adding field validators (pydantic/functional_validators.py line 410)
- model_validator — decorator adding model-level validators (pydantic/functional_validators.py line 677)
- computed_field — decorator for computed properties (pydantic/fields.py line 1713)
- create_model — runtime model factory (pydantic/main.py line 1799)

## entrypoints
- model_validate — classmethod validating arbitrary objects into the model (pydantic/main.py line 748)
- model_validate_json — validates a JSON string (pydantic/main.py line 802)
- model_validate_strings — validates from string inputs (pydantic/main.py line 851)
- model_dump — serializes the model to a dict (pydantic/main.py line 469)
- model_dump_json — serializes the model to JSON (pydantic/main.py line 535)
- model_construct — builds an instance without validation (pydantic/main.py line 331)
- model_json_schema — generates the model's JSON schema (pydantic/main.py line 604)
- TypeAdapter.validate_python — entry for arbitrary-type validation (pydantic/type_adapter.py line 396)
- TypeAdapter.dump_json — entry for arbitrary-type JSON serialization (pydantic/type_adapter.py line 633)

## flows
- __get_pydantic_core_schema__ — hook producing the core schema used by pydantic-core (pydantic/main.py line 889)
- generate — GenerateJsonSchema.generate turns a core schema into a JSON schema (pydantic/json_schema.py line 399)
- model_rebuild — re-resolves forward references and rebuilds the schema (pydantic/main.py line 675)
- TypeAdapter.json_schema — JSON schema for non-model types (pydantic/type_adapter.py line 704)
- model_post_init — post-init hook called by __init__ and model_construct (pydantic/main.py line 669)
- __pydantic_init_subclass__ — subclass hook invoked by ModelMetaclass (pydantic/main.py line 935)

## ownership
- model_config — per-model configuration dict (pydantic/main.py line 97)
- __pydantic_fields__ — compiled dict of FieldInfo per model class (pydantic/main.py line 109)
- __pydantic_fields_set__ — set of fields explicitly set on an instance (pydantic/main.py line 321)
- model_extra — property exposing extra fields stored in __pydantic_extra__ (pydantic/main.py line 312)
- __pydantic_private__ — storage for PrivateAttr attributes (pydantic/main.py line 114)
- ConfigDict.validate_assignment — config key controlling assignment validation (pydantic/config.py line 264)

## contracts
- model_config = ConfigDict(validate_assignment=True) — config contract in tests/test_main.py (line 343)
- strict=True — strict-mode parameter tested in test_model_validate_strict (tests/test_main.py line 2794)
- mode='json' — serialization modes of model_dump (pydantic/main.py line 471)
- ConfigDict(defer_build=True) — deferred-schema-build contract in tests/test_type_adapter.py (line 74)
- indent — pretty-print parameter of model_dump_json (pydantic/main.py line 538)
- mode='validation' — JSON-schema generation mode (pydantic/json_schema.py line 399)

## tests
- tests/test_main.py — BaseModel behavior (test_model_validate_strict, test_validating_assignment_pass)
- tests/test_type_adapter.py — TypeAdapter contracts (test_validate_strings, test_global_namespace_variables)
- tests/test_json_schema.py — JSON schema generation coverage
- tests/test_validators.py — field/model validator behavior
- tests/test_model_validator.py — model_validator modes
- tests/test_dataclasses.py — pydantic dataclass behavior
- tests/test_create_model.py — create_model runtime construction
- tests/test_errors.py — error classes and codes
