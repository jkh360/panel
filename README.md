# Panel

A minimal, keyboard-first manga reader for Linux. Browse CBZ manga in a terminal library, then read in a native GTK window with automatic reading regions and saved progress. An optional external detector can provide better regions; no models or manga are bundled.

Run `panel` to browse from your home directory, or `panel book.cbz` to open a book directly. In the library, press `d` to make the current directory your default starting folder.

## Controls

| Library | Action |
| --- | --- |
| `j` / `k` | Move down / up |
| `l` / `Enter` | Open |
| `h` | Go back |
| `d` | Set default folder |
| `q` | Quit |

| Reader | Action |
| --- | --- |
| `j` / `k` | Next / previous region |
| `h` / `l` | Next / previous page |
| `f` / `c` / `w` | Focus / crop / whole-page mode |
| `Tab` | Cycle view modes |
| `+` / `-` | Zoom in / out |
| `q` / `Esc` | Quit |

In whole-page mode, `j` and `k` also move between pages. Page direction follows right-to-left manga reading order.

GPL-3.0-only.
