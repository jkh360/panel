#!/bin/sh
# Run with Node 24, bubblewrap, slirp4netns, unshare and iptables installed.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
NODE=$(command -v node)
export PANEL_HN_SECRET_TEST=must_not_cross_boundary
"$ROOT/hn/net-sandbox.sh" "$NODE" -e '
    const fs = require("node:fs");
    const assert = require("node:assert/strict");
    (async () => {
        assert.equal(process.env.PANEL_HN_SECRET_TEST, undefined);
        assert.deepEqual(fs.readdirSync("/home"), []);
        assert.equal(process.env.HOME, "/home/panel-hn");
        assert.equal(fs.existsSync("/app/web/index.html"), true);
        assert.match(fs.readFileSync("/etc/resolv.conf", "utf8"), /10\.0\.2\.3/);
        for (const url of ["http://127.0.0.1:8765/", "http://192.168.1.1/"]) {
            const controller = new AbortController();
            const timeout = setTimeout(() => controller.abort(), 1500);
            try {
                await fetch(url, { signal: controller.signal });
                throw new Error("private network address was reachable: " + url);
            } catch (error) {
                if (error.message?.includes("private network address was reachable")) throw error;
            } finally { clearTimeout(timeout); }
        }
        console.log("panel-hn: secrets hidden; loopback and LAN destinations denied");
    })().catch(error => { console.error(error); process.exitCode = 1; });
'
