#!/bin/sh
# Run with Node 24, bubblewrap, slirp4netns, unshare and iptables installed.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
NODE=$(command -v node)
umask 077
STATE=$(mktemp -d)
server=
cleanup() {
    [ -z "$server" ] || kill "$server" 2>/dev/null || true
    [ -z "$server" ] || wait "$server" 2>/dev/null || true
    rm -rf -- "$STATE"
}
trap cleanup EXIT
# A *live* host service makes the loopback test meaningful: a refused port
# could otherwise look identical to a successful firewall rule.
"$NODE" -e '
    const http = require("node:http");
    const fs = require("node:fs");
    const server = http.createServer((_req, res) => res.end("host-secret"));
    server.listen(0, "0.0.0.0", () => fs.writeFileSync(process.argv[1], String(server.address().port)));
' "$STATE/port" &
server=$!
i=0
while [ ! -s "$STATE/port" ]; do
    if ! kill -0 "$server" 2>/dev/null || [ "$i" -ge 100 ]; then
        echo 'panel-hn: test server failed to start' >&2
        exit 1
    fi
    i=$((i + 1))
    sleep 0.05
done
# When the host has an RFC1918 address, verify that same listening service is
# unreachable at both its loopback and LAN addresses from inside the sandbox.
private_ip=$("$NODE" -e '
    const os = require("node:os");
    for (const addresses of Object.values(os.networkInterfaces())) {
        const address = addresses?.find(a => a.family === "IPv4" && !a.internal &&
            /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(a.address));
        if (address) { console.log(address.address); break; }
    }
')
export PANEL_HN_SECRET_TEST=must_not_cross_boundary
"$ROOT/hn/net-sandbox.sh" "$NODE" -e '
    const fs = require("node:fs");
    const assert = require("node:assert/strict");
    (async () => {
        assert.equal(process.env.PANEL_HN_SECRET_TEST, undefined);
        assert.deepEqual(fs.readdirSync("/home"), []);
        assert.equal(process.env.HOME, "/tmp/panel-hn");
        assert.notEqual(process.getuid(), 0);
        assert.equal(fs.existsSync("/app/backend/web/index.html"), true);
        assert.equal(process.env.PANEL_HN_NET_ISOLATED, "1");
        assert.match(fs.readFileSync("/etc/resolv.conf", "utf8"), /10\.0\.2\.3/);
        const [port, ip] = process.argv.slice(1);
        const urls = [`http://127.0.0.1:${port}/`];
        if (ip) urls.push(`http://${ip}:${port}/`);
        for (const url of urls) {
            const controller = new AbortController();
            const timeout = setTimeout(() => controller.abort(), 1500);
            try {
                await fetch(url, { signal: controller.signal });
                throw new Error("host service was reachable: " + url);
            } catch (error) {
                if (error.message?.includes("host service was reachable")) throw error;
            } finally { clearTimeout(timeout); }
        }
        console.log("panel-hn: secrets hidden; live host loopback/LAN service unreachable");
    })().catch(error => { console.error(error); process.exitCode = 1; });
' "$(head -n 1 "$STATE/port")" "$private_ip"
