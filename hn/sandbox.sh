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
ASSETS="$ROOT/hn/generated"
if [ ! -f "$ASSETS/web/index.html" ] || [ ! -f "$ASSETS/app/main.js" ]; then
    echo 'panel-hn: run hn/build.sh first' >&2
    exit 1
fi
if [ "${PANEL_HN_NET_ISOLATED:-}" = 1 ]; then
    # Root is needed to set up the namespace and filesystem. A nested user
    # namespace maps that same host user to a non-root browser UID, retaining
    # access to world-readable Nix runtime libraries but no host capabilities.
    set -- "$(readlink -f "$(command -v unshare)")" \
        --map-user "${PANEL_HN_HOST_UID:?}" \
        --map-group "${PANEL_HN_HOST_GID:?}" -- "$@"
fi
# No bind of $HOME, ~/.config, SSH keys, user browser profiles, Panel's SQLite
# database, or manga library. /app is read-only; the profile lives in tmpfs.
set -- --clearenv --die-with-parent --new-session --unshare-ipc --unshare-pid \
    --proc /proc --dev /dev --tmpfs /tmp --tmpfs /home \
    --dir /app --ro-bind "$ASSETS" /app/backend \
    --ro-bind "$(readlink -f /etc/fonts)" /etc/fonts \
    --ro-bind "$(readlink -f "$(command -v sleep)")" /app/sleep \
    --setenv HOME /tmp/panel-hn \
    --setenv XDG_CONFIG_HOME /tmp/panel-hn/.config \
    --setenv XDG_CACHE_HOME /tmp/panel-hn/.cache \
    --setenv XDG_DATA_HOME /tmp/panel-hn/.local/share \
    --setenv TMPDIR /tmp \
    --setenv PATH /usr/bin:/bin \
    --setenv LANG C.UTF-8 \
    --setenv FONTCONFIG_FILE /etc/fonts/fonts.conf \
    --setenv PANEL_HN_NET_ISOLATED "${PANEL_HN_NET_ISOLATED:-0}" \
    "$@"
# Bind runtime directories only when present. Never bind an entire home or /run.
for directory in /nix/store /usr /lib /lib64 /bin /sbin; do
    if [ -d "$directory" ]; then
        set -- --ro-bind "$directory" "$directory" "$@"
    fi
done
# DNS and TLS trust, without granting access to other host configuration.
# Network namespaces must use their own resolver; never expose the host DNS IP.
if [ -n "${PANEL_HN_RESOLV:-}" ]; then
    set -- --ro-bind "$PANEL_HN_RESOLV" /etc/resolv.conf "$@"
else
    set -- --ro-bind /etc/resolv.conf /etc/resolv.conf "$@"
fi
for file in /etc/hosts /etc/nsswitch.conf /etc/ssl/certs/ca-certificates.crt; do
    if [ -f "$file" ]; then
        set -- --ro-bind "$file" "$file" "$@"
    fi
done
exec bwrap "$@"
