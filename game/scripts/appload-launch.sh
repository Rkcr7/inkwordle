#!/bin/sh
# AppLoad entry point for takeover mode. AppLoad runs this inside xochitl's
# world, which is about to be stopped — so detach the real launch into a
# transient systemd unit (PID-1-owned, survives xochitl) and exit immediately.
HERE=$(cd "$(dirname "$0")" && pwd)
# A folder-push or release-zip install can drop the executable bits; restore
# them here so InkWordle launches without a manual `chmod` over SSH.
chmod +x "$HERE/inkwordle" "$HERE"/*.sh "$HERE/libquill.so" 2>/dev/null
systemctl is-active --quiet inkwordle-takeover && exit 0
# ExecStopPost is the safety net the in-script trap can't be: it runs even if
# the app is SIGKILLed or OOM-killed, so the tablet never stays UI-less.
systemd-run --unit=inkwordle-takeover --collect \
    --property="ExecStopPost=-/bin/systemctl start xochitl" \
    /bin/bash "$HERE/inkwordle-takeover.sh" \
  || {
        # Older systemd rejected ExecStopPost — arm a companion watchdog unit.
        systemd-run --unit=inkwordle-takeover --collect /bin/bash "$HERE/inkwordle-takeover.sh"
        systemd-run --unit=inkwordle-restore-xochitl --collect /bin/sh -c \
            'sleep 3; while systemctl is-active --quiet inkwordle-takeover; do sleep 2; done; systemctl start xochitl'
    }
exit 0
