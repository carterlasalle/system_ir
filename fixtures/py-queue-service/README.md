# Incident Queue Service

This service consumes incident messages from a Kafka topic, normalizes and
classifies them, and persists incidents to a SQLite database. Classification
retries on transient failures and falls back to a default severity when the
model service is unavailable.

## Fixture notes

Golden repository for SCC tests: queue consumer -> processor -> store flow
with retry and fallback behavior.
