# Stop re-running the golden integration suite in every scc-cli test binar…

<!-- trace:v1 id=doc.stop-re-running-the-golden-integration-suite-in-every-scc-cli-test-binar -->

<!-- trace:exempt reason=document-structure -->
## Goal

Stop re-running the golden integration suite in every scc-cli test binary. Cargo treats mod golden as compiling golden.rs into each binary, so all 14 tests run about 30 times and starve CI. Split helpers into tests/common/mod.rs with no tests; keep golden.rs as the only binary that runs those tests.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-stop-re-running-the-golden-integration-suite-in-every-scc-cl — Implement Stop re-running the golden integration suite in every scc-cl…

<!-- trace:v1 id=REQ-implement-stop-re-running-the-golden-integration-suite-in-every-scc-cl type=requirement work=WORK-stop-re-running-the-golden-integration-suite-in-every-scc-cli-test-binar -->

Stop re-running the golden integration suite in every scc-cli test binary. Cargo treats mod golden as compiling golden.rs into each binary, so all 14 tests run about 30 times and starve CI. Split helpers into tests/common/mod.rs with no tests; keep golden.rs as the only binary that runs those tests.
