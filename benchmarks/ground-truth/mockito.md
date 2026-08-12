# mockito
> https://github.com/mockito/mockito | Java | java service (lib) | ~102k LOC

## components
- Mockito — facade class in mockito-core/src/main/java/org/mockito/Mockito.java (4096 lines) exposing the static mock/when/verify API
- MockitoCore — internal engine in mockito-core/src/main/java/org/mockito/internal/MockitoCore.java implementing mock/when/verify/validateMockitoUsage behind the facade
- ArgumentMatchers — static matcher factory in org/mockito/ArgumentMatchers.java (any, eq, anyString, anyInt, argThat)
- MockSettings — settings interface built via withSettings() for name/defaultAnswer/extraInterfaces on a mock
- Answers — enum of default answers incl. RETURNS_DEFAULTS, RETURNS_SMART_NULLS, CALLS_REAL_METHODS
- MockMaker — SPI interface in org/mockito/plugins/MockMaker.java for pluggable mock-creation backends
- ByteBuddyMockMaker — default subclass mock maker in internal/creation/bytebuddy/ generating mock classes via Byte Buddy
- InlineByteBuddyMockMaker — inline mock maker in internal/creation/bytebuddy/ that can mock final classes/types via instrumentation
- MockUtil — helpers in internal/util/MockUtil.java (isMock, getMockHandler) used across internal and API code
- MockingProgress — thread-local stubbing/verification state in internal/progress/ driving when()/verify() sequencing
- MockitoSession — org.mockito.MockitoSession API owning initMocks/strictness/finishMocking lifecycle
- MockitoExtension — JUnit Jupiter extension in mockito-extensions/mockito-junit-jupiter/ (implements BeforeEachCallback, AfterEachCallback, ParameterResolver)

## entrypoints
- `mock(Class<T> classToMock)` — primary mock creation entry, delegates to MOCKITO_CORE.mock(classToMock, settings)
- `when(T methodCall)` — stubbing entry returning OngoingStubbing, delegates to MOCKITO_CORE.when
- `verify(T mock)` — interaction verification entry, defaults to times(1)
- `MockitoAnnotations.openMocks` — initializes @Mock/@InjectMocks/@Captor fields on a test instance, returns AutoCloseable
- `mockitoSession()` — MockitoSessionBuilder entry configuring initMocks/strictness/logger then startMocking()
- `MockitoExtension.beforeEach` — JUnit Jupiter lifecycle hook that starts a MockitoSession per test method
- `MockitoJUnitRunner` — JUnit4 runner in org/mockito/junit/ driving @Mock init and strict stubbings (imported by MockitoExtension)
- `spy(T object)` — wraps a real instance with CALLS_REAL_METHODS default answer
- `doReturn(Object toBeReturned)` — Stubber entry for stubbing methods that bypass when() (e.g. void methods)

## flows
- `mock()` -> MockitoCore.mock -> MockMaker.createMock — creation chain from facade through internal engine to the plugin mock maker
- `when()` -> MockitoCore.when -> stubbingStarted() -> OngoingStubbing.thenReturn — stubbing flow sequenced by MockingProgress
- `verify()` -> MockitoCore.verify -> VerificationModeFactory.times(1) — default verification flow
- `any()`/`eq()` -> reportMatcher() -> matcher stack — argument-matcher registration in ArgumentMatchers consumed by the recorded invocation
- `timeout(long millis)` — asynchronous verification mode wrapping VerificationModeFactory.times(1)
- `inOrder(mocks)` -> InOrder.verify — ordered verification across multiple mocks via MOCKITO_CORE.inOrder
- `MockitoExtension.beforeEach` — per-test JUnit setup flow: MockitoSession.startMocking -> initMocks + Strictness.STRICT_STUBS
- `doThrow(Throwable...)` -> MOCKITO_CORE.stubber() -> Stubber.doThrow — stubber flow for stubbing exceptions on any method

## ownership
- MockingProgress — thread-local mocking state owned per thread, reported by mockingProgress() in MockitoCore
- Plugins — internal/configuration/plugins/Plugins.java registry owning MockMaker/MockitoLogger plugin resolution
- MockitoSession — owns per-test mocking lifecycle (startMocking/finishMocking) incl. strict stubbing enforcement
- MockMakers — org.mockito.MockMakers constants: INLINE = "mock-maker-inline", SUBCLASS = "mock-maker-subclass"
- mockito-core — Gradle module include("mockito-core") in settings.gradle.kts owning the main library artifact
- MockUtil — internal/util/MockUtil.java owns mock-identity queries used by spy() and MockingDetails
- GlobalConfiguration — internal/configuration/GlobalConfiguration.java loads org.mockito.plugins.* config for default answers and plugins

## contracts
- `OngoingStubbing` — contract returned by when(): thenReturn/thenThrow/thenAnswer/thenCallRealMethod
- `VerificationMode` — contract implemented by times/never/atLeast/atMost/atLeastOnce in Mockito.java
- `ArgumentMatcher<T>` — functional interface accepted by argThat for custom matching
- `ArgumentCaptor` — capture contract for argument values (captor.getValue())
- `@Mock` — field annotation contract processed by openMocks/MockitoExtension
- `@InjectMocks` — injection annotation for constructor/setter/field injection of mocks
- `Strictness.STRICT_STUBS` — strict-stubbing quality contract enforced by MockitoSession (unused stubs fail)
- `MockSettings.extraInterfaces` — contract for making a mock implement additional interfaces
- `@Captor` — annotation in org/mockito/Captor.java for ArgumentCaptor field initialization

## tests
- MockitoTest — mockito-core/src/test/java/org/mockito/MockitoTest.java covering core mock/when/verify API
- ArgumentCaptorTest — mockito-core/src/test/java/org/mockito/ArgumentCaptorTest.java captor behavior tests
- JunitJupiterTest — mockito-extensions/mockito-junit-jupiter/src/test/java/org/mockitousage/JunitJupiterTest.java extension integration tests
- StrictnessTest — mockito-extensions/mockito-junit-jupiter/src/test/java/org/mockitousage/StrictnessTest.java strict stubbing enforcement tests
- InjectMocksTest — mockito-extensions/mockito-junit-jupiter/src/test/java/org/mockitousage/InjectMocksTest.java injection tests
- verification — mockito-core/src/test/java/org/mockito/verification/ package testing verification modes
- internal — mockito-core/src/test/java/org/mockito/internal/ package testing the internal engine
- StaticMockingExperimentTest — mockito-core/src/test/java/org/mockito/StaticMockingExperimentTest.java static-mock experiments
