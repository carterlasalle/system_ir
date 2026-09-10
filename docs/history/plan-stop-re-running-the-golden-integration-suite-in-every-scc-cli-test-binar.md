# PLAN-stop-re-running-the-golden-integration-suite-in-every-scc-cli-test-binar

<!-- trace:v1 id=PLAN-stop-re-running-the-golden-integration-suite-in-every-scc-cli-test-binar type=plan work=WORK-stop-re-running-the-golden-integration-suite-in-every-scc-cli-test-binar -->

<!-- trace:exempt reason=document-structure -->
## Steps

- Implement: Stop re-running the golden integration suite in every scc-cli test binary. Cargo treats mod golden as compiling golden.rs into each binary, so all 14 tests run about 30 times and starve CI. Split help…
