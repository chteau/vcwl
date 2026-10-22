# 12. Build-time prerendering with canvas take-over

Status: accepted

## Context

Pages are drawn on a canvas by `render.wasm`. Until both modules have loaded and compiled nothing
is visible (measured: first canvas frame at ~290 ms on a 40 ms / 20 Mbps link), and crawlers that
do not run JavaScript see only the page metadata.

## Decision

At build time, run the real app and renderer in Node.js for each route and serialize the
renderer's tree to HTML (`Engine::html`), styled by the app's own compiled style sheet plus
generated classes for bound styles. One `index.html` holds every route, switched with CSS
`:target`, so no server-side routing or JavaScript is needed. The inline style sheet is allowed by
a CSP hash. When the canvas has painted its first frame, it replaces the static markup at the same
scroll position.

Alternatives: a `<noscript>` text dump (no first paint, poor for crawlers that run JavaScript but
time out), one HTML file per route (needs history routing, see ADR 7), or server-side rendering
(no static hosting).

## Consequences

- First contentful paint of `examples/tokenforge` on the slow link went from none before the
  canvas (~290 ms) to 64–92 ms ([benchmarks](../benchmarks.md#start-up)).
- The browser lays out the static HTML, so it can differ slightly from the renderer's layout.
- `index.html` grows with the content (~100 KB raw, ~22 KB gzip for tokenforge's routes).
- Builds with `prerender = true` need Node.js.
