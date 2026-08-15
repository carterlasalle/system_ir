from __future__ import annotations

from typing import List, Optional


async def fetch_all(
    endpoint: str,
    limit: int = 20,
    retries: Optional[int] = None,
) -> List[dict]:
    """Fetch records with pagination."""
    return []


class QueryBuilder:
    def build(
        self,
        fields: List[str],
        where: Optional[str] = None,
        order_by: str = "id",
    ) -> "QueryBuilder":
        return self
