#!/usr/bin/env bash
# Stage the WINDOWED (qtfb) AppLoad bundle into dist/inkwordle-win/. No vendor
# libs (no libquill.so), no takeover script — just the binary + model + data +
# a qtfb:true manifest. Ready for `remagic install dist/inkwordle-win`.
set -euo pipefail
cd "$(dirname "$0")/.."

BIN=target/aarch64-unknown-linux-gnu/release/inkwordle
[ -f "$BIN" ] || { echo "build first: ./build-win.sh" >&2; exit 1; }

rm -rf dist/inkwordle-win
mkdir -p dist/inkwordle-win
install -m 755 "$BIN" dist/inkwordle-win/inkwordle
install -m 755 scripts/inkwordle-win.sh dist/inkwordle-win/
sed -i 's/\r$//' dist/inkwordle-win/*.sh 2>/dev/null || true
install -m 644 external.manifest.win.json dist/inkwordle-win/external.manifest.json
install -m 644 settings.schema.json dist/inkwordle-win/
install -m 644 assets/icon.png dist/inkwordle-win/
# GPLv3 license + epfb-re attribution ship with the binary.
install -m 644 ../LICENSE dist/inkwordle-win/LICENSE
install -m 644 ../quill/NOTICE dist/inkwordle-win/NOTICE 2>/dev/null || true
# On-device recognition model + hint definitions (loaded next to the binary).
install -m 644 assets/emnist-62.onnx dist/inkwordle-win/
install -m 644 assets/definitions.tsv dist/inkwordle-win/

echo "staged: $(du -sh dist/inkwordle-win | cut -f1) in dist/inkwordle-win/"
