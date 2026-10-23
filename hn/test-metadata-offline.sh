#!/bin/sh
# Exercise the opted-in production bridge without an external route or website.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
for tool in electron Xvfb bwrap unshare ip node timeout; do
    command -v "$tool" >/dev/null 2>&1 || { echo "panel-hn: missing $tool" >&2; exit 1; }
done
STATE=$(mktemp -d)
trap 'rm -rf -- "$STATE"' EXIT
export ROOT XVFB="$(command -v Xvfb)" ELECTRON="$(command -v electron)" \
    PANEL_HN_HOST_UID="$(id -u)" PANEL_HN_HOST_GID="$(id -g)" PANEL_HN_NET_ISOLATED=1 \
    PANEL_HN_ENABLE_METADATA=1
if ! printf '%s\n' '{"id":1,"op":"sources-panel"}' | \
    unshare --user --map-root-user --net --fork sh -c '
        set -eu
        ip link set lo up
        if [ -n "$(ip -4 route show default)" ]; then
            echo "panel-hn: offline metadata probe has an external route" >&2
            exit 1
        fi
        timeout 45 "$ROOT/hn/sandbox.sh" /bin/sh /app/backend/app/run.sh "$XVFB" "$ELECTRON"
    ' > "$STATE/replies" 2> "$STATE/errors"; then
    echo 'panel-hn: offline metadata startup failed' >&2
    exit 1
fi
node -e '
    const fs = require("node:fs");
    const lines = fs.readFileSync(process.argv[1], "utf8").trimEnd().split("\n");
    const items = lines.filter(line => /^1\titem\t[A-Za-z0-9+/=]+\t[A-Za-z0-9+/=]+$/.test(line));
    if (items.length < 100 || lines.length !== items.length + 1 || lines.at(-1) !== "1\tend") {
        console.error("panel-hn: offline metadata reply invalid");
        process.exit(1);
    }
    console.log(`panel-hn: ${items.length} connector definitions listed offline; no site contacted`);
' "$STATE/replies"
