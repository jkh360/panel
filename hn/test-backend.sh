#!/bin/sh
# Requires built hn/generated, Electron, Xvfb, Node 24 and sandbox tools.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
for tool in electron Xvfb node timeout; do
    command -v "$tool" >/dev/null || { echo "panel-hn: missing $tool" >&2; exit 1; }
done
ELECTRON=$(command -v electron)
XVFB=$(command -v Xvfb)
STATE=$(mktemp -d)
trap 'rm -rf -- "$STATE"' EXIT
printf '%s\n' '{"id":1,"op":"sources"}' '{"id":2,"op":"invalid"}' |
    timeout 40 "$ROOT/hn/net-sandbox.sh" /bin/sh /app/backend/app/run.sh "$XVFB" "$ELECTRON" \
    > "$STATE/replies" 2> "$STATE/browser-errors"
node -e '
    const fs = require("node:fs");
    const assert = require("node:assert/strict");
    const replies = fs.readFileSync(process.argv[1], "utf8").trim().split("\n").map(JSON.parse);
    assert.equal(replies.length, 2);
    assert.equal(replies[0].id, 1);
    assert.ok(replies[0].sources.length > 500, "upstream catalog should be present");
    assert.ok(replies[0].sources.some(site => site.id === "mangadex"));
    assert.deepEqual(replies[1], { id: 2, error: "request failed" });
    console.log(`panel-hn: ${replies[0].sources.length} upstream connectors listed (not site-tested)`);
' "$STATE/replies"
