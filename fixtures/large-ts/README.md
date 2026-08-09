# Large TypeScript Service

This repository is a moderately large TypeScript service with an API layer,
a domain layer with business rules, infrastructure adapters (database and
queue), and a web rendering layer. The API layer owns the HTTP contracts;
the domain layer owns business logic and invariants; the infrastructure
layer owns persistence and messaging.

## Fixture notes

Golden repository for SCC tests: a larger codebase for scale-sensitive
context tasks (SCC-003 large-TS category).
