#!/bin/sh
set -eu
# Debian remove/deconfigure and RPM final erase (0); upgrades leave the daemon alone.
case "${1:-}" in
    remove|deconfigure|0)
        /usr/share/macafon/cleanup-user-sessions.sh || true
        ;;
esac

