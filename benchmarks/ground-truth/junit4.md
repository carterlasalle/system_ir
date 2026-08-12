# junit4
> https://github.com/junit-team/junit4 | Java | java service (lib) | ~45k LOC

## architecture
- JUnitCore — runner facade in org/junit/runner/JUnitCore.java with run/runClasses/main
- Runner — abstract base class in runner/Runner.java defining run(Notifier)/getDescription
- ParentRunner — runners/ParentRunner.java base class for hierarchical runners (Suite, BlockJUnit4ClassRunner)
- BlockJUnit4ClassRunner — runners/BlockJUnit4ClassRunner.java executes @Test methods with @Before/@After lifecycle
- Suite — runners/Suite.java runs a group of test classes together
- Parameterized — runners/Parameterized.java data-driven parameterized test runner
- JUnit38ClassRunner — internal/runners/JUnit38ClassRunner.java adapter running legacy junit.framework.Test
- RunNotifier — runner/notification/RunNotifier.java event dispatch (fireTestStarted/fireTestFailure)
- TestRule — org/junit/rules/TestRule.java interface for reusable rule wrappers

## entrypoints
- `JUnitCore.main` — CLI entry: System.exit(runMain(args)) in JUnitCore.java
- `JUnitCore.run` — programmatic entry building a Request and running it
- `runClasses(Class<?>... classes)` — static convenience entry wrapping new JUnitCore().run
- `JUnitCommandLineParseResult` — parses CLI args including `--filter` for test selection
- `@RunWith` — runner/RunWith.java annotation selecting a custom Runner class
- `Request.classes` — runner/Request.java entry constructing a Runner for given classes
- `@Test` — method-level marker recognized by BlockJUnit4ClassRunner

## behavior
- `JUnitCore.run` -> Request.classes -> Computer -> Runner — runner assembly flow from facade to suite
- `BlockJUnit4ClassRunner` -> MethodRoadie -> RunRules -> before/test/after — per-method execution flow in internal/runners
- `ParentRunner.run` -> runChildren — recursive execution of child runners
- `RunNotifier.fireTestStarted` -> listeners — notification flow to registered RunListeners
- `--filter` -> FilterFactories -> Filter — CLI filtering flow in JUnitCommandLineParseResult
- `assertThrows` -> ThrowingRunnable.run() inside try/catch — assertion flow in Assert.java
- `@RunWith(Parameterized.class)` -> Parameterized -> per-parameter-set child runners — parameterized execution flow

## state_authority
- Result — owns run/failure/ignore counters via AtomicInteger and the failure list
- Description — owns test names, unique ids, and the children tree
- Failure — runner/notification/Failure.java owns the failed Description plus thrown exception (getException)
- TestClass — internal/runners/TestClass.java owns reflection over a test class's annotations/methods
- TestMethod — internal/runners/TestMethod.java wraps a single @Test method's invocation metadata
- RuleContainer — runners/RuleContainer.java owns the ordered rule instances applied to tests
- RunNotifier — owns the listener list (CopyOnWriteArrayList) and the pleaseStop flag

## contracts
- `@Test` — expected-exception contract on the Test annotation (expected = ...)
- `timeout()` — timeout contract on the Test annotation (long timeout() default 0L)
- `assertEquals(Object expected, Object actual)` — core equality assertion contract in Assert.java
- `assertThrows` — contract returning the thrown throwable (Class<? extends Throwable> expectedThrowable, ThrowingRunnable)
- `@Before` — per-test setup method contract; @After the matching teardown contract
- `@BeforeClass` — static setup contract run once per class
- `@Rule` — rule field contract processed by RuleContainer
- `@Ignore` — org/junit/Ignore.java contract to skip a test or class
- `@FixMethodOrder(MethodSorters.NAME_ASCENDING)` — execution-order contract with runners/MethodSorters.java

## landmarks
- Test — @Test annotation in src/main/java/org/junit/Test.java with expected() and timeout() attributes
- Assert — static assertion class org.junit.Assert (assertEquals/assertTrue/assertThrows/fail)
- Result — runner/Result.java collecting run counts, failures, and ignored counts
- Description — runner/Description.java test identity and display names (createTestDescription)
- Before — @Before lifecycle annotation in org/junit/Before.java (run before each @Test)

## tests
- AssertionTest — src/test/java/org/junit/tests/assertion/AssertionTest.java covering assert APIs
- JUnitCoreReturnsCorrectExitCodeTest — src/test/java/org/junit/tests/running/core/ verifying JUnitCore.main exit codes
- CommandLineTest — src/test/java/org/junit/tests/running/core/CommandLineTest.java CLI parsing tests
- TimeoutTest — src/test/java/org/junit/tests/running/methods/TimeoutTest.java timeout behavior tests
- ExpectedTest — src/test/java/org/junit/tests/running/methods/ExpectedTest.java expected-exception tests
- AllTests — src/test/java/org/junit/tests/AllTests.java top-level suite aggregating all unit tests
- AllCoreTests — src/test/java/org/junit/tests/running/core/AllCoreTests.java suite for runner core tests
