"""Payment database access."""
import sqlite3

class PaymentDb:
    def __init__(self):
        self.db = sqlite3.connect("payments.db")

    def insert(self, amount: int) -> dict:
        cur = self.db.execute("INSERT INTO payments (amount) VALUES (?)", (amount,))
        self.db.commit()
        return {"id": cur.lastrowid, "amount": amount}

    def recent(self, limit: int = 20) -> list[dict]:
        rows = self.db.execute("SELECT id, amount FROM payments ORDER BY id DESC LIMIT ?", (limit,))
        return [dict(r) for r in rows]
