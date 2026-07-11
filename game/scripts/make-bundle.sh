#!/usr/bin/env bash
# Stage the AppLoad bundle into dist/inkwordle/, ready for `remagic install`.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/aarch64-unknown-linux-gnu/release/inkwordle
[ -f "$BIN" ] || { echo "build first: ./build-takeover.sh" >&2; exit 1; }
[ -f ../quill/build/libquill.so ] || { echo "missing ../quill/build/libquill.so" >&2; exit 1; }

rm -rf dist/inkwordle
mkdir -p dist/inkwordle
install -m 755 "$BIN" dist/inkwordle/inkwordle
install -m 755 ../quill/build/libquill.so dist/inkwordle/
install -m 755 scripts/appload-launch.sh scripts/inkwordle-takeover.sh dist/inkwordle/
sed -i 's/\r$//' dist/inkwordle/*.sh 2>/dev/null || true
install -m 644 external.manifest.json settings.schema.json dist/inkwordle/
install -m 644 assets/icon.png dist/inkwordle/
# GPLv3 license text + epfb-re attribution must ship with the binary (it links libquill/epfb-re).
install -m 644 ../LICENSE dist/inkwordle/LICENSE
install -m 644 ../quill/NOTICE dist/inkwordle/NOTICE
# Only the accurate model ships (emnist-62). It loads from disk next to the binary.
install -m 644 assets/emnist-62.onnx dist/inkwordle/

echo "staged: $(du -sh dist/inkwordle | cut -f1) in dist/inkwordle/"
