"""Contract-ontology python half: http route, CLI flags, config reads,
event topics, and a serializer/deserializer pair — the evidence the
subclass ontology derives from."""
from flask import Flask
import os


class User:
    """A user record with a serializer/deserializer pair around the type."""

    def __init__(self, name):
        self.name = name

    def to_dict(self):
        return {"name": self.name}

    @classmethod
    def from_dict(cls, data):
        return cls(data["name"])


app = Flask(__name__)


@app.get("/users")
def list_users():
    """Route handler (http contract)."""
    return [u.to_dict() for u in [User("a")]]


def main():
    """CLI entry: argparse surface (cli contract)."""
    import argparse

    parser = argparse.ArgumentParser(prog="ontology")
    parser.add_argument("--port", type=int, default=8080)
    parser.add_argument("--verbose", action="store_true")
    parser.parse_args()


def emit_events():
    """Event producer (kafka publish) — event contract topic."""
    kafka.publish("user.created")
    kafka.publish("user.updated")


def consume_events():
    """Event consumer (kafka subscribe) — same topic surface."""
    kafka.subscribe("user.created")


def read_config():
    """Config reads (config contract)."""
    port = os.getenv("PORT", "8080")
    debug = settings.DEBUG
    print(port, debug)
