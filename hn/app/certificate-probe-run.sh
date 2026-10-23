#!/bin/sh
# Only invoked by hn/test-certificates.sh inside hn/sandbox.sh.
set -eu
xvfb=$1 electron=$2 certutil=$3 mkdir_bin=$4 mode=$5 browser_session=$6
"$mkdir_bin" -p /tmp/panel-hn/.pki/nssdb /tmp/panel-ca-profile
"$certutil" -N -d sql:/tmp/panel-hn/.pki/nssdb --empty-password
"$certutil" -A -d sql:/tmp/panel-hn/.pki/nssdb -n panel-test-ca -t 'C,,' -i /app/backend/app/certificate-fixture/root.crt
"$xvfb" :77 -nolisten tcp -screen 0 800x600x24 < /dev/null > /dev/null 2>&1 &
xpid=$!
trap 'kill "$xpid" 2>/dev/null || true' EXIT
n=0
while [ ! -S /tmp/.X11-unix/X77 ]; do
    if [ "$n" -ge 100 ]; then echo 'certificate probe: Xvfb did not start' >&2; exit 1; fi
    n=$((n + 1))
    /app/sleep 0.05
done
export DISPLAY=:77
if [ "$mode" = mismatch ] && [ "$browser_session" = backend ]; then
    "$electron" /app/backend/app/certificate-probe.cjs --mismatch --backend-session
elif [ "$mode" = mismatch ]; then
    "$electron" /app/backend/app/certificate-probe.cjs --mismatch
elif [ "$browser_session" = backend ]; then
    "$electron" /app/backend/app/certificate-probe.cjs --backend-session
else
    "$electron" /app/backend/app/certificate-probe.cjs
fi
