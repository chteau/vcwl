# Changelog

All notable changes. The project has no release yet; entries are grouped by milestone.

## Unreleased

### Milestone 6 — networking and tooling

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
