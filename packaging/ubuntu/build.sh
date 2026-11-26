#!/usr/bin/env bash
# Build a self-contained Line-GTK Super tarball for Ubuntu inside podman (or docker).
#
#   ./packaging/ubuntu/build.sh [24.04|26.04]
#
# Builds the committed HEAD (uncommitted changes are ignored) and writes
# target/dist/line-gtk-super-<version>-<sha>-ubuntu<rel>-x86_64.tar.gz
set -euo pipefail
REL="${1:-24.04}"
ENGINE="${CONTAINER_ENGINE:-podman}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
DENO_VERSION=2.9.8
IMG="line-gtk-super-build:ubuntu$REL-deno$DENO_VERSION"
WORK="$ROOT/target/ubuntu-build"
SRC="$WORK/src"
CACHE="$WORK/cache"
DIST="$ROOT/target/dist"

"$ENGINE" build -q --build-arg BASE="docker.io/library/ubuntu:$REL" \
  --build-arg DENO_VERSION="$DENO_VERSION" -t "$IMG" -f "$HERE/Containerfile" "$HERE"

rm -rf "$SRC"; mkdir -p "$SRC" "$CACHE/cargo" "$CACHE/deno" "$DIST"
git -C "$ROOT" archive HEAD | tar -x -C "$SRC"
SHA="$(git -C "$ROOT" rev-parse --short HEAD)"

"$ENGINE" run --rm -v "$SRC":/w:Z -w /w \
  -v "$CACHE/cargo":/root/.cargo/registry:Z -v "$CACHE/deno":/root/.cache/deno:Z \
  "$IMG" bash -c '
    set -euo pipefail
    cargo build --release --locked
    ./scripts/build-sidecar.sh
    echo "== tests"
    cargo test --release --locked 2>&1 | grep -E "^test result|FAILED|panicked"
    (cd protocol && deno task check 2>&1 | sed "s/\x1b\[[0-9;]*m//g" | grep -E "passed|failed|not formatted|^error" | tail -3)
  '

VER="$(grep -m1 '^version' "$SRC/Cargo.toml" | cut -d'"' -f2)"
STAGE="line-gtk-super-$VER-$SHA-ubuntu$REL-x86_64"
OUT="$DIST/$STAGE"; rm -rf "$OUT"; mkdir -p "$OUT"
cp -a "$SRC/target/release/line-gtk" "$OUT/"
cp -a "$SRC/protocol" "$SRC/assets" "$OUT/"
rm -rf "$OUT/protocol/node_modules" "$OUT/protocol/.deno" "$OUT/protocol/tests" "$OUT/protocol/vendor/archives"
cp -a "$SRC/packaging/dev.linegtk.LineGtk.desktop" "$SRC/README.md" "$SRC/LICENSE" "$OUT/"
cp -a "$HERE/INSTALL.md" "$OUT/INSTALL.md"
tar -C "$DIST" -czf "$DIST/$STAGE.tar.gz" "$STAGE"
sha256sum "$DIST/$STAGE.tar.gz"
