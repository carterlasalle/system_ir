"""Transcript persistence and normalization.

The repository owns all access to the `db` store. Raw transcripts are
immutable once stored: normalization always produces a new record.
"""

import sqlite3


class TranscriptRepository:
    """Owns transcript rows in the database."""

    def __init__(self):
        self.db = sqlite3.connect("transcripts.db")

    def find_all(self) -> list[dict]:
        rows = self.db.execute("SELECT id, raw_text, normalized_text FROM transcripts").fetchall()
        return [dict(r) for r in rows]

    def save(self, raw_text: str, normalized_text: str) -> None:
        self.db.execute(
            "INSERT INTO transcripts (raw_text, normalized_text) VALUES (?, ?)",
            (raw_text, normalized_text),
        )
        self.db.commit()


class Normalizer:
    """Normalizes raw ASR output; falls back to raw text on failure."""

    def __init__(self, resolver=None):
        self.resolver = resolver

    def normalize(self, raw_text: str) -> str:
        try:
            return self.resolver.resolve(raw_text)
        except Exception:
            return raw_text
