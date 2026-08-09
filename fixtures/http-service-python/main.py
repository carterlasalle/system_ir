"""Transcript HTTP API (fixture repo).

This application serves normalized radio transcripts over HTTP.
The API layer routes requests; the service layer owns the
transcript repository and its database.
"""

from fastapi import FastAPI
from services.transcripts import TranscriptRepository, Normalizer

app = FastAPI()


@app.get("/api/transcripts")
def handle_transcripts() -> list[dict]:
    """List all transcripts."""
    repo = TranscriptRepository()
    return repo.find_all()


@app.get("/health")
def health() -> dict:
    return {"status": "ok"}


if __name__ == "__main__":
    import uvicorn

    uvicorn.run(app)
