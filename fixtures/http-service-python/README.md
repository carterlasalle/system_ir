# Transcript HTTP API

This application serves normalized radio transcripts over HTTP. The API
layer routes requests, and the service layer owns the transcript repository
and its database. Raw transcripts are immutable once stored.

## Fixture notes

Golden repository for SCC tests: HTTP -> service -> database flow.
