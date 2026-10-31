#!/bin/sh
# Refresh upgraded files for logged-in users, preserving their autostart choice.
set -eu
case "${1:-}" in
    configure|1|2) ;;
    *) exit 0 ;;
esac
command -v loginctl >/dev/null 2>&1 || exit 0
loginctl list-users --no-legend 2>/dev/null | while read -r uid user rest; do
    case "$uid" in ''|*[!0-9]*|0) continue ;; esac
    runtime="/run/user/$uid"
    [ -S "$runtime/bus" ] || continue
    runuser -u "$user" -- env XDG_RUNTIME_DIR="$runtime" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
        systemctl --user daemon-reload >/dev/null 2>&1 || true
    # Never enable autostart or start an inactive daemon during package installation.
    runuser -u "$user" -- env XDG_RUNTIME_DIR="$runtime" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
        systemctl --user try-restart macafond.service >/dev/null 2>&1 || true
done
