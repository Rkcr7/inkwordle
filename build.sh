#!/usr/bin/env bash
# Build GlyphLab (quill + the takeover app) inside the reMarkable cross-toolchain
# container, then stage the install bundle. Reuses the muse-xbuild toolchain
# image (a generic Ubuntu + Rust + ferrari-SDK image — not Muse-specific).
#
#   ./build.sh
#
# Output: game/dist/inkwordle/  (ready for `remagic install game/dist/inkwordle`)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
IMAGE=muse-xbuild:latest
DEVICE=${REMAGIC_HOST:-10.11.99.1}

# The vendor lib (reMarkable's proprietary engine) is pulled from the device on
# the host, where SSH works — the container has no route to the tablet.
mkdir -p "$HERE/quill/vendor"
if [ ! -f "$HERE/quill/vendor/libqsgepaper.so" ]; then
    echo "==> pulling libqsgepaper.so from the device"
    scp -O "root@$DEVICE:/usr/lib/plugins/scenegraph/libqsgepaper.so" "$HERE/quill/vendor/"
fi

case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) MOUNT=$(cygpath -w "$HERE"); export MSYS_NO_PATHCONV=1 ;;
    *) MOUNT="$HERE" ;;
esac

docker build -t "$IMAGE" "$HERE/../muse-standalone/build" 2>/dev/null || true

echo "==> building quill + inkwordle in the container"
docker run --rm -v "${MOUNT}:/work" "$IMAGE" bash -lc "
    set -e
    find /work -name '*.sh' -exec sed -i 's/\r\$//' {} +
    # quill never changes between iterations — only build it once
    [ -f /work/quill/build/libquill.so ] || ( cd /work/quill && bash build.sh )
    cd /work/game && bash build-takeover.sh
    cd /work/game && bash scripts/make-bundle.sh
"
echo "==> done. Install with:  remagic install \"$HERE/game/dist/inkwordle\""
