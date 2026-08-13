# retrofit
> https://github.com/square/retrofit | Java | java http client framework | ~40k LOC

## architecture
- retrofit — the core module: Retrofit, Call, converters (retrofit/)
- Retrofit.java — the framework core: Retrofit, Builder, create (retrofit/src/main/java/retrofit2/Retrofit.java)
- ServiceMethod.java — service method analysis (ServiceMethod.java)
- HttpServiceMethod.java — the http method invocation (HttpServiceMethod.java)
- RequestFactory.java — request building (RequestFactory.java)
- RequestBuilder.java — request construction (RequestBuilder.java)
- OkHttpCall.java — the OkHttp call implementation (OkHttpCall.java)
- ParameterHandler.java — parameter handling (ParameterHandler.java)
- Call.java — the call interface (Call.java)
- Callback.java — the async callback interface (Callback.java)
- CallAdapter.java — the call adapter interface (CallAdapter.java)
- Converter.java — the converter interface (Converter.java)
- BuiltInConverters.java — the built-in converters (BuiltInConverters.java)
- DefaultCallAdapterFactory.java — the default adapter (DefaultCallAdapterFactory.java)
- Platform.java — platform detection (Platform.java)
- retrofit-converters — the converter modules: gson, jackson, moshi, protobuf, scalars, java8, wire, jaxb (retrofit-converters/)
- retrofit-adapters — the adapter modules: rxjava, java8 (retrofit-adapters/)

## entrypoints
- Retrofit.Builder — the builder entry
- new Retrofit.Builder() — builder construction
- Builder.baseUrl — set the base URL
- Builder.addConverterFactory — add a converter
- Builder.addCallAdapterFactory — add a call adapter
- Builder.client — set the http client
- Builder.build — build the Retrofit instance
- retrofit.create(Service.class) — service proxy creation
- Call.execute — synchronous execution
- Call.enqueue — asynchronous execution
- Call.clone — clone a call
- Call.cancel — cancel a call
- Call.request — the underlying request
- retrofit2.Callback.onResponse — success callback
- retrofit2.Callback.onFailure — failure callback
- Converter.Factory — the converter factory interface
- CallAdapter.Factory — the call adapter factory interface

## behavior
- create(service) -> ServiceMethod -> proxy -> invoke (Retrofit.java)
- Call.execute -> OkHttpCall -> response (OkHttpCall.java)
- Call.enqueue -> Callback -> async response (OkHttpCall.java)
- RequestFactory.build -> RequestBuilder -> request (RequestFactory.java)
- ServiceMethod.adapt -> CallAdapter -> call (HttpServiceMethod.java)
- Converter -> body conversion (Converter.java)
- ParameterHandler.apply -> request parameters (ParameterHandler.java)
- Builder.build -> Retrofit instance (Retrofit.java)

## state_authority
- Retrofit — the framework state: base URL, converter factories, adapter factories (Retrofit.java)
- Builder — the builder state (Retrofit.java)
- ServiceMethod — the method metadata state (ServiceMethod.java)
- OkHttpCall — the call state: request, response, cancellation (OkHttpCall.java)
- Callback — the async callback state (Callback.java)
- Converter — the converter state (Converter.java)
- CallAdapter — the adapter state (CallAdapter.java)
- RequestBuilder — the request construction state (RequestBuilder.java)
- HttpUrl — the base URL state (via OkHttp)

## contracts
- baseUrl("https://api.example.com/") — base URL contract
- @GET("users") — GET method contract
- @POST("users") — POST method contract
- @PUT("users/{id}") — PUT method contract
- @DELETE("users/{id}") — DELETE method contract
- @PATCH("users/{id}") — PATCH method contract
- @Path("id") — path parameter contract
- @Query("q") — query parameter contract
- @QueryMap — query map contract
- @Body — request body contract
- @Header("Authorization") — header contract
- @Headers("Cache-Control: ...") — static headers contract
- @FormUrlEncoded — form encoding contract
- @Multipart — multipart contract
- @Streaming — streaming response contract
- call.execute() — sync call contract
- call.enqueue(callback) — async call contract
- response.body() — response body contract
- response.code() — status code contract

## landmarks
- Retrofit — the core class (Retrofit.java)
- Builder — the builder (Retrofit.java)
- create — the proxy factory (Retrofit.java)
- Call — the call interface (Call.java)
- Callback — the callback interface (Callback.java)
- CallAdapter — the adapter interface (CallAdapter.java)
- Converter — the converter interface (Converter.java)
- OkHttpCall — the okhttp implementation (OkHttpCall.java)
- ServiceMethod — the method model (ServiceMethod.java)
- HttpServiceMethod — the http method (HttpServiceMethod.java)
- RequestFactory — the request factory (RequestFactory.java)
- ParameterHandler — the parameter handler (ParameterHandler.java)
- Platform — the platform detector (Platform.java)
- HttpException — the http error (HttpException.java)

## tests
- retrofit/src/test/java/retrofit2/RetrofitTest.java — core tests
- retrofit/src/test/java/retrofit2/ServiceMethodTest.java — service method tests
- retrofit/src/test/java/retrofit2/RequestFactoryTest.java — request factory tests
- retrofit/src/test/java/retrofit2/OkHttpCallTest.java — call tests
- retrofit/src/test/java/retrofit2/ParameterHandlerTest.java — parameter tests
- retrofit-converters/gson/src/test/ — gson converter tests
- retrofit-converters/moshi/src/test/ — moshi converter tests
- retrofit-adapters/java8/src/test/ — java8 adapter tests
