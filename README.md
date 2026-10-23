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
| `s` / `/` | Experimental HaruNeko metadata search (`panel-hn` only; online gate remains closed) |
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

On `panel-hn`, Search is a read-only source/title/chapter browser. It cannot
contact sites while certificate verification is under review; it cannot
download chapters. Loaded sources are not verified websites. The stable
`master` branch does not contain the HaruNeko backend.

GPL-3.0-only.
