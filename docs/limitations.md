# Known limitations

Current, deliberate restrictions of the implementation. Each is a candidate for future work.

## Language

- No closures or first-class functions, traits/interfaces, tuples, or maps.
- No `async`; `fetch` is a statement with arms that run on completion.
- Strings are byte arrays: `len` and `slice` count bytes, not characters.
- Arrays are immutable values; `push`/`set` copy the array (O(n) per update).
- `Float.round` rounds half away from zero; `Float.to_int` saturates.
- Generic functions are monomorphized: code size grows with the number of instantiations.

## Components

- At most 62 values (properties, state, derived values) per component: dependencies are an i64 mask.
- Derived values are recomputed after a handler returns; inside a handler they keep their old value.
- `let` and `state` are only allowed at the top of a component body.
- A single command string is limited to 60,000 bytes (text content, attribute values, fetch bodies).
- Keyed list matching tries the same position first, then scans: matching is O(n·m) in the worst
  case (n new, m old items) when many items change position.
- Routing uses the URL hash; there is no history-API routing or route parameters yet.

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
- No text selection or copy outside text fields (`copy()` writes to the clipboard from handlers),
  no browser context menus on content, no middle-click on links. Find-in-page searches the mirror,
  so matches are found but highlighted only approximately.
- CSS subset: no margin collapsing, no inline padding/borders/backgrounds on `span`-like elements,
  no `vertical-align` (items in a line are centered), no `align-content`, no grid rows placement or
  named areas, `position: absolute` resolves against the parent element, `sticky` behaves as
  `relative`. Only `opacity`, `transform`, `color` and `background` (colors) transition; other
  properties change at once, and there are no keyframe animations.
- Rotated elements are hit-tested by their bounding box.
- `render.wasm` adds 114 KB (gzip) to every app.
- Components cannot take child content (no slots); compose with properties instead.

## Platform

- Requires WebAssembly GC (Chrome/Edge 119+, Firefox 120+, Safari 18.2+); there is no fallback.
- `vcwl run` and `vcwl test` need Node.js; tests run outside the browser (no renderer in tests).
- Building the CLI needs the `wasm32-unknown-unknown` Rust target (for `render.wasm`).
- No source maps or debugger integration; runtime errors name the WebAssembly function.
- No package manager or third-party libraries.
