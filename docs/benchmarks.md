# Benchmarks

Numbers on this page are measurements on one machine, not general claims. Re-run them before
drawing conclusions.

## Keyed rows (`bench/`)

The same four actions on a list of 1000 rows, implemented in VCWL (`bench/rows`) and as
hand-written DOM code specialised for each action (`bench/vanilla/index.html`). Each sample is
timed in the page from `click()` to the end of a forced style/layout (`offsetHeight`); 5 warm-up
rounds are discarded; the table shows the median of 25 rounds.

```sh
cargo build && (cd tests/browser && npm ci) && node bench/run.mjs
```

Measured 2026-10-21; Linux, Chrome 151 headless, Node 26, debug build of the compiler (compiler
build mode does not affect the generated module):

| action | VCWL (ms) | vanilla JS (ms) | ratio |
| --- | ---: | ---: | ---: |
| create 1000 rows | 7.7 | 9.1 | 0.8 |
| update every 10th row | 0.6 | 0.5 | 1.2 |
| swap two rows | 0.3 | 0.3 | 1.0 |
| clear | 1.1 | 0.7 | 1.6 |

Timer resolution in the page is 0.1 ms, so the small values are approximate. Before the
longest-increasing-subsequence change, swapping took 7.1 ms because 997 rows were moved.

What this shows: for these operations the generated code stays within a small factor of
hand-written DOM code. It does not compare against UI frameworks, larger applications, memory use
or start-up time.

## Output size

`vcwl build` of the examples (bytes, raw / gzip -9):

| file | todo example | rows benchmark |
| --- | ---: | ---: |
| app.wasm | 15,962 / 6,021 | 5,649 / 2,567 |
| vcwl.js (host) | 14,768 / 5,204 | 14,768 / 5,204 |
| app.css | 1,907 / 872 | 674 / 401 |
| index.html + boot.js | 828 / 573 | 829 / 576 |
