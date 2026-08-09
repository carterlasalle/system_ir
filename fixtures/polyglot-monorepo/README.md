# Polyglot Monorepo

This repository combines a Python service that processes payments, a
TypeScript web frontend that calls the service, and a shared contract
module used by both sides. The Python service owns payment state; the web
layer consumes the payment API contract.

## Fixture notes

Golden repository for SCC tests: cross-language monorepo with shared
contracts (SCC-003 polyglot category).
