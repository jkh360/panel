#!/bin/sh
# Run the experimental browser backend without the host home or browser data.
# This is not enabled by Panel until the headless backend has passed review.
set -eu
if [ "$#" -lt 1 ]; then
    echo 'usage: hn/sandbox.sh COMMAND [ARG...]' >&2
    exit 2
fi
if ! command -v bwrap >/dev/null 2>&1; then
    echo 'panel-hn: bubblewrap is required for the browser backend' >&2
    exit 1
fi
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ASSETS="$ROOT/vendor/haruneko/web/build"
if [ ! -f "$ASSETS/index.html" ]; then
    echo 'panel-hn: run hn/build.sh first' >&2
    exit 1
fi
# No bind of $HOME, ~/.config, SSH keys, user browser profiles, Panel's SQLite
# database, or manga library. /app is read-only; the profile lives in tmpfs.
set -- --clearenv --die-with-parent --new-session --unshare-ipc --unshare-pid \
    --proc /proc --dev /dev --tmpfs /tmp --tmpfs /home \
    --dir /app --ro-bind "$ASSETS" /app/web \
    --setenv HOME /home/panel-hn \
    --setenv XDG_CONFIG_HOME /home/panel-hn/.config \
    --setenv XDG_CACHE_HOME /home/panel-hn/.cache \
    --setenv XDG_DATA_HOME /home/panel-hn/.local/share \
    --setenv TMPDIR /tmp \
    --setenv PATH /usr/bin:/bin \
    --setenv LANG C.UTF-8 \
    "$@"
# Bind runtime directories only when present. Never bind an entire home or /run.
for directory in /nix/store /usr /lib /lib64 /bin /sbin; do
    if [ -d "$directory" ]; then
        set -- --ro-bind "$directory" "$directory" "$@"
    fi
done
# DNS and TLS trust, without granting access to other host configuration.
for file in /etc/resolv.conf /etc/hosts /etc/nsswitch.conf /etc/ssl/certs/ca-certificates.crt; do
    if [ -f "$file" ]; then
        set -- --ro-bind "$file" "$file" "$@"
    fi
done
exec bwrap "$@"
