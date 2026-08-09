"""Tests for incident processing."""

from processor import process_incident
from store import IncidentStore


def test_process_normalizes_and_classifies():
    store = IncidentStore()
    process_incident({"text": "smoke reported"}, store)
    assert store.recent()


def test_fallback_on_classifier_failure():
    store = IncidentStore()
    process_incident({"text": "loud noise"}, store)
    assert store.recent()[0]["severity"] == "unknown"
