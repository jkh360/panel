#!/bin/sh
# The display lives entirely inside bubblewrap's private /tmp; never use the
# user's X11 or Wayland socket. Xvfb is local only (-nolisten tcp).
set -eu
if [ "$#" -ne 2 ]; then
    echo 'usage: run.sh /path/to/Xvfb /path/to/electron' >&2
    exit 2
fi
xvfb=$1 electron=$2
"$xvfb" :77 -nolisten tcp -screen 0 800x600x24 < /dev/null > /dev/null 2>&1 &
xpid=$!
trap 'kill "$xpid" 2>/dev/null || true' EXIT
n=0
while [ ! -S /tmp/.X11-unix/X77 ]; do
    if ! kill -0 "$xpid" 2>/dev/null || [ "$n" -ge 100 ]; then
        echo 'panel-hn: isolated display failed to start' >&2
        exit 1
    fi
    n=$((n + 1))
    /app/sleep 0.05
done
export DISPLAY=:77
"$electron" /app/backend/app
