#!/bin/sh
# Experimental: outbound internet without access to host loopback or private IPs.
# The child also runs under hn/sandbox.sh (isolated filesystem and environment).
set -eu
if [ "$#" -lt 1 ]; then
    echo 'usage: hn/net-sandbox.sh COMMAND [ARG...]' >&2
    exit 2
fi
for tool in unshare slirp4netns iptables bwrap; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "panel-hn: $tool is required for network isolation" >&2
        exit 1
    fi
done
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
umask 077
STATE=$(mktemp -d)
printf 'nameserver 10.0.2.3\noptions attempts:1 timeout:3\n' > "$STATE/resolv.conf"
export PANEL_HN_RESOLV="$STATE/resolv.conf"
worker= slirp=
cleanup() {
    [ -z "$slirp" ] || kill "$slirp" 2>/dev/null || true
    [ -z "$worker" ] || kill "$worker" 2>/dev/null || true
    [ -z "$slirp" ] || wait "$slirp" 2>/dev/null || true
    rm -rf -- "$STATE"
}
trap cleanup EXIT HUP INT TERM
# Inside the new user+network namespace, set policy BEFORE starting the child.
# DNS to slirp's virtual resolver is the sole exception in its private subnet.
unshare --user --map-root-user --net --fork sh -c '
    set -eu
    printf "%s\n" "$$" > "$1/pid"
    while [ ! -f "$1/ready" ]; do sleep 0.05; done
    iptables -A OUTPUT -d 10.0.2.3 -p udp --dport 53 -j ACCEPT
    iptables -A OUTPUT -d 10.0.2.3 -p tcp --dport 53 -j ACCEPT
    for subnet in 0.0.0.0/8 10.0.0.0/8 100.64.0.0/10 127.0.0.0/8 \
        169.254.0.0/16 172.16.0.0/12 192.168.0.0/16 198.18.0.0/15 \
        224.0.0.0/4 240.0.0.0/4; do
        iptables -A OUTPUT -d "$subnet" -j REJECT
    done
    shift
    exec "$@"
' sh "$STATE" "$ROOT/hn/sandbox.sh" "$@" &
worker=$!
# Slirp attaches only to the child's new net namespace; it has no host ports.
n=0
while [ ! -s "$STATE/pid" ]; do
    if ! kill -0 "$worker" 2>/dev/null || [ "$n" -ge 200 ]; then
        echo 'panel-hn: network namespace did not start' >&2
        exit 1
    fi
    n=$((n + 1))
    sleep 0.05
done
netpid=$(head -n 1 "$STATE/pid")
slirp4netns --configure --disable-host-loopback "$netpid" tap0 > "$STATE/slirp.log" 2>&1 &
slirp=$!
n=0
while ! grep -q '^tap0[[:space:]]' "/proc/$netpid/net/route" 2>/dev/null; do
    if ! kill -0 "$slirp" 2>/dev/null || [ "$n" -ge 200 ]; then
        echo 'panel-hn: isolated network did not become ready' >&2
        grep . "$STATE/slirp.log" >&2 || true
        exit 1
    fi
    n=$((n + 1))
    sleep 0.05
done
: > "$STATE/ready"
wait "$worker"
