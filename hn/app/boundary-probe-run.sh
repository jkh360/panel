#!/bin/sh
# Only run by hn/test-boundaries.sh without an external network route.
set -eu
xvfb=$1 electron=$2
"$xvfb" :78 -nolisten tcp -screen 0 800x600x24 < /dev/null > /dev/null 2>&1 &
xpid=$!
trap 'kill "$xpid" 2>/dev/null || true' EXIT
n=0
while [ ! -S /tmp/.X11-unix/X78 ]; do
    if [ "$n" -ge 100 ]; then echo 'boundary probe: Xvfb did not start' >&2; exit 1; fi
    n=$((n + 1))
    /app/sleep 0.05
done
export DISPLAY=:78
"$electron" /app/backend/app/boundary-probe.cjs
