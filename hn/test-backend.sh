#!/bin/sh
# Offline fail-closed regression while online metadata remains gated.
# This intentionally makes NO website requests.
set -eu
unset PANEL_HN_ENABLE_METADATA
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
for tool in electron Xvfb timeout; do
    command -v "$tool" >/dev/null || { echo "panel-hn: missing $tool" >&2; exit 1; }
done
ELECTRON=$(command -v electron)
XVFB=$(command -v Xvfb)
STATE=$(mktemp -d)
trap 'rm -rf -- "$STATE"' EXIT
if printf '%s\n' '{"id":1,"op":"sources-panel"}' |
    timeout 15 "$ROOT/hn/net-sandbox.sh" /bin/sh /app/backend/app/run.sh "$XVFB" "$ELECTRON" \
    > "$STATE/replies" 2> "$STATE/errors"; then
    echo 'panel-hn: SECURITY FAILURE: backend started while the online gate is closed' >&2
    exit 1
fi
if [ -s "$STATE/replies" ] || ! grep -q 'online backend paused' "$STATE/errors"; then
    echo 'panel-hn: backend failed for a reason other than the expected online pause' >&2
    exit 1
fi
printf 'panel-hn: confirmed online backend refuses to start; no site contacted\n'
