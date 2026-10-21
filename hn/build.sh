#!/bin/sh
# Build the pinned official engine locally; never run its upstream Electron app.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
UPSTREAM="$ROOT/vendor/haruneko"
PIN=5043f6959108d7215f8e0e994f156b9c5cdc653b
if [ "$(git -C "$UPSTREAM" rev-parse HEAD)" != "$PIN" ]; then
    echo 'panel-hn: refusing unreviewed HaruNeko revision' >&2
    exit 1
fi
if ! command -v node >/dev/null || ! command -v npm >/dev/null; then
    echo 'panel-hn: Node.js 24 and npm 11 are required to build upstream' >&2
    exit 1
fi
if [ "$(node -p 'process.versions.node.split(".")[0]')" -lt 24 ]; then
    echo 'panel-hn: Node.js 24 or later is required' >&2
    exit 1
fi
# Installation hooks in the upstream dependency graph are not audited for Panel.
export npm_config_ignore_scripts=true
cd "$UPSTREAM"
npm install --ignore-scripts --no-audit --no-fund
npm --workspace=web run build
# The web build verifies that the pinned engine and all its connectors compile.
# The upstream Electron app is intentionally not built or started: its current
# startup configuration does not meet Panel's security requirements.
printf 'panel-hn: compiled pinned connectors; secure browser backend is not enabled yet\n'
