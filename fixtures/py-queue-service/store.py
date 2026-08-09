"""Incident persistence in SQLite."""
import sqlite3


class IncidentStore:
    """Owns the incidents table."""

    def __init__(self):
        self.db = sqlite3.connect("incidents.db")

    def save_incident(self, text: str, severity: str) -> None:
        self.db.execute(
            "INSERT INTO incidents (text, severity) VALUES (?, ?)",
            (text, severity),
        )
        self.db.commit()

    def recent(self, limit: int = 50) -> list[dict]:
        rows = self.db.execute(
            "SELECT id, text, severity FROM incidents ORDER BY id DESC LIMIT ?",
            (limit,),
        ).fetchall()
        return [dict(r) for r in rows]
