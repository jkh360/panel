# HaruNeko backend security boundary

Source: https://github.com/manga-download/haruneko, pinned in `vendor/haruneko`.
The upstream Unlicense covers its code, **not** content served by websites.
This experimental branch wires a metadata-only source-catalog, title-search,
and chapter-listing prototype to the TUI (`s` or `/` in the library). Website
access is disabled by default and requires `PANEL_HN_ENABLE_METADATA=1` plus
an explicit Search action. `d` requests a complete chapter CBZ; `D` queues
all listed chapters in that title (up to the 2,000 metadata cap). Browsing alone
never starts a download. `hn/build.sh` compiles
the pinned upstream engine/connectors and Panel's private Electron bridge with
Node.js 24, excluding the upstream GUI and Electron entry. Listing 826
connectors does not establish that their websites work.

**Opt-in metadata access only.** The affected emSign ECC Root CA - G3
is still in the host Node default trust store. The bridge removes it from its
own Node trust before any networking. Its separate Chromium policy rejects
chains identifying that issuer without overriding other certificate failures.

On 2026-10-23, `hn/test-certificates.sh` exercised the **real Electron/Node
TLS paths inside the production filesystem/browser sandbox**, in a network
namespace with loopback but **no external route**. Fresh, synthetic X.509
chains bearing the affected root's name were deliberately trusted only in a
temporary test profile. A fresh-origin Node `fetch` rejected them after the
bridge's root removal; both Electron sessions reported a valid original chain
and rejected the direct and intermediate-signed cases. An unaffected trusted
chain loaded; a hostname mismatch remained rejected. This verifies the
certificate-policy mechanism end-to-end without visiting a website or
installing any CA into the host. **It does not test the real compromised
intermediate certificates or establish that website content is safe.**

The default fail-closed gate refuses online startup; `hn/test-backend.sh`
verifies this without visiting a site. The offline review removed upstream's
cookie-reading fetch controller, limits the private fetch bridge to anonymous
requests with manual redirects, and restricts remote-window redirects and
navigations to the requested origin. Private URL literals and unsafe extra
headers are rejected or removed. The namespace firewall remains essential
for DNS names resolving to private IPs. Remote windows are capped at three;
loads and scripts have deadlines, and script results are rejected above 2 MB
after Electron deserializes them. `hn/test-boundaries.sh` exercised actual
Electron IPC sender frames, same-origin redirects and blocked cross-origin
redirects/navigations without an external route. `hn/test-metadata-offline.sh`
started the opted-in bridge without a route and listed 826 connector definitions.
On 2026-10-23, the **actual TUI** was driven through MangaDex source selection,
a live title search, a seven-chapter metadata listing and return to the library.
Separate isolated live requests confirmed official MANGA Plus by Shueisha
returned 32 matching titles and two chapter entries for each of two tested
titles. Some MangaDex titles returned zero chapters, which the TUI reports as
"No results"; that alone does not show whether the website or connector has
changed. Those October checks requested no images. These two connectors are
not representative of the other 824; associated hosts may be contacted by each
connector. Renderer memory and oversized data before IPC deserialization remain
limitations. Chapter downloads stream one bounded page at a time through the
private pipe; only Panel (outside the backend sandbox) writes files, under the
current library folder's `Panel Downloads/` directory. Panel checks decoded
JPEG/PNG/WebP images, limits pages and bytes, and publishes a CBZ only after all
pages pass. A failed or cancelled chapter leaves no final CBZ; completed
chapters in a cancelled batch remain available, and existing CBZs are never
overwritten. The writer is tested with synthetic images. On 2026-11-24, the
**official MANGA Plus** connector saved a complete 23-page chapter through
the actual TUI. A subsequent `D` batch saved three valid CBZs with 23, 25 and
55 ordered pages. ZIP integrity was checked and all test output was removed.
Panel's anonymous adapter skips upstream's MANGA Plus login-token lookup in a
remote window: the isolated profile has no login token and its public API
works without one. This does not bypass authentication or certificate checks.
Some other official chapters were unavailable; the remaining connectors are
not yet verified for downloads. The TUI starts the backend only on explicit
Search, sanitizes replies, and cancels the child on exit. See [certifi advisory](https://github.com/advisories/GHSA-6q48-6c7w-9gw2)
and the [PHP RCE advisory](https://github.com/php/php-src/security/advisories/GHSA-w8hw-w9fx-j79w).
The PHP issue affects website servers, not Panel's client, but compromised
servers can supply hostile pages and archives.


The official Electron `Main.ts` must **not** be run unmodified as Panel's backend:
it defaults to a remotely hosted web app and disables Chromium's sandbox and
web security. Its RPC contract can import cookies from another browser.

Security boundary for the opt-in backend:

- Build the upstream engine/connectors locally from the pinned checkout. Do not
  execute remotely hosted scripts or fetch source code at runtime. The
  prototype uses a privileged **local** `panel-hn://app` protocol and a dedicated
  nonpersistent Electron session; it never loads the upstream remote app URL.
- Use a fresh, dedicated browser profile with no access to the user's existing
  browser profiles, cookies, credentials, or home directory. Do not import cookies.
- Disable upstream RPC, remote debugging, and unnecessary file access; expose
  only a private parent/child JSON-lines pipe to Panel. No listening network port.
  The bridge validates and limits requests, accepts IPC only from its local
  top-level renderer (not untrusted iframes), rejects browser permissions, and
  does not log connector errors (which can contain site data). Upstream's
  cookie-reading FetchProvider is replaced with empty cookie responses;
  no browser cookie jar is queried. Anonymous cross-origin fetches use a
  bounded IPC bridge with cookies and authorization stripped. The stdin pipe
  accepts only bounded, encoded source/title/chapter metadata operations.
  Remote websites may set temporary cookies in the disposable isolated profile,
  but existing host cookies are never imported. Connectors requiring account
  login or browser automation may fail.
- Sandbox pages from websites. Allow requested website network access but never
  mount Panel's SQLite database, manga library, SSH keys, or browser profiles
  into the browsing process. `hn/sandbox.sh` alone does **not** restrict network
  access; only use it through `hn/net-sandbox.sh` for a networked child. That
  launcher uses a separate user/network namespace, slirp with host loopback
  disabled, and pre-launch firewall rules denying private IPv4 destinations
  except its DNS resolver. IPv6 has no external route. The browser runs as a
  non-root user under an isolated Xvfb, not on the user's desktop display.
  Its network policy is experimental, not a guarantee that site scripts are safe.
- Downloaded data must decode as JPEG, PNG or WebP; each page is capped at
  12 MB, each chapter at 300 pages / 512 MB, and dimensions are bounded.
  Site-supplied filenames are never used as archive paths. Output names are
  sanitized, and the resulting CBZ is published atomically without replacing
  existing files. The isolated browser has no mount of the destination.
- Run only when a user explicitly opens Search. Keep the reader offline.

Website connectors may require further privileges or stop working. A connector
that cannot run inside this boundary should fail visibly, not silently weaken
these protections.
