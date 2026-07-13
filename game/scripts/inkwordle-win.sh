#!/bin/sh
# AppLoad entry point for WINDOWED (qtfb) mode. AppLoad allocates a qtfb window,
# passes its key in QTFB_KEY, and runs this — no xochitl stop, no takeover, so
# quitting drops straight back to the launcher. We just repair exec bits (a
# release-zip / folder-push install can drop them) and exec the binary.
HERE=$(cd "$(dirname "$0")" && pwd)
chmod +x "$HERE/inkwordle" 2>/dev/null
exec "$HERE/inkwordle"
