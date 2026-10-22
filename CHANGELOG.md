# Changelog

All notable changes. The project has no release yet; entries are grouped by milestone.

## Unreleased

### Milestone 7 — our own renderer instead of the DOM

- Apps no longer use the browser DOM. `render.wasm`, a dependency-free Rust crate (`render/`),
  keeps the element tree, parses the app's styles, computes styles with inheritance and built-in
  element defaults, lays out (block flow, inline text wrapping, flexbox, grid, stacks,
  absolute/fixed/relative positioning, scroll containers), paints to a `<canvas>` through a display
  list, and handles input: hit testing, hover/active/focus states, Tab order and visible focus,
  clicks and double clicks, keyboard activation, text fields with caret, selection, word movement,
  clipboard copy/cut/paste, checkboxes, select pop-ups, links, wheel/keyboard/touch scrolling.
- Style, layout and paint are incremental: commands mark changed nodes, unchanged subtrees keep
  cached layout, off-screen subtrees are not painted (swap of 2 of 1000 rows: 4.6 ms → 1.0 ms).
- `index.html` contains only metadata and one script; styles and configuration travel in custom
  sections of `app.wasm`; the CSP drops `style-src`. `app.css` is no longer produced.
- Browser tests read the renderer's tree (`window.vcwl.tree()`) and use real mouse and keyboard
  input; 36 native renderer tests.
- Regression: no accessibility tree, IME, find-in-page or text selection outside fields
  (see `docs/limitations.md`). `render.wasm` adds 107 KB gzip.

### Milestone 6 — networking and tooling

- The `?` operator for `Result` and optional values.
- `fetch get|post|put|patch|delete(url[, body]) as T { arms }` in event handlers. Responses are
  parsed by the browser and validated at run time by generated decoders; errors are an
  `HttpError` enum (`Network`, `Timeout`, `Status`, `Decode`) and decode errors name the path.
- Request bodies are encoded to JSON by generated encoders.
- The host refuses URLs outside the app's `network` capability; `fetch` without the capability
  is a compile error. Responses for destroyed components are discarded.
- Keyed lists move only items outside the longest increasing subsequence of old positions
  (swapping 2 of 1000 rows went from 997 DOM moves to 2).
- `vcwl test`, `vcwl dev` (server, rebuild on change, error overlay) and `vcwl fmt`.
- Clearer diagnostics: assignment in a view, missing `alt`/`label`, fetch arms using handler locals.
- `bench/`: reproducible keyed-list benchmark against hand-written DOM code.

### Milestone 5 — component events and the todo example

- `event name(T)` / `emit name(v)` and `on name(v) { ... }` on component uses.
- Buttons: `submit:` and `pressed:` arguments.
- `examples/todo`: routing, forms, keyed list, child components, tests.
- Headless-Chrome test suite (puppeteer-core).

### Milestones 2–4 — components, reactivity, styles, DOM host

- Components with props, state, derived values; views with `if`, `match`, `for ... key`, routes.
- Reactive updates from static dependency bitmasks; batched DOM commands through a shared buffer.
- Typed style properties compiled to content-hashed CSS classes; `when` variants for viewport,
  container and pseudo-classes; `style if` toggles.
- Native, accessible elements: required `label` on inputs, `alt` on images, focus-visible outlines,
  reduced-motion support.
- `vcwl build` producing a static site with a Content-Security-Policy derived from capabilities.

### Milestone 1 — compiler core

- Lexer, parser, bidirectional type checker with generics and exhaustiveness checking.
- Typed IR, monomorphizing lowering, constant folding and tree shaking.
- WasmGC code generation with `wasm-encoder`, validated with `wasmparser`.
- Prelude written in VCWL; `vcwl run` executes programs in Node.js.
