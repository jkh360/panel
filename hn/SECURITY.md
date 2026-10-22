# HaruNeko backend security boundary

Source: https://github.com/manga-download/haruneko, pinned in `vendor/haruneko`.
The upstream Unlicense covers its code, **not** content served by websites.
This experimental branch has a **standalone** source-catalog, title-search,
and chapter-listing prototype, not yet wired into Panel's TUI. It has **no**
download feature. `hn/build.sh` compiles the pinned upstream engine/connectors
and Panel's private Electron bridge with Node.js 24, excluding the upstream GUI
and Electron entry. Earlier isolated tests listed 826 connectors and completed
MangaDex search and chapter listing; no other site has been verified.

**Online testing is paused as of 2026-10-21.** The compromised emSign ECC Root
CA - G3 remains in the host Node default trust store (rechecked 2026-10-22).
The bridge removes it from its own Node trust before any networking, and a
separate Chromium certificate policy rejects chains identifying that issuer
without overriding other certificate failures. These policies have **offline
unit tests only**; Chromium's real-chain behavior has not been verified.
An independent fail-closed gate still prevents the bridge from starting.
`hn/test-backend.sh` verifies this without visiting a site. Do not run live
website tests, add downloads, or remove the gate until both trust paths are
verified end-to-end. See [certifi advisory](https://github.com/advisories/GHSA-6q48-6c7w-9gw2)
and the [PHP RCE advisory](https://github.com/php/php-src/security/advisories/GHSA-w8hw-w9fx-j79w).
The PHP issue affects website servers, not Panel's client, but compromised
servers can supply hostile pages and archives.


The official Electron `Main.ts` must **not** be run unmodified as Panel's backend:
it defaults to a remotely hosted web app and disables Chromium's sandbox and
web security. Its RPC contract can import cookies from another browser.

Requirements before enabling the backend:

- Build the upstream engine/connectors locally from the pinned checkout. Do not
  execute remotely hosted scripts or fetch source code at runtime. The
  prototype uses a privileged **local** `panel-hn://app` protocol and a dedicated
  nonpersistent Electron session; it never loads the upstream remote app URL.
- Use a fresh, dedicated browser profile with no access to the user's existing
  browser profiles, cookies, credentials, or home directory. Do not import cookies.
- Disable upstream RPC, remote debugging, and unnecessary file access; expose
  only a private parent/child JSON-lines pipe to Panel. No listening network port.
  The bridge validates and limits requests, rejects browser permissions,
  and does not log connector errors (which can contain site data). Anonymous
  cross-origin fetches use a bounded IPC bridge with cookies and authorization
  stripped; connectors requiring account login or browser automation may fail.
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
