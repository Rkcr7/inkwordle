#!/bin/bash
# Launch InkWordle in full-takeover mode: stop xochitl, run the app against the
# vendor e-ink engine, ALWAYS restore xochitl on exit.
#
# Exit: power button, the Quit button, or SIGTERM. Escape hatch if anything
# wedges: ssh rm 'systemctl start xochitl'.
set -u

restore() {
    rm -f /tmp/epframebuffer.lock
    systemctl start xochitl
}
if [ -z "${REMAGIC_SESSION:-}" ]; then
    trap restore EXIT INT TERM
fi

HERE=$(cd "$(dirname "$0")" && pwd)

# Optional settings (INKWORDLE_IDLE_MS) written by `remagic config inkwordle`.
if [ -f "$HERE/settings.env" ]; then
    set -a; . "$HERE/settings.env"; set +a
fi

if [ -z "${REMAGIC_SESSION:-}" ]; then
    systemctl stop xochitl
fi
rm -f /tmp/epframebuffer.lock          # stale EPD lock blocks the engine
[ -z "${REMAGIC_SESSION:-}" ] && sleep 1

cd "$HERE" || { echo "inkwordle: cannot cd to $HERE" >&2; exit 1; }
LD_LIBRARY_PATH="$HERE:/home/root/quill:/usr/lib/plugins/scenegraph" \
    HOME=/home/root \
    "$HERE/inkwordle"
echo "inkwordle-takeover: closed ($?), restoring xochitl"
