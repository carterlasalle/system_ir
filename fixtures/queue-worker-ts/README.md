# Queue Worker

This application consumes radio audio events from Kafka, transcribes them
through an external ASR API with retries, normalizes street names, and
persists normalized transcripts in Redis.

## Fixture notes

Golden repository for SCC tests: async queue workflow with retry and fallback.
