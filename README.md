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

On `panel-hn`, Search browses sources, titles and chapters. Press **`f`** on the
sources screen to filter by name. Press **`d`** on a chapter to fetch all pages,
or **`D`** to queue every chapter shown for that title. Complete CBZs are saved
under `Panel Downloads/<source>/<manga>/` in the current library directory.
Website access is **disabled by default**. From the `panel-hn` branch, run:

```sh
nix develop
sh hn/build.sh
PANEL_HN_ENABLE_METADATA=1 cargo run --release
```

The development shell supplies Node 24/npm 11, Electron, Xvfb, Bubblewrap,
slirp4netns, iptables and the Rust/GTK toolchain; user namespaces must also
be available. The browser
uses a fresh isolated profile and does not read your existing cookies or files.
Connectors can fail, and loaded sources are not verified websites. CBZs are
published only after every page decodes successfully; existing books are never
overwritten. A batch continues past unavailable or already-saved chapters;
Esc stops further downloads without deleting completed CBZs. See
[`hn/SECURITY.md`](hn/SECURITY.md) before opting
in. Stable `master` does not contain this backend.

GPL-3.0-only.
