#!/bin/sh
# Real Electron IPC/navigation behavior without website or host-service access.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ASSETS="$ROOT/hn/generated/app"
for tool in electron Xvfb bwrap unshare ip node timeout; do
    command -v "$tool" >/dev/null 2>&1 || { echo "panel-hn: missing $tool" >&2; exit 1; }
done
if [ ! -f "$ROOT/hn/generated/web/index.html" ] || [ ! -f "$ASSETS/main.js" ]; then
    echo 'panel-hn: build offline assets with hn/build.sh first' >&2
    exit 1
fi
STATE=$(mktemp -d)
if ! mkdir "$ASSETS/boundary-fixture" 2>/dev/null; then
    echo 'panel-hn: another boundary probe appears to be running' >&2
    rm -rf -- "$STATE"
    exit 1
fi
cleanup() {
    rm -rf -- "$ASSETS/boundary-fixture" "$STATE"
    rm -f -- "$ASSETS/ipc-policy.mjs" "$ASSETS/net-policy.mjs" \
        "$ASSETS/boundary-preload.cjs" "$ASSETS/boundary-probe.cjs" "$ASSETS/boundary-probe-run.sh"
}
trap cleanup EXIT HUP INT TERM
cp "$ROOT/hn/app/ipc-policy.mjs" "$ROOT/hn/app/net-policy.mjs" \
    "$ROOT/hn/app/boundary-preload.cjs" "$ROOT/hn/app/boundary-probe.cjs" \
    "$ROOT/hn/app/boundary-probe-run.sh" "$ASSETS/"
export ROOT XVFB="$(command -v Xvfb)" ELECTRON="$(command -v electron)" \
    PANEL_HN_HOST_UID="$(id -u)" PANEL_HN_HOST_GID="$(id -g)" PANEL_HN_NET_ISOLATED=1
if ! unshare --user --map-root-user --net --fork sh -c '
    set -eu
    ip link set lo up
    if [ -n "$(ip -4 route show default)" ]; then
        echo "panel-hn: probe namespace has an external route" >&2
        exit 1
    fi
    timeout 45 "$ROOT/hn/sandbox.sh" /bin/sh /app/backend/app/boundary-probe-run.sh "$XVFB" "$ELECTRON"
' > "$STATE/output" 2> "$STATE/errors"; then
    echo 'panel-hn: isolated boundary probe failed' >&2
    exit 1
fi
result=$(grep '^BOUNDARY_PROBE=' "$STATE/output" | tail -n 1 | cut -d= -f2-)
if [ -z "$result" ]; then
    echo 'panel-hn: isolated boundary probe produced no result' >&2
    exit 1
fi
printf '%s' "$result" | node -e '
    const r = JSON.parse(require("node:fs").readFileSync(0,"utf8"));
    if (r.main !== "allowed" || !String(r.subframe).includes("untrusted IPC sender") ||
        !r.safeLoaded || r.crossLoaded || !r.crossNavigatePrevented ||
        r.hits.safe !== 1 || r.hits.cross !== 0 ||
        !r.events.some(event => event.name === "will-redirect" &&
            event.url.endsWith("/cross") && event.prevented === true)) {
        console.error("panel-hn: isolated boundary check failed:", r);
        process.exit(1);
    }
'
printf 'panel-hn: offline Electron IPC and redirect boundaries passed; no site contacted\n'
