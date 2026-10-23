# Known limitations

Current, deliberate restrictions of the implementation. Each is a candidate for future work.

## Language

- Interfaces have no type parameters, and generic types cannot implement them.
- Map keys are Strings; tuples have 2 to 4 elements.
- Closures capture copies of the values they use: they cannot assign to captured variables or
  component state, and they cannot `emit`, `fetch` or `navigate`.
- `await` only works in event handlers (also inside `if`/`else` and `match` arms, not in loops or
  closures), and only on `fetch`-able calls (HTTP, server functions, `extern fn`s); there are no
  `async` functions.
- `len` and `slice` count bytes; use `chars()`, `char_len()` and `char_slice` for code points.
- Arrays are immutable values. `push` (and `concat`) append in place when the array is the
  newest one built from its storage (`xs = xs.push(x)` in a loop is amortized O(1)); pushing onto
  an older version, `set`, `remove_at` and `slice` copy (O(n)).
- `Float.round` rounds half away from zero; `Float.to_int` saturates.
- Generic functions are monomorphized: code size grows with the number of instantiations.

## Components

- Values past the 60th in a component share one dependency bit, so they re-render slightly more.
- `let` and `state` are not allowed inside `for` or `match`/routes arms.
- With `routing = "path"`, route paths must be letters, digits, `-`, `_`, `.` and `~` between
  slashes (each becomes a directory of `dist/`), and `404.html` needs `url` in vcwl.toml when the
  site is served from a sub-path.

## Rendering (canvas, see [ADR 10](adr/0010-canvas-renderer.md))

- Assistive technology reads an invisible DOM mirror of the semantic elements
  ([ADR 11](adr/0011-accessibility-mirror.md)). Generic containers are left out, hidden
  (`display: none`) subtrees are omitted, and reading order is tree order, not visual order.
  Screen-reader browse mode does not scroll the canvas to the text it reads; focusing a control
  does. Live regions work only through `aria-live` attributes the app sets.
- Text fields are edited by the browser (IME, autocorrect, virtual keyboards, undo), but the caret
  and selection are drawn by the renderer: composition text is shown without an underline, and
  Up/Down in a `textarea` move by the browser's line breaks, which can differ from the renderer's.
  `on keydown` fires for every key, but the app cannot prevent the browser's edit.
- Page text is selected with the mouse (drag, double click for a word, triple click for a block,
  Ctrl+A) and copied with Ctrl+C, but not with a long press on touch screens, and there is no
  browser context menu on content and no middle-click on links. Find-in-page searches the mirror,
  so matches are found but highlighted only approximately.
- CSS subset: no margin collapsing, no inline padding/borders/backgrounds on `span`-like elements,
  no `vertical-align` (items in a line are centered), no `align-content`, no grid rows placement or
  named areas, `position: absolute` resolves against the parent element, `sticky` behaves as
  `relative`. Only `opacity`, `transform`, `color` and `background` (colors) transition; other
  properties change at once, and there are no keyframe animations.
- Rotated elements are hit-tested by their bounding box.
- `render.wasm` adds 134 KB (gzip) to every app.
- Prerendered HTML is laid out by the browser, not by the renderer: it matches closely but not to
  the pixel (measured drift up to ~15 px at the bottom of long pages of `examples/tokenforge`), so
  content can shift slightly when the canvas takes over. Text widths at build time use a fixed
  estimate, so anything the app computes from measured text is approximate in the static page.
- Web fonts are loaded from the app's own origin only (`font-src 'self'`).

## Server functions

- Outgoing requests go through `http`, which shells out to `curl` with a 10 s timeout. `query`
  and `exec` use one SQLite file next to the store, with params bound as text.
- The store is one JSON file rewritten after each change: fine for small data, not for large or
  write-heavy workloads.
- Prerendering does not call server functions (they render their loading state).
- `vcwl serve` speaks HTTP/1.1 without TLS or keep-alive.

## 3D

- Models: triangles only; no skins or morph targets (node translation, rotation and scale
  animations play); every material map reads the first texture coordinates and ignores texture
  transforms and samplers; no image-based lighting, so metals look dark away from their
  highlights; shading works on sRGB colors, not linear ones; normal maps need the
  `OES_standard_derivatives` extension (they are ignored without it).
- One shadow-casting light, from a fixed 1024×1024 map (directional) or six 512×512 faces
  (point).
- Translucent meshes are blended four surfaces deep per pixel; surfaces behind those are not
  drawn. Peeling costs eight extra passes over the translucent meshes and one over the opaque
  ones per layer when a scene has any (not measured).
- Requires WebGL; without it the scene shows its placeholder. Assistive technology sees the
  scene's `label` and its clickable meshes' labels, nothing else of its content.

## JavaScript interop

- Values cross the boundary as JSON: no functions, DOM nodes, binary data or callbacks. JavaScript
  reaches the app through `window.vcwl.dispatch` and `on js` handlers.
- Sync calls block the UI thread like any JavaScript call.

## Platform

- Requires WebAssembly GC (Chrome/Edge 119+, Firefox 120+, Safari 18.2+); there is no fallback.
- `vcwl run` and `vcwl test` need Node.js; tests run outside the browser (no renderer in tests).
- Building the CLI needs the `wasm32-unknown-unknown` Rust target (for `render.wasm`).
- No source maps or debugger integration; runtime errors carry `file:line:col`.
