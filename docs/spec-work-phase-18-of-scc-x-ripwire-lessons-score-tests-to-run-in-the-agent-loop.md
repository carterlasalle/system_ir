# Phase 18 of SCC x Ripwire lessons: score tests_to_run in the agent loop …

<!-- trace:v1 id=doc.phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the-agent-loop -->

<!-- trace:exempt reason=document-structure -->
## Goal

Phase 18 of SCC x Ripwire lessons: score tests_to_run in the agent loop against gold tests. Locator and explore arms parse SCC TESTS rows (name + file + reason) and Ripwire pack-task <test p=...> rows and report tests_localization vs tasks.json ground_truth.tests. Baseline has no tests_to_run list and scores 0 when gold tests exist. Empty gold tests are omitted from the clustered mean, not scored as 1.0. Do not change production ranking, MCP tool count, context levels, or packer quotas. This is still a deterministic pack-consumer, not an LLM/SWE-bench repair claim. Do not add C++ extractors or symbol-addressed edits.

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-implement-phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the — Implement Phase 18 of SCC x Ripwire lessons: score tests_to_run in the…

<!-- trace:v1 id=REQ-implement-phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the type=requirement work=WORK-phase-18-of-scc-x-ripwire-lessons-score-tests-to-run-in-the-agent-loop -->

Phase 18 of SCC x Ripwire lessons: score tests_to_run in the agent loop against gold tests. Locator and explore arms parse SCC TESTS rows (name + file + reason) and Ripwire pack-task <test p=...> rows and report tests_localization vs tasks.json ground_truth.tests. Baseline has no tests_to_run list and scores 0 when gold tests exist. Empty gold tests are omitted from the clustered mean, not scored as 1.0. Do not change production ranking, MCP tool count, context levels, or packer quotas. This is still a deterministic pack-consumer, not an LLM/SWE-bench repair claim. Do not add C++ extractors or symbol-addressed edits.
