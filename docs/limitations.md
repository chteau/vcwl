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

- **No accessibility tree.** Screen readers see a single canvas. Keyboard navigation (Tab order,
  Enter/Space activation, visible focus) works; everything announced by assistive technology does not.
- No find-in-page, no text selection or copy outside text fields, no autofill, spellcheck or
  password managers, no browser context menus on content, no middle-click on links.
- No input method (IME) composition and no mobile virtual keyboard: text fields accept keys that
  produce a character on their own (including pasted text), not composed input.
- No undo/redo in text fields.
- CSS subset: no margin collapsing, no inline padding/borders/backgrounds on `span`-like elements,
  no `vertical-align` (items in a line are centered), no `align-content`, no grid rows placement or
  named areas, `position: absolute` resolves against the parent element, `sticky` behaves as
  `relative`. Transitions and animations jump to their end state.
- Rotated elements are hit-tested by their bounding box.
- `render.wasm` adds 107 KB (gzip) to every app.

## Platform

- Requires WebAssembly GC (Chrome/Edge 119+, Firefox 120+, Safari 18.2+); there is no fallback.
- `vcwl run` and `vcwl test` need Node.js; tests run outside the browser (no renderer in tests).
- Building the CLI needs the `wasm32-unknown-unknown` Rust target (for `render.wasm`).
- No source maps or debugger integration; runtime errors name the WebAssembly function.
- No package manager or third-party libraries.
