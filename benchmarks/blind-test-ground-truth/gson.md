# gson
> https://github.com/google/gson | Java | java json library | ~50k LOC

## architecture
- gson — the core module (gson/)
- Gson.java — the main facade: Gson, toJson, fromJson (gson/src/main/java/com/google/gson/Gson.java)
- GsonBuilder.java — the builder: GsonBuilder (GsonBuilder.java)
- JsonElement.java — the element model: JsonElement, JsonObject, JsonArray, JsonPrimitive, JsonNull (JsonElement.java)
- JsonParser.java — the parser facade: JsonParser (JsonParser.java)
- JsonStreamParser.java — the streaming parser (JsonStreamParser.java)
- stream — the token stream: JsonReader, JsonWriter, JsonToken, JsonScope (stream/)
- internal — the internals: Excluder, ConstructorConstructor, LinkedTreeMap (internal/)
- internal/reflect — reflection-based serialization (internal/reflect/)
- internal/bind — the type adapters: TypeAdapterRuntimeTypeWrapper, CollectionTypeAdapterFactory (internal/bind/)
- annotations — the annotation package: SerializedName, Expose, Since, Until (annotations/)
- TypeAdapter.java — the adapter base: TypeAdapter (TypeAdapter.java)
- TypeToken.java — the type token (TypeToken.java)
- FieldNamingPolicy.java — the naming policies (FieldNamingPolicy.java)
- LongSerializationPolicy.java — the long policy (LongSerializationPolicy.java)
- ExclusionStrategy.java — the exclusion strategy (ExclusionStrategy.java)
- gson/src/test — the test suite (gson/src/test/)

## entrypoints
- new Gson() — the default instance entry
- new GsonBuilder().create() — the builder entry
- gson.toJson(obj) — serialize to JSON string
- gson.toJson(obj, type) — typed serialization
- gson.toJsonTree(obj) — serialize to element tree
- gson.toJson(obj, writer) — serialize to a writer
- gson.fromJson(json, class) — deserialize to a class
- gson.fromJson(json, type) — typed deserialization
- gson.fromJson(json, TypeToken) — token deserialization
- gson.fromJson(reader, type) — reader deserialization
- gson.fromJson(element, type) — element deserialization
- JsonParser.parseString(json) — parse to an element
- JsonStreamParser — streaming parse entry
- TypeAdapter.write — adapter write entry
- TypeAdapter.read — adapter read entry
- GsonBuilder.serializeNulls — null serialization
- GsonBuilder.setPrettyPrinting — pretty printing
- GsonBuilder.registerTypeAdapter — custom adapter registration
- GsonBuilder.excludeFieldsWithoutExposeAnnotation — expose filtering
- GsonBuilder.setFieldNamingPolicy — naming policy

## behavior
- toJson(obj) -> type adapter -> JsonWriter -> string (Gson.java)
- fromJson(json, class) -> JsonReader -> type adapter -> object (Gson.java)
- GsonBuilder.create() -> Gson with factories (GsonBuilder.java)
- JsonParser.parseString -> JsonReader -> element tree (JsonParser.java)
- TypeAdapter.write -> JsonWriter output (TypeAdapter.java)
- JsonWriter.value -> token write (stream/JsonWriter.java)
- JsonReader.nextName -> token read (stream/JsonReader.java)
- excludeFieldsWithoutExpose -> Excluder -> skip fields (internal/Excluder.java)
- registerTypeAdapter -> factory -> adapter resolution (GsonBuilder.java)

## state_authority
- Gson — the configuration state: factories, serialization options (Gson.java)
- GsonBuilder — the builder state (GsonBuilder.java)
- JsonElement — the element tree state (JsonElement.java)
- JsonObject — the object node state (JsonObject.java)
- JsonArray — the array node state (JsonArray.java)
- TypeAdapter — the adapter state (TypeAdapter.java)
- TypeToken — the type state (TypeToken.java)
- JsonReader — the reader state (stream/JsonReader.java)
- JsonWriter — the writer state (stream/JsonWriter.java)
- Excluder — the exclusion state (internal/Excluder.java)

## contracts
- gson.toJson(obj) — toJson contract
- gson.fromJson(json, Type.class) — fromJson contract
- new GsonBuilder().create() — builder contract
- .serializeNulls() — null serialization contract
- .setPrettyPrinting() — pretty print contract
- .registerTypeAdapter(Type, adapter) — adapter contract
- .excludeFieldsWithoutExposeAnnotation() — expose contract
- .setFieldNamingPolicy(FieldNamingPolicy.LOWER_CASE_WITH_UNDERSCORES) — naming contract
- .setVersion(1.0) — versioning contract
- .disableHtmlEscaping() — html escaping contract
- @SerializedName("name") — serialized name contract
- @Expose — expose annotation contract
- @Since(1.0) — version annotation contract
- {"key":"value"} — JSON object contract
- [1,2,3] — JSON array contract
- null — null contract
- JsonParser.parseString(json) — parse contract
- gson.toJsonTree(obj) — tree contract

## landmarks
- Gson — the facade class (Gson.java)
- GsonBuilder — the builder (GsonBuilder.java)
- JsonElement — the element base (JsonElement.java)
- JsonObject — the object node (JsonObject.java)
- JsonArray — the array node (JsonArray.java)
- JsonPrimitive — the primitive node (JsonPrimitive.java)
- JsonNull — the null node (JsonNull.java)
- JsonParser — the parser (JsonParser.java)
- JsonReader — the reader (stream/JsonReader.java)
- JsonWriter — the writer (stream/JsonWriter.java)
- TypeAdapter — the adapter base (TypeAdapter.java)
- TypeToken — the type token (TypeToken.java)
- FieldNamingPolicy — the naming policy (FieldNamingPolicy.java)
- Excluder — the excluder (internal/Excluder.java)
- LinkedTreeMap — the map implementation (internal/LinkedTreeMap.java)

## tests
- gson/src/test/java/com/google/gson/GsonTest.java — core tests
- gson/src/test/java/com/google/gson/GsonBuilderTest.java — builder tests
- gson/src/test/java/com/google/gson/JsonParserTest.java — parser tests
- gson/src/test/java/com/google/gson/JsonObjectTest.java — object tests
- gson/src/test/java/com/google/gson/JsonArrayTest.java — array tests
- gson/src/test/java/com/google/gson/stream/JsonReaderTest.java — reader tests
- gson/src/test/java/com/google/gson/stream/JsonWriterTest.java — writer tests
- gson/src/test/java/com/google/gson/functional/ — functional tests
- gson/src/test/java/com/google/gson/internal/bind/ — bind tests
