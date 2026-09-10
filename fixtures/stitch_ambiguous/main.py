# trace:exempt reason=test-data
"""Health endpoint with a conflicting verb (ambiguous-stitch fixture)."""

from fastapi import FastAPI

app = FastAPI()


@app.post("/health")
def update_health() -> dict:
    return {"status": "updated"}
