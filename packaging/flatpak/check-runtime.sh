#!/bin/sh
set -eu
for binary in /app/bin/* $(find /app/lib -path /app/lib/debug -prune -o -type f -name '*.so*' ! -name '*.debug' -print); do
    result=$(ldd "$binary")
    echo "$result"
    if echo "$result" | grep -E 'not found|version .* not found'; then exit 1; fi
done
/app/bin/macafond --help
/app/bin/macafon-stt --help
