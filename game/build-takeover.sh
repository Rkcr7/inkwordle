#!/bin/bash
# Build wordle in TAKEOVER mode (links libquill.so + vendor Qt/qsgepaper).
# Runs inside the reMarkable cross-toolchain container. Must link with the
# ferrari SDK's gcc (glibc 2.38) — see muse's build-takeover.sh for the why.
set -euo pipefail
cd "$(dirname "$0")"

SDK=~/rm-sdk-3.26
ENV=$(ls "$SDK"/environment-setup-* | head -n1)
unset LD_LIBRARY_PATH
source "$ENV"

# Ensure quill's libquill.so + vendor/libqsgepaper.so exist.
if [ ! -f ../quill/build/libquill.so ]; then
    echo "building quill first..."
    ( cd ../quill && ./build.sh )
fi

cat > /tmp/glyph-sdk-cc.sh <<EOF
#!/bin/bash
exec $CC "\$@"
EOF
chmod +x /tmp/glyph-sdk-cc.sh
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=/tmp/glyph-sdk-cc.sh

cargo build --release --target aarch64-unknown-linux-gnu --features takeover "$@"
echo "built: target/aarch64-unknown-linux-gnu/release/wordle (takeover)"
