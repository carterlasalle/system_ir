"""Tests for transcript persistence and normalization."""

from services.transcripts import TranscriptRepository, Normalizer


def test_repository_find_all():
    repo = TranscriptRepository()
    rows = repo.find_all()
    assert isinstance(rows, list)


def test_normalization_preserves_raw():
    n = Normalizer(resolver=None)
    out = n.normalize("raw audio text")
    assert out == "raw audio text"
