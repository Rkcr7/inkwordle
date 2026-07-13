#!/bin/bash
# Build inkwordle in WINDOWED (qtfb) mode — renders into an AppLoad window via
# the qtfb shared-memory protocol, no takeover, no libquill / vendor Qt libs.
# Runs inside the cross-toolchain container; still links with the ferrari SDK's
# gcc so the binary matches the device glibc (2.38).
set -euo pipefail
cd "$(dirname "$0")"

SDK=~/rm-sdk-3.26
ENV=$(ls "$SDK"/environment-setup-* | head -n1)
unset LD_LIBRARY_PATH
source "$ENV"

cat > /tmp/glyph-sdk-cc.sh <<EOF
#!/bin/bash
exec $CC "\$@"
EOF
chmod +x /tmp/glyph-sdk-cc.sh
export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=/tmp/glyph-sdk-cc.sh

# No --features takeover: this build has no quill FFI and links no vendor libs.
cargo build --release --target aarch64-unknown-linux-gnu "$@"
echo "built: target/aarch64-unknown-linux-gnu/release/inkwordle (windowed / qtfb)"
