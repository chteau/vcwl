# Benchmarks

Numbers on this page are measurements on one machine, not general claims. Re-run them before
drawing conclusions.

## Keyed rows (`bench/`)

The same four actions on a list of 1000 rows, implemented in VCWL (`bench/rows`) and as
hand-written DOM code specialised for each action (`bench/vanilla/index.html`). 5 warm-up rounds
are discarded; the table shows the median of 25 rounds.

```sh
cargo build && (cd tests/browser && npm ci) && node bench/run.mjs
```

Two timings per action:

- **sync**: from the click to the end of the synchronous work. For VCWL this is everything: event
  handling in the renderer, the app update, style, layout, building the display list and the canvas
  drawing calls. For the DOM version it is the DOM update and a forced style/layout; the browser
  paints later, outside the measurement. The DOM column therefore leaves out work the VCWL column
  includes.
- **frame**: from the click until the next frame has been produced (`requestAnimationFrame` then a
  task). This includes the browser's rendering for both, but is quantized by frame scheduling.
- **settled**: like frame, but also waits for VCWL's accessibility mirror, which is synced in a
  task after the frame (`vcwl.mirrored()`). For the DOM version it equals frame.

Measured 2026-10-22; Linux, Chrome 151 headless, Node 26, median of 25 rounds:

| action | VCWL sync | DOM sync | VCWL frame | DOM frame | VCWL settled | DOM settled |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| create 1000 rows | 6.4 | 7.3 | 24.7 | 25.1 | 34.0 | 25.3 |
| update every 10th row | 0.8 | 0.6 | 5.9 | 17.6 | 14.1 | 17.6 |
| swap two rows | 0.8 | 0.3 | 16.5 | 17.4 | 17.6 | 17.3 |
| clear | 0.9 | 0.8 | 16.9 | 16.5 | 17.7 | 16.5 |

Timer resolution in the page is 0.1 ms. Rows below the viewport are laid out but not painted
(list bullets included, since Milestone 9: before, each frame drew all 1000 bullets).

Frame numbers are quantized by the 60 Hz frame clock (~16.7 ms), so differences under one frame
mostly reflect where in the frame the click landed. The settled column shows the remaining cost:
creating 1000 rows still takes about 9 ms more than the DOM version, mostly the mirror's 2000+
DOM elements.

Changes in Milestone 9, each measured with this benchmark before being kept:

- the mirror is synced in a task after the frame instead of inside it (frame create 38.1 → 26);
- mirror elements take their shared style from one constructed style sheet, are cloned from
  per-tag prototypes, and an element whose content is one text run carries the text itself
  (mirror script time on create 13.4 → 4.2 ms);
- one canvas context per font for measuring, so the font string is not re-parsed per call;
- a paint state cache that skips redundant `fillStyle`/`font` assignments;
- offscreen list markers are culled (update sync 1.3 → 0.8, swap 1.2 → 0.8).

History of the same benchmark (sync column):

| version | create | update | swap | clear |
| --- | ---: | ---: | ---: | ---: |
| DOM host, greedy keyed moves | 7.7 | 0.6 | 7.1 | 1.1 |
| DOM host, LIS keyed moves (2026-10-21) | 7.7 | 0.6 | 0.3 | 1.1 |
| canvas renderer, full restyle/relayout/repaint | 10.5 | 4.9 | 4.6 | 0.7 |
| canvas renderer, incremental | 6.9 | 1.0 | 1.0 | 0.8 |
| + accessibility mirror updated during events (first version) | 21.9 | 6.4 | 17.3 | 6.1 |
| + mirror after paint, keyed moves, entry hashes | 7.0 | 1.1 | 1.1 | 0.8 |
| + deferred mirror, marker culling, `opt-level = "s"` (current) | 6.4 | 0.8 | 0.8 | 0.9 |

What this shows: with incremental style and layout and viewport culling, the canvas renderer's
complete pipeline stays within a small factor of the browser's DOM update for these operations.
It does not compare against UI frameworks, larger or text-heavy pages, memory use or start-up time
(see below).

## Start-up

`node bench/startup.mjs` builds `examples/tokenforge` and loads it in headless Chrome, locally
and throttled to 40 ms latency / 20 Mbps, reporting first contentful paint (the prerendered HTML)
and the first canvas frame (`vcwl:first-frame` performance mark). Median of 7 loads with an empty cache, ranges over several
runs:

| | first contentful paint | first canvas frame |
| --- | ---: | ---: |
| local, prerendered | 36 ms | 42–59 ms |
| local, not prerendered | — | 56 ms |
| 40 ms / 20 Mbps, prerendered | 64–92 ms | 287–304 ms |
| 40 ms / 20 Mbps, not prerendered | — | 288 ms |
| 40 ms / 20 Mbps, before preload hints | — | 373 ms |

Without prerendering nothing is painted before the canvas (the page is blank). `index.html`
preloads `vcwl.js`, `render.wasm` and `app.wasm` so they download in parallel.

`render.wasm` optimization level, measured on the same machine:

| opt-level | raw / gzip | create (sync) | slow-link first canvas frame |
| --- | ---: | ---: | ---: |
| `"z"` | 255,719 / ~114K | ~7.3 ms | 286 ms |
| `"s"` (chosen) | 303,161 / 129,815 | 6.5 ms | 303 ms |
| `3` | 387,501 / 150,717 | 6.0 ms | 341 ms |

## Output size

`vcwl build` (bytes, raw / gzip -9):

| file | todo example | rows benchmark |
| --- | ---: | ---: |
| app.wasm (includes styles) | 17,291 / 6,598 | 5,740 / 2,651 |
| render.wasm (same for every app) | 311,430 / 132,961 | 311,430 / 132,961 |
| vcwl.js (host) | 39,560 / 12,781 | 39,560 / 12,781 |
| index.html + boot.js | 622 / 474 | 623 / 477 |

`render.wasm` is mostly Rust's standard library (allocator, formatting, Unicode case mapping,
hashing) and the renderer's own style, layout and paint code; it is built with `opt-level = "s"`,
LTO and stripped names. For comparison, the DOM version of the todo example shipped 15,962 / 6,021
(app.wasm) + 14,768 / 5,204 (vcwl.js) + 1,907 / 872 (app.css). The tokenforge site remake
(`examples/tokenforge`, seven routes) compiles to an `app.wasm` of 152,363 / 46,569, and its
prerendered `index.html` is 100,561 / 21,576.
