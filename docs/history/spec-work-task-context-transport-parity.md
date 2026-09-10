# Task context transport parity

<!-- trace:v1 id=doc.task-context-transport-parity -->

<!-- trace:exempt reason=document-structure -->
## Goal

One complete task artifact (pack + surface delta) derived from a single builder consumed by CLI text, CLI JSON, MCP, HTTP, Hermes and both SDKs, so transport choice cannot change semantic quality (spec Part C).

<!-- trace:exempt reason=document-structure -->
## Requirements

### REQ-complete-task-context-identical-across-transports — Complete task context identical across transports

<!-- trace:v1 id=REQ-complete-task-context-identical-across-transports type=requirement work=WORK-task-context-transport-parity -->

For identical inputs, config and ledger state, every transport must derive the task pack AND surface delta from the same TaskContextArtifact builder; outputs may differ in serialization only, never in selected ids, omissions or scorer availability.
