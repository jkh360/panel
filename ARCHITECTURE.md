# Architecture

Panel is a local GTK application. It has no account system, network service,
scraper, daemon, embedded browser, or runtime plugin framework.

## Components

- `document`: lazy CBZ indexing, image decoding, and GTK texture conversion
- `page_view`: rendering and focus/crop/page transforms
- `region`: conservative built-in gutter analysis and reading order
- `detector`: optional persistent external detector process
- `detection_cache`: SQLite persistence for built-in and external regions
- `progress`: document position and completion state
- `import_regions`: offline prediction import
- `main`: CLI dispatch, reader state, workers, and GTK input

## Concurrency

GTK owns rendering and input on its main thread. A bounded native worker handles
archive reads, image decoding, and built-in analysis. If configured, an isolated
persistent detector process runs behind a second Rust worker. Navigation can
reprioritize its queue. It processes the current page, remaining pages ahead
in the current volume, then later volumes of the same manga; earlier pages in
the current volume come last. Results are cached against each CBZ separately.
Only detections for the open book can replace visible regions; its decoded
texture remains unchanged. Panel ordering recursively groups regions across
page-wide horizontal gutters (top first), then vertical gutters (right first).
Irregular overlapping layouts use a deterministic fallback. Cached regions
are reordered when loaded, without rerunning the external detector.

## Detector boundary

The external detector API is a line-oriented stdin/stdout protocol rather than
a linked model runtime. This keeps Panel independent of model frameworks,
licenses, accelerators, and language runtimes. The reader validates, orders,
and caches all returned source-coordinate rectangles.

## Persistence

SQLite lives under the XDG state directory. Document identity consists of its
canonical path, size, and modification time. Page dimensions are also part of a
cached detection's identity, so changed files and mismatched results are not
silently reused. External regions take priority over built-in results.

## Performance rules

- Never unpack a complete book.
- Decode reader pages on demand; external detection may additionally decode
  pages queued ahead, but never unpack an entire book.
- Keep blocking work off the GTK thread.
- Analyze reduced-resolution images where possible.
- Keep region coordinates integral and relative to the source image.
- Fall back to a whole page instead of presenting uncertain crops.

## Input roadmap

1. CBZ containing JPEG, PNG, or WebP
2. Image directories
3. PDF through a system rendering library
