"""Native behavior fixture: same-file call chains, no language server.

Every call below is intra-file, so the native extractor resolves each
target to a local symbol and emits an EXTRACTED CALLS edge. Behavior
flows must exist from these native chains alone — LSP resolution is not
required to see the run -> handle -> normalize -> validate -> parse
pipeline in the atlas FLOWS section.
"""


def parse(raw: str) -> list:
    return [line.strip() for line in raw.splitlines() if line.strip()]


def validate(rows: list) -> list:
    cleaned = []
    for row in rows:
        if "skip" not in row:
            cleaned.append(row)
    return cleaned


def normalize(raw: str) -> list:
    return validate(parse(raw))


def handle(payload: dict) -> list:
    return normalize(payload.get("text", ""))


def run() -> None:
    print(handle({"text": "a\nskip\nb"}))


if __name__ == "__main__":
    run()
