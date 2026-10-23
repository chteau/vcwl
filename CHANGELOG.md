# Changelog

All notable changes, grouped by milestone.

## 0.3.0 — 2026-10-23

"It ain't Rust" edition. VCWL looks like Rust. It is not Rust. There is no borrow checker, no
lifetimes, no `unsafe`, and no crab. Writing Rust in a `.vcwl` file gets you a diagnostic, not a
compiler.

- Tuples: `(Int, String)`, `(a, b)`, `t.0`, `let (a, b) = t`, `for (k, v) in xs` and tuple
  patterns in `match`, with 2 to 4 elements. They are sugar for the prelude enums `Tuple2` to
  `Tuple4` and encode to JSON as such.
- `Map<V>` with String keys (JSON objects have string keys, and so do we): `empty_map<V>()`,
  `get`, `has`, `set`, `remove`, `len`, `keys`, `values`, `entries`. Maps are immutable: `set`
  returns a new map. A set of Strings: `empty_set()`, `has`, `add`, `remove`, `len`, `items`,
  `union`, `intersect`.
- Strings: `chars()`, `char_len()` and `char_slice(a, b)` count Unicode code points; `len` and
  `slice` still count bytes.
- `await` also works inside `if`/`else` and `match` arms of handlers (the rest of the handler is
  copied into each branch). Still not in loops or closures.
- Not-Rust diagnostics: a file that fails to parse and contains `println!`, `::`, `&`,
  `let mut`, `#[derive]`, `.unwrap()`, `Vec<`, `pub fn`, `use` or lifetimes gets an error with
  the VCWL spelling. `.rs` entry files are refused.
- Runtime errors carry `at file:line:col`, and wasm modules have a name section, so stack traces
  show VCWL function names.
- Route parameters: `"/post/:id" => Post(id: id)` binds `id: String` in the arm.
  `prerender_routes = ["/post/1"]` in vcwl.toml prerenders concrete URLs; other parameter URLs
  are served by `404.html` with path routing.
- Components: more than 62 values (those past 60 share one dependency bit, so they re-render a
  little more); `state` and `let` anywhere in a component body except inside `for` and
  `match`/routes arms; derived `let` values read in a handler after an assignment see the new
  value; `style if` always overrides a plain `style` on the same element.
- `on js "name"(msg: T) { ... }` handlers; JavaScript calls `window.vcwl.dispatch("name",
  value)` (also on the app object). `@web` gains `websocket(url, event) -> Int`,
  `ws_send(id, text)` and `ws_close(id)`; each message arrives as the js event `event`.
- `meta(title: ..., description: ...)` view item sets the document title and meta description
  while shown; prerendered pages get them.
- Texts and attributes over 60,000 bytes are sent in chunks, so the limit is gone. Keyed lists
  use a key map and a longest increasing subsequence: O(n log n), minimal moves.
- Server functions: calls run in parallel, each in its own instance. `query<T>(sql, params)` and
  `exec(sql, params)` use a SQLite file next to the store (`<store>.sqlite`), params bound as
  text. `http(method, url, body)` returns `Result<HttpResponse, String>` by shelling out to curl
  with a 10 s timeout (we invented the network request again, but worse). Sessions:
  `session(key)`, `set_session(key, value)`, `end_session(key)`, in an HMAC-SHA256 signed
  `vcwl_session` cookie whose secret comes from `VCWL_SECRET` or is generated and kept in the
  store. An optional `vcwl.limits.toml` sets `calls_per_minute` and per-function limits per
  client IP; over the limit is a 429.
- `vcwl deploy <netlify|pages|folder DIR> [--yes]` builds, performs a ceremony (type the project
  name unless `--yes`) and writes `dist/.vcwl-manifest.json` with the sha256 of every file;
  `VCWL_DEPLOY_DRY_RUN=1` skips the upload.
- `vcwl doctor` checks the environment and project, including Rust contamination.
- `vcwl new --enterprise <dir>` creates a working counter app spread over domain, application
  and presentation layers, with factories and interfaces.

## 0.2.0 — 2026-10-23

- Closures and first-class functions: `|x| x * 2`, `|a, b: Int| -> Int { ... }`, function types
  `fn(Int) -> String`, named functions as values, and calls through any expression
  (`make_adder(1)(2)`, `button.on_press()`). Closures capture copies of the values they use,
  component state included (views update when it changes); they cannot assign to them.
- Array methods that take functions: `map`, `filter`, `fold`, `each`, `any`, `all`, `find`,
  `sort_by` (stable merge sort).
- Methods: `impl Type { fn name(self, ...) ... }`, `Self`, and functions without `self` called as
  `Type.name(...)`; generic types name their parameters (`impl Stack<T>`).
- Interfaces: `interface Shape { fn area(self) -> Float }` and `impl Shape for Circle { ... }`. A
  value whose type implements the interface converts to it wherever a `Shape` is expected, so
  arrays can mix implementations (`let shapes: [Shape] = [Circle {...}, Square {...}]`).
- `await` in event handlers: `let res = await get(url) as T` (or a server function or
  `extern fn`); the rest of the handler runs when the response arrives, with its local variables.
- Arrays keep spare room: `push` and `concat` append in place when the array is the newest
  version of its storage, so building an array with `xs = xs.push(x)` is amortized O(1) instead
  of O(n) per element. Older versions are unaffected.
- Editors highlight `impl`, `interface`, `await`, `as`, `self` and `Self`, and `vcwl lsp`
  completes them.
- `vcwl upgrade` installs the latest release, once you solve a random math problem. Other commands
  print a notice when a newer release exists; the check runs in the background at most once a
  day, never blocks a command, and is off with `VCWL_NO_UPDATE_CHECK=1` or in CI. Programs written
  for 0.1.0 compile unchanged.

## 0.1.0 — 2026-10-22

The first release, republished in place the same day with the changes under "Republished".

### Republished

- Page text can be selected with the mouse (drag, double and triple click, Ctrl+A) and copied
  with Ctrl+C; the cursor is a text cursor over it.
- `font "Inter" from google weight 400..700`: Google Fonts, linked from the page, with the
  Content-Security-Policy allowing Google's hosts.
- `touch_icon` in vcwl.toml: a PNG home-screen icon, also a favicon fallback; `icon` gets a
  `type`.
- The canvas sizes itself from its own box: on phones, a prerendered page wider than the screen
  no longer leaves the app laid out at the wider width.
- The site has a phone menu, and its docs fold the page list into a button on narrow screens.
- `supabase` (vcwl-supabase), the first package that does something: Supabase auth (password,
  magic link, OAuth), the database through PostgREST, storage URLs; `examples/supabase`.
- Packages can ship a JavaScript module (`js` in their vcwl.toml) for their `extern fn`s; the
  security review flags it as a danger.
- `fetch f(args) as T` decodes a JavaScript function's result as `T`; `to_json(value)`.
- `oauth` (vcwl-oauth): sign-in popups for Google, Discord, GitHub, Roblox, Microsoft, Apple,
  Twitch, Spotify, GitLab, Facebook, Reddit, LinkedIn, Slack, X and custom providers (PKCE,
  implicit, or a token proxy for providers that need a client secret); `examples/oauth`.
- The site has a Packages page that lists the registry branch from GitHub, with a search bar;
  `@web` gains `fetch_text(url)`.
- The sample packages have no unit tests anymore. Vibes.
- A text field no longer keeps a horizontal scroll from a narrower layout (its text showed cut
  off on the left).
- `vcwl lint`: `TODO`/`FIXME` markers are case-sensitive (`todo:` as a property name is fine).
- Touch scrolling keeps its momentum after the finger lifts (a fling that slows down like
  native scrolling); the next touch stops it.

### Milestone 16 — packages, editors, styles beyond CSS, JavaScript interop

- `vcwl install/add/remove/publish/search`: a package manager whose registry is the `registry`
  branch of a git repository (like Wally); `^ ~ = *` requirements, `vcwl.lock`,
  `vcwl_packages/`. Sample packages: is-even, is-odd, left-pad, strings-plus, vibes.
- Registry packages are reviewed before installation (`vcwl audit`); dangerous findings block
  until the app trusts the exact version in `[trust]`.
- `vcwl lint` (unused functions, components, state and bindings; placeholder text; deep views)
  and `vcwl lsp` (diagnostics, formatting, completion); a tree-sitter grammar, a VS Code and a
  Zed extension.
- Styles CSS lacks: `corner` shapes (squircle, cut, scoop), `grain`, `sketch` borders, `spring`
  and `bounce` easing.
- `extern fn` calls JavaScript (sync, or async with `fetch`); `js` in vcwl.toml names the app's
  module; `import "@web"` covers URL, history, popups and postMessage (OAuth), storage and
  cookies, Intl, Web Crypto and device APIs.
- The pipe operator: `x |> f(a)` is `f(x, a)`.
- `site/`: the project's GitHub Pages site, written in VCWL (satire, with a section of real
  facts), and a Pages workflow that builds it; styled with the debugger's palette, a typing
  terminal and a highlighted code sample.
- The site has documentation pages (install, tour, components, styles, data, 3D, packages,
  interop, tools, deploy), a mascot (Vibbles) and Open Graph images; it uses real URLs.
- `vcwl --version`; `write_clipboard` in `@web`.
- Fixed: with path routing, prerendered pages resolved relative `src` and `href` values against
  the route's directory instead of the site root.
- Fixed: changing a text node's content did not lay out (or repaint) its parent again when the
  text node itself had no cached layout.

### Milestone 15 — the rest of the 3D

- Point lights cast shadows (a cube map of distances), like directional lights.
- Overlapping translucent meshes blend in depth order at each pixel (depth peeling, four layers),
  whatever their shapes; before, they were sorted by their centers.
- glTF materials are shaded as metallic-roughness materials (Cook-Torrance), with their
  metallic-roughness, normal, emissive and occlusion maps.
- glTF node animations (translation, rotation, scale; step, linear and cubic spline) loop; a
  mesh's `animation` names the one to play.
- Accessibility mirror layout: inline text spans were tried and made no measurable difference;
  see `docs/benchmarks.md`.

### Milestone 14 — finer slot updates

- Slot content updates only the parts that read a changed value: the user passes its dirty mask
  through the child instead of refreshing the whole content (one state change read by one of two
  texts in three rows: 6 → 3 DOM commands, checked by a browser test).

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
