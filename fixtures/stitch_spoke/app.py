# trace:exempt reason=test-data
"""App consuming the shared library (multi-repo stitch fixture)."""

from stitch_hub import normalize_email


def render(addr: str) -> str:
    return f"contact: {normalize_email(addr)}"
