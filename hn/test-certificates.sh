#!/bin/sh
# No website requests: verify the real Electron and Node TLS paths against
# freshly generated local certificates in the same filesystem/browser sandbox.
# Unlike hn/net-sandbox.sh, this user/network namespace has NO external route.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ASSETS="$ROOT/hn/generated/app"
for tool in openssl certutil electron Xvfb bwrap unshare ip node timeout; do
    command -v "$tool" >/dev/null 2>&1 || { echo "panel-hn: missing $tool" >&2; exit 1; }
done
if [ ! -f "$ROOT/hn/generated/web/index.html" ] || [ ! -f "$ASSETS/main.js" ]; then
    echo 'panel-hn: build offline assets with hn/build.sh first' >&2
    exit 1
fi
STATE=$(mktemp -d)
# Never touch the published source tree with keys, and remove generated test
# assets even when a certificate verification assertion fails.
if ! mkdir "$ASSETS/certificate-fixture" 2>/dev/null; then
    echo 'panel-hn: another certificate probe appears to be running' >&2
    rm -rf -- "$STATE"
    exit 1
fi
cleanup() {
    rm -rf -- "$ASSETS/certificate-fixture" "$STATE"
    rm -f -- "$ASSETS/ca-policy.mjs" "$ASSETS/node-ca.mjs" \
        "$ASSETS/certificate-probe.cjs" "$ASSETS/certificate-probe-run.sh"
}
trap cleanup EXIT HUP INT TERM
cp "$ROOT/hn/app/ca-policy.mjs" "$ROOT/hn/app/node-ca.mjs" \
    "$ROOT/hn/app/certificate-probe.cjs" "$ROOT/hn/app/certificate-probe-run.sh" "$ASSETS/"
XVFB=$(command -v Xvfb)
ELECTRON=$(command -v electron)
CERTUTIL=$(command -v certutil)
MKDIR=$(command -v mkdir)
# NixOS's /run/current-system paths are not mounted inside bubblewrap.
case "$MKDIR" in /run/*) MKDIR=$(readlink "$MKDIR");; esac
export PANEL_HN_HOST_UID="$(id -u)" PANEL_HN_HOST_GID="$(id -g)" PANEL_HN_NET_ISOLATED=1
export ROOT XVFB ELECTRON CERTUTIL MKDIR
for mode in blocked-direct blocked-intermediate safe mismatch; do
    if [ "$mode" = safe ] || [ "$mode" = mismatch ]; then
        subject='Panel Offline Test Safe Root'
    else
        subject='emSign ECC Root CA - G3'
    fi
    openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 2 \
        -keyout "$STATE/root.key" -out "$STATE/root.crt" -subj "/CN=$subject" \
        -addext 'basicConstraints=critical,CA:TRUE' \
        -addext 'keyUsage=critical,keyCertSign,cRLSign' > /dev/null 2>&1
    openssl req -newkey rsa:2048 -sha256 -nodes -keyout "$STATE/leaf.key" \
        -out "$STATE/leaf.csr" -subj '/CN=localhost' > /dev/null 2>&1
    san=DNS:localhost,IP:127.0.0.1
    [ "$mode" != mismatch ] || san=DNS:localhost
    printf 'subjectAltName=%s\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\n' \
        "$san" > "$STATE/leaf.ext"
    if [ "$mode" = blocked-intermediate ]; then
        openssl req -newkey rsa:2048 -sha256 -nodes \
            -keyout "$STATE/int.key" -out "$STATE/int.csr" \
            -subj '/CN=Panel Synthetic Intermediate' > /dev/null 2>&1
        printf 'basicConstraints=critical,CA:TRUE,pathlen:0\nkeyUsage=critical,keyCertSign,cRLSign\n' > "$STATE/int.ext"
        openssl x509 -req -sha256 -days 2 -in "$STATE/int.csr" -CA "$STATE/root.crt" \
            -CAkey "$STATE/root.key" -CAcreateserial -out "$STATE/int.crt" \
            -extfile "$STATE/int.ext" > /dev/null 2>&1
        openssl x509 -req -sha256 -days 2 -in "$STATE/leaf.csr" -CA "$STATE/int.crt" \
            -CAkey "$STATE/int.key" -CAcreateserial -out "$STATE/leaf.crt" \
            -extfile "$STATE/leaf.ext" > /dev/null 2>&1
        cat "$STATE/int.crt" >> "$STATE/leaf.crt"
    else
        openssl x509 -req -sha256 -days 2 -in "$STATE/leaf.csr" -CA "$STATE/root.crt" \
            -CAkey "$STATE/root.key" -CAcreateserial -out "$STATE/leaf.crt" \
            -extfile "$STATE/leaf.ext" > /dev/null 2>&1
    fi
    cp "$STATE/root.crt" "$STATE/leaf.crt" "$STATE/leaf.key" "$ASSETS/certificate-fixture/"
    # Certificate verification is cached across sessions. Test each session
    # in its own Electron process so both callbacks must really fire.
    for browser_session in default backend; do
        if ! unshare --user --map-root-user --net --fork sh -c '
            set -eu
            ip link set lo up
            if [ -n "$(ip -4 route show default)" ]; then
                echo "panel-hn: probe namespace has an external route" >&2
                exit 1
            fi
            timeout 60 "$ROOT/hn/sandbox.sh" /bin/sh /app/backend/app/certificate-probe-run.sh \
                "$XVFB" "$ELECTRON" "$CERTUTIL" "$MKDIR" "$1" "$2"
        ' sh "$mode" "$browser_session" > "$STATE/output" 2> "$STATE/errors"; then
            echo "panel-hn: offline certificate probe failed ($mode, $browser_session)" >&2
            exit 1
        fi
        result=$(grep '^CERT_PROBE=' "$STATE/output" | tail -n 1 | cut -d= -f2-)
        if [ -z "$result" ]; then
            echo "panel-hn: certificate probe produced no result ($mode, $browser_session)" >&2
            exit 1
        fi
        if ! printf '%s' "$result" | node -e '
            const [mode, browserSession] = process.argv.slice(1);
            const r = JSON.parse(require("node:fs").readFileSync(0,"utf8"));
            const fail = message => { console.error(`panel-hn: ${mode}/${browserSession}: ${message}`); process.exit(1); };
            if (!r.testRootBefore || r.fetchBefore !== 200 || !Array.isArray(r.browsers) ||
                r.browsers.length !== 1 || r.browsers[0].name !== browserSession) fail("probe did not establish trust");
            const { verification, loaded } = r.browsers[0];
            if (!verification) fail("Electron did not invoke certificate verification");
            if (mode.startsWith("blocked")) {
                if (r.testRootAfter || r.fetchAfter === 200 || verification.errorCode !== 0 ||
                    verification.decision >= 0 || loaded ||
                    (mode === "blocked-intermediate" && verification.chainIssuer !== "emSign ECC Root CA - G3") ||
                    (mode === "blocked-direct" && verification.issuer !== "emSign ECC Root CA - G3")) {
                    fail("affected issuer accepted by Node or Chromium");
                }
            } else if (mode === "safe") {
                if (!r.testRootAfter || r.fetchAfter !== 200 || verification.errorCode !== 0 ||
                    verification.decision !== 0 || !loaded) fail("unaffected valid chain rejected");
            } else if (mode === "mismatch") {
                if (verification.errorCode >= 0 ||
                    verification.decision !== verification.errorCode || loaded) {
                    fail("invalid hostname was trusted");
                }
            }
        ' "$mode" "$browser_session"; then
            exit 1
        fi
        printf 'panel-hn: offline certificate probe passed: %s (%s session)\n' "$mode" "$browser_session"
    done
done
