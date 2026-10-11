#!/bin/sh
# Package removal runs as root, but audio restoration always runs as its owner.
set -eu
command -v loginctl >/dev/null 2>&1 || exit 0
loginctl list-users --no-legend 2>/dev/null | while read -r uid user rest; do
    case "$uid" in ''|*[!0-9]*|0) continue ;; esac
    runtime="/run/user/$uid"
    [ -S "$runtime/bus" ] || continue
    runuser -u "$user" -- env XDG_RUNTIME_DIR="$runtime" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
        /usr/bin/macafon ctl autostart off >/dev/null 2>&1 || true
    runuser -u "$user" -- env XDG_RUNTIME_DIR="$runtime" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
        /usr/bin/macafon ctl quit >/dev/null 2>&1 || true
    runuser -u "$user" -- env XDG_RUNTIME_DIR="$runtime" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
        systemctl --user disable --now macafond.service >/dev/null 2>&1 || true
    runuser -u "$user" -- env XDG_RUNTIME_DIR="$runtime" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=$runtime/bus" \
        /usr/bin/macafon restore-default >/dev/null 2>&1 || true
done

