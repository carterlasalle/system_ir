"""Payment service: owns payment records."""
from db import PaymentDb

def handle_payment(amount: int) -> dict:
    db = PaymentDb()
    payment = db.insert(amount)
    return {"id": payment["id"], "amount": payment["amount"]}

def list_payments() -> list[dict]:
    return PaymentDb().recent()
