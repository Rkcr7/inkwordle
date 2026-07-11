#!/usr/bin/env bash
# Stage the AppLoad bundle into dist/wordle/, ready for `remagic install`.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/aarch64-unknown-linux-gnu/release/wordle
[ -f "$BIN" ] || { echo "build first: ./build-takeover.sh" >&2; exit 1; }
[ -f ../quill/build/libquill.so ] || { echo "missing ../quill/build/libquill.so" >&2; exit 1; }

rm -rf dist/wordle
mkdir -p dist/wordle
install -m 755 "$BIN" dist/wordle/wordle
install -m 755 ../quill/build/libquill.so dist/wordle/
install -m 755 scripts/appload-launch.sh scripts/wordle-takeover.sh dist/wordle/
sed -i 's/\r$//' dist/wordle/*.sh 2>/dev/null || true
install -m 644 external.manifest.json settings.schema.json dist/wordle/
# Only the accurate model ships (emnist-62). It loads from disk next to the binary.
install -m 644 assets/emnist-62.onnx dist/wordle/

echo "staged: $(du -sh dist/wordle | cut -f1) in dist/wordle/"
