# Panel

A minimal, keyboard-first manga reader for Linux. Browse CBZ manga in a terminal library, then read in a native GTK window with automatic reading regions and saved progress. An optional external detector can provide better regions; no models or manga are bundled.

Run `panel` to browse from your home directory, or `panel book.cbz` to open a book directly. In the library, press `d` to make the current directory your default starting folder. A series' **Last read** entry resumes the most recently opened volume at its saved chapter and page; volumes also have their own **Last read** entries.

## Controls

| Library | Action |
| --- | --- |
| `j` / `k` | Move down / up |
| `l` / `Enter` | Open |
| `h` | Go back |
| `d` | Set default folder |
| `s` / `/` | Experimental HaruNeko metadata search (`panel-hn` only; explicit opt-in) |
| `q` | Quit |

| Reader | Action |
| --- | --- |
| `j` / `k` | Next / previous region |
| `h` / `l` | Next / previous page |
| `f` / `c` / `w` | Focus / crop / whole-page mode |
| `Tab` | Cycle view modes |
| `+` / `-` | Zoom in / out |
| `Esc` | Return to the terminal library |
| `q` | Quit Panel entirely |

In whole-page mode, `j` and `k` also move between pages. Page direction follows right-to-left manga reading order.

On `panel-hn`, Search is a read-only source/title/chapter browser. Website
access is **disabled by default**; after building with `sh hn/build.sh`, opt in
with `PANEL_HN_ENABLE_METADATA=1 cargo run --release` (Electron, Xvfb,
Bubblewrap, slirp4netns, iptables and user namespaces required). The browser
uses a fresh isolated profile and does not read your existing cookies or files.
Connectors can fail, and loaded sources are not verified websites. There are
**no chapter downloads**. See [`hn/SECURITY.md`](hn/SECURITY.md) before opting
in. Stable `master` does not contain this backend.

GPL-3.0-only.
