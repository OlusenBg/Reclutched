#!/usr/bin/env bash
# Builds the static (musl) release binary and packs it into dist/ with a checksum.
# Usage: .github/scripts/package.sh <label>     e.g. v1.0.0 or test-abc1234
set -euo pipefail

label="${1:?usage: package.sh <label>}"
target=x86_64-unknown-linux-musl
name="reclutched-${label}-x86_64-linux"

# The bundled SQLite is C code; musl-gcc compiles it so the result has no shared libraries.
CC_x86_64_unknown_linux_musl=musl-gcc \
    cargo build --release --locked --target "$target" -p reclutched-cli

rm -rf dist
mkdir -p "dist/$name"
cp "target/$target/release/reclutched" README.md LICENSE "dist/$name/"
tar -C dist -czf "dist/$name.tar.gz" "$name"
(cd dist && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
rm -rf "dist/$name"
ls -l dist
