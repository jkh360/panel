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
# Do not fetch dependencies through a TLS trust store containing the emSign G3
# root affected by compromised intermediate keys (GHSA-6q48-6c7w-9gw2).
# Local recompilation using already installed dependencies remains possible.
export npm_config_ignore_scripts=true
cd "$UPSTREAM"
if [ ! -x ./node_modules/.bin/vite ]; then
    if ! node -e 'const t=require("node:tls"),c=require("node:crypto"); process.exit(t.getCACertificates("default").some(p=>/CN=emSign ECC Root CA - G3\b/.test(new c.X509Certificate(p).subject))?1:0)' ; then
        echo 'panel-hn: refusing npm download with compromised CA root trusted' >&2
        exit 1
    fi
    npm install --ignore-scripts --no-audit --no-fund
fi
./node_modules/.bin/vite build --config ../../hn/web/vite.config.mjs --configLoader runner
./node_modules/.bin/vite build --config ../../hn/app/vite.config.mjs --configLoader runner
cp "$ROOT/hn/app/package.json" "$ROOT/hn/app/run.sh" "$ROOT/hn/generated/app/"
# Only the pinned engine/connectors and Panel's custom Electron entry are built.
# The upstream GUI, remote app entry point and RPC server are excluded.
printf 'panel-hn: built headless engine and local Electron bridge\n'
