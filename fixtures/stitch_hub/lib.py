# trace:exempt reason=test-data
"""Shared email-normalization library (multi-repo stitch fixture)."""


def normalize_email(addr: str) -> str:
    """Lowercase the domain half of an email address."""
    local, sep, domain = addr.partition("@")
    return local + sep + domain.lower() if sep else addr
