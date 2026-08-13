"""Python semantic-facts fixture: fastapi-style service.

Exercises the Wave 9 fact families: __all__, module-level exports,
decorators, class fields, include_router/add_middleware registrations,
on_event callbacks and configuration reads.
"""
from dataclasses import dataclass, field
from fastapi import FastAPI, APIRouter
import os

__all__ = ["create_app", "ping", "get_item", "Item", "Cart", "router", "startup_event"]


def _internal_helper() -> int:
    return 1


@dataclass
class Item:
    """A catalog item."""

    name: str
    tags: list = field(default_factory=list)

    def describe(self) -> str:
        return self.name


class Cart:
    """Shopping cart state."""

    capacity = 5
    default_items = []

    def __init__(self, owner: str):
        self.owner = owner
        self.items = []
        self.tags = {}

    def add(self, item: str) -> None:
        self.items.append(item)


router = APIRouter()


@router.get("/ping")
def ping() -> dict:
    """Liveness probe."""
    return {"pong": True}


@router.get("/items/{item_id}")
def get_item(item_id: int) -> int:
    return item_id


class RequestLogger:
    """Middleware placeholder."""

    def __call__(self, request) -> None:
        pass


def create_app() -> FastAPI:
    """Application factory: assembles the router and middleware."""
    app = FastAPI(title="facts")
    app.include_router(router)
    app.add_middleware(RequestLogger)
    app.add_exception_handler(ValueError, _internal_helper)

    @app.on_event("startup")
    def startup_event() -> None:
        port = os.getenv("PORT", "8080")
        database_url = os.environ["DATABASE_URL"]
        debug = settings.DEBUG
        print(port, database_url, debug)

    @app.on_event("shutdown")
    def shutdown_event() -> None:
        pass

    return app


def main() -> None:
    """CLI entry: argparse surface (cli_flags + cli-subcommand entrypoint)."""
    import argparse

    parser = argparse.ArgumentParser(prog="facts")
    parser.add_argument("--port", type=int, default=8080)
    parser.add_argument("--verbose", action="store_true")
    sub = parser.add_subparsers()
    serve = sub.add_parser("serve")
    serve.add_argument("--workers", type=int, default=1)
    parser.parse_args()
