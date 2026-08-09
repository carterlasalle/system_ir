"""Incident processing: normalize, classify with retry, fallback on failure."""
import tenacity
from store import IncidentStore


@tenacity.retry(wait=tenacity.wait_fixed(1), stop=tenacity.stop_after_attempt(3))
def classify(text: str) -> str:
    """Ask the model service for a severity classification."""
    raise NotImplementedError("model call")


def process_incident(message: dict, store: IncidentStore) -> None:
    text = message.get("text", "")
    try:
        severity = classify(text)
    except Exception:
        severity = "unknown"  # fallback: default severity
    store.save_incident(text, severity)
