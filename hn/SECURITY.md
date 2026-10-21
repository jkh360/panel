# HaruNeko backend security boundary

Source: https://github.com/manga-download/haruneko, pinned in `vendor/haruneko`.
The upstream Unlicense covers its code, **not** content served by websites.
This experimental branch does not enable search or downloads yet. It has a
**standalone source-catalog prototype**, not yet wired into Panel's TUI.
`hn/build.sh` verifies the pinned source and compiles only the upstream engine,
connectors, and Panel's private Electron bridge with Node.js 24. It excludes
the upstream GUI and its Electron entry point. `hn/test-backend.sh` verifies
the isolated bridge can list connectors; this does NOT test any website.


The official Electron `Main.ts` must **not** be run unmodified as Panel's backend:
it defaults to a remotely hosted web app and disables Chromium's sandbox and
web security. Its RPC contract can import cookies from another browser.

Requirements before enabling the backend:

- Build the upstream engine/connectors locally from the pinned checkout. Do not
  execute remotely hosted scripts or fetch source code at runtime. The catalog
  prototype uses a privileged **local** `panel-hn://app` protocol and a dedicated
  nonpersistent Electron session; it never loads the upstream remote app URL.
- Use a fresh, dedicated browser profile with no access to the user's existing
  browser profiles, cookies, credentials, or home directory. Do not import cookies.
- Disable upstream RPC, remote debugging, and unnecessary file access; expose
  only a private parent/child JSON-lines pipe to Panel. No listening network port.
  The catalog prototype rejects unexpected commands and browser permissions.
- Sandbox pages from websites. Allow requested website network access but never
  mount Panel's SQLite database, manga library, SSH keys, or browser profiles
  into the browsing process. `hn/sandbox.sh` alone does **not** restrict network
  access; only use it through `hn/net-sandbox.sh` for a networked child. That
  launcher uses a separate user/network namespace, slirp with host loopback
  disabled, and pre-launch firewall rules denying private IPv4 destinations
  except its DNS resolver. IPv6 has no external route. The browser runs as a
  non-root user under an isolated Xvfb, not on the user's desktop display.
  Its network policy is experimental, not a guarantee that site scripts are safe.
- Confirm downloaded data are images, bound their size and count, write CBZ
  files atomically, and prevent traversal or overwrite through remote filenames.
- Run only when a user explicitly opens Search. Keep the reader offline.

Website connectors may require further privileges or stop working. A connector
that cannot run inside this boundary should fail visibly, not silently weaken
these protections.
