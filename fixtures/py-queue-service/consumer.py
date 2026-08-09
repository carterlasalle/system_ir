"""Kafka consumer: receives incident messages and dispatches them."""
from processor import process_incident
from store import IncidentStore


def consume(message: dict) -> None:
    store = IncidentStore()
    process_incident(message, store)


if __name__ == "__main__":
    consume({"text": "smoke reported"})
