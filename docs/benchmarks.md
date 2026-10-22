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

Measured 2026-10-22; Linux, Chrome 151 headless, Node 26, VCWL canvas renderer:

| action | VCWL sync | DOM sync | VCWL frame | DOM frame |
| --- | ---: | ---: | ---: | ---: |
| create 1000 rows | 6.9 | 6.4 | 11.2 | 15.4 |
| update every 10th row | 1.0 | 0.5 | 11.7 | 9.2 |
| swap two rows | 1.0 | 0.2 | 16.4 | 16.0 |
| clear | 0.8 | 0.6 | 15.3 | 15.7 |

Timer resolution in the page is 0.1 ms. Rows below the viewport are laid out but not painted.

History of the same benchmark (sync column):

| version | create | update | swap | clear |
| --- | ---: | ---: | ---: | ---: |
| DOM host, greedy keyed moves | 7.7 | 0.6 | 7.1 | 1.1 |
| DOM host, LIS keyed moves (2026-10-21) | 7.7 | 0.6 | 0.3 | 1.1 |
| canvas renderer, full restyle/relayout/repaint | 10.5 | 4.9 | 4.6 | 0.7 |
| canvas renderer, incremental (current) | 6.9 | 1.0 | 1.0 | 0.8 |

What this shows: with incremental style and layout and viewport culling, the canvas renderer's
complete pipeline stays within a small factor of the browser's DOM update for these operations.
It does not compare against UI frameworks, larger or text-heavy pages, memory use or start-up time
(which now includes compiling `render.wasm`).

## Output size

`vcwl build` (bytes, raw / gzip -9):

| file | todo example | rows benchmark |
| --- | ---: | ---: |
| app.wasm (includes styles) | 17,276 / 6,589 | 5,729 / 2,645 |
| render.wasm (same for every app) | 236,790 / 106,659 | 236,790 / 106,659 |
| vcwl.js (host) | 23,854 / 7,797 | 23,854 / 7,797 |
| index.html + boot.js | 622 / 474 | 623 / 477 |

`render.wasm` is mostly Rust's standard library (allocator, formatting, Unicode case mapping,
hashing) and the renderer's own style, layout and paint code; it is built with `opt-level = "z"`,
LTO and stripped names. For comparison, the DOM version of the todo example shipped 15,962 / 6,021
(app.wasm) + 14,768 / 5,204 (vcwl.js) + 1,907 / 872 (app.css).
