"""CFG evidence fixture: branch on validity, cleanup in finally, one awaitable.

Exercises the CFG-backed causal FlowGraph:
- `process` validates (inside try), branches save/reject on the if/else,
  then cleans up in a finally block (a sequential Next edge that follows
  the branches by lexical order).
- `persist` awaits a local coroutine (Async edge).
- `fanout` is straight-line fanout (Next-only, zero Branch edges).
"""
import asyncio


def validate(payload: dict) -> bool:
    """True when the payload is acceptable."""
    return payload.get("ok") is True


def save(payload: dict) -> None:
    """Persist the validated payload."""
    pass


def reject(payload: dict) -> None:
    """Record the rejected payload."""
    pass


def cleanup() -> None:
    """Best-effort cleanup, always runs."""
    pass


def first() -> None:
    pass


def second() -> None:
    pass


def fanout() -> None:
    first()
    second()


async def tick() -> None:
    """Local awaitable."""
    pass


async def persist(payload: dict) -> None:
    await tick()
    save(payload)


def process(payload: dict) -> None:
    try:
        valid = validate(payload)
        if valid:
            save(payload)
        else:
            reject(payload)
    finally:
        cleanup()


def main() -> None:
    process({"ok": True})
    fanout()
    asyncio.run(persist({"ok": True}))


if __name__ == "__main__":
    main()
