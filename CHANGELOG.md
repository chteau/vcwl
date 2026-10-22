# Changelog

All notable changes. The project has no release yet; entries are grouped by milestone.

## Unreleased

### Milestone 13 — slots that pass values, glTF materials

- Slots can pass a value to their content: `slot row(Type)` declares it, `slot(row, value)` gives
  it, `slot(row, x) { ... }` names it at the use site.
- Slots work inside `if`, `match` and `for`, at the root of a component, and can be forwarded into
  a child's content; a slot's body is fallback content shown when it is not filled. Slot content
  is now created by the child through functions the user passes, once per place it appears.
- `scene3d.js` is only written to builds of apps that show a `scene`.
- 3D models are drawn per glTF material, with its base color and base color texture (an image
  file or one stored in the `.glb`); a mesh's `color` multiplies it and its `texture` replaces it.

### Milestone 12 — named slots, a cheaper mirror, more 3D

- Named slots: `slot(name)` in a component's view, `slot(name) { ... }` at the use site; other
  content still goes to the unnamed `slot`.
- Accessibility mirror: fonts of mirror elements come from one shared rule per font instead of an
  inline `font` on each element (mirror script time when creating 1000 rows 4.5 → 3.3 ms, median
  of 20 rounds; browser style and layout of the mirror unchanged).
- 3D: `model` (Wavefront OBJ, glTF and GLB, centered and fitted to a unit cube), `texture`,
  `opacity` (sorted, blended after opaque meshes), `shadows` on a scene (shadow map from the first
  directional light). Clickable meshes require a `label`, are reachable with Tab and activated
  with Enter or Space, appear in the accessibility mirror as buttons and are highlighted when
  focused; arrow keys orbit a focused `orbit` scene. Pick passes no longer see through meshes
  without a handler.

### Milestone 11 — built-in 3D

- `scene`, `mesh`, `group` and `light` elements ([ADR 15](docs/adr/0015-built-in-3d.md)): boxes,
  spheres, planes, cylinders, cones and tori with colors, shininess and wireframe; directional,
  point and ambient lights; nested groups; `spin` animation (off with reduced motion); orbiting
  camera; `on click` on meshes via a GPU pick pass; arguments bound to state. Drawn by
  `scene3d.js` with WebGL, loaded only when an app shows a scene. `Vec3` and `vec3()` in the
  prelude.

### Milestone 10 — real URLs, dev tools and server functions

- `routing = "path"` ([ADR 13](docs/adr/0013-path-routing.md)): History API navigation, one page
  per route (`about/index.html`) and a `404.html` with the fallback route, relative asset URLs so
  the site works under any sub-path, canonical links and `sitemap.xml` when `url` is set, old
  `#/` links redirected. Prerendering writes each route into its own page.
- Relative URLs in apps (fetch, images, fonts) resolve against the app's root.
- Dev tools in dev builds: element tree of the renderer, box model and computed style of the
  selected element, pick-on-page, frame timings (renderer exports `inspect` and `node_at`).
- Server functions ([ADR 14](docs/adr/0014-server-functions.md)): `server fn`, called from
  handlers with `fetch f(args) { ... }`, compiled into `dist-server/server.wasm` and run by
  `vcwl serve` / `vcwl dev` with wasmtime; arguments validated against their types; `env`,
  `save`, `load<T>` and `remove` for server code; a 5 s / 256 MB limit per call.
- Packages ([docs](docs/packages.md)): `[dependencies]` in vcwl.toml, `import "@name"` and
  `import "@name/file.vcwl"`, packages' `public/` files copied into the build, transitive
  dependencies, rebuilds on package changes in `vcwl dev`. `examples/kit` and its demo.
- `vcwl dev` behaves like a static host: `/about` redirects to `/about/`, unknown paths get
  `404.html`.

### Milestone 9 — fonts, slots, prerendering and speed

- Web fonts: `font "Family" from "fonts/file.woff2" [weight N[..M]] [italic]`. The build checks
  the file, preloads it and allows it in the CSP (`font-src 'self'`); the host loads it before the
  first frame (at most 3 s) and re-measures text when a font arrives later.
- Component slots: one `slot` in a component's view receives the content given at the use site.
  The content is compiled as part of the parent (its state, its handlers) and mounted inside the
  child.
- Prerendering (`prerender = true`, [ADR 12](docs/adr/0012-prerendering.md)): every route is
  rendered at build time to escaped static HTML with a CSP-hashed style sheet, switched by
  `:target`; the canvas takes over after its first frame at the same scroll position.
- Renderer: inset box shadows, `em` font sizes against the parent font, default margins that
  follow a class's font size, text placed on the font's alphabetic baseline like CSS.
- Speed (measured, see [benchmarks](docs/benchmarks.md)): accessibility mirror synced after the
  frame with prototype cloning, a shared constructed style sheet and text carried in attributes;
  per-font measuring contexts; paint state caching; offscreen list markers culled; preload hints;
  `render.wasm` built with `opt-level = "s"`. Rows benchmark frame time for create 38.1 → 24.7 ms,
  update 18.8 → 5.9 ms; first contentful paint of the tokenforge site on a slow link 64–92 ms.
- `examples/tokenforge` uses a `Section` component with a slot and is prerendered.

### Milestone 8 — accessibility, motion and a real website

- Accessibility mirror ([ADR 11](docs/adr/0011-accessibility-mirror.md)): invisible DOM elements
  for headings, landmarks, links, buttons, form controls, lists and text, placed over the canvas.
  Screen readers, focus from outside the page, activation by assistive technology, and crawlers
  work with it. Text fields are edited natively through it: IME composition, mobile keyboards,
  autocorrect, paste and undo.
- Transitions of `opacity`, `transform`, `color` and `background` colors, retargeted from the
  current value when interrupted, disabled under reduced motion.
- `when appear` styles: applied until an element first scrolls into view, then transitioned away
  (scroll reveal).
- `style bind <property>: <expr> <unit>`: typed, reactive style values computed at run time (bar
  charts, progress, positions).
- `id:` on every element and `#id` links that scroll smoothly to it; `current:` on links
  (`aria-current`); `copy(String)` writes to the clipboard from handlers; route changes scroll
  to the top.
- `public/` is copied into the build; `vcwl.toml` takes `description`, `theme_color`, `icon`,
  `image` and `url` for meta, Open Graph and Twitter tags.
- Fixed: keyed `for` lists with `String` keys produced an invalid module.
- `examples/tokenforge`: a remake of https://chteau.github.io/tokenforge in VCWL (home, benchmark,
  three docs pages, roadmap; charts built from bound boxes; data fetched from `bench.json`).
- Measured: the event-to-canvas path is unchanged (rows benchmark sync column 7.0 / 1.1 / 1.1 /
  0.8 ms); the mirror adds about 20 ms when 1000 rows are created and 1.5–4 ms per update, after
  the frame is drawn. See [benchmarks](docs/benchmarks.md).

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
