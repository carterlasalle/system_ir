#!/usr/bin/env bash
# Release helper (SCC-226): static binary + SBOM + signed-style checksums.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=${1:-$(grep '^version' crates/scc-core/Cargo.toml | head -1 | cut -d'"' -f2)}
echo "==> building release ${VERSION}"
cargo build --release -p scc-cli
mkdir -p dist
cp target/release/scc "dist/scc-${VERSION}-$(uname -s)-$(uname -m)"
cargo tree --edges normal --prefix none --format '{p} {v}' > "dist/sbom-${VERSION}.txt"
cd dist
shasum -a 256 "scc-${VERSION}-$(uname -s)-$(uname -m)" > "scc-${VERSION}.sha256"
echo "==> artifacts:"
ls -la
