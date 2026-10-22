# VCWL — Vibe Coded Web Language

A statically typed language for web applications. The compiler, written in Rust, compiles VCWL
to WebAssembly (with the GC proposal, Wasm 3.0). Application logic, views, state and styles are
written in VCWL. Apps do not use the browser DOM: a renderer written in Rust and compiled to
WebAssembly keeps its own element tree, applies the styles, lays out, paints into a single
`<canvas>` and handles input. JavaScript is limited to a host that connects the two modules to
browser APIs (canvas drawing, text measurement, `fetch`, events) and keeps an invisible DOM
mirror of the semantic elements for screen readers, native text input and crawlers.

```vcwl
app "Counter"

component App {
    state count: Int = 0

    column(gap: 12px, align: center) {
        heading("Clicks: {count}", level: 1)
        button("+1") {
            on click {
                count += 1
            }
        }
    }
}
```

This is an experimental project; see [known limitations](docs/limitations.md) before relying on it.
`examples/tokenforge` is a full website (a remake of https://chteau.github.io/tokenforge) built
with it.

## Status

Working end to end, tested in Node.js and headless Chrome:

- language: structs, enums with payloads, generics (monomorphized), pattern matching with
  exhaustiveness checking, immutable values with `var` locals, string interpolation, modules;
- components with props, state, derived values, typed events, conditional and keyed list
  rendering, routing (URL hash or real paths) and forms with native HTML elements;
- typed styles with responsive (`when viewport < 600px`) and state (`when hover`) variants;
- a renderer of its own ([ADR 10](docs/adr/0010-canvas-renderer.md)): CSS cascade and
  inheritance, block/inline/flex/grid layout, text wrapping, painting to canvas, focus and keyboard
  navigation, text fields with selection and clipboard, checkboxes, select pop-ups, scrolling;
- `fetch` with JSON validated at run time against the expected type, and network access limited
  by declared capabilities (enforced by the host and the Content-Security-Policy);
- server functions (`server fn`) called with typed `fetch` from the client and run by
  `vcwl serve` in a WebAssembly runtime, with a persistent key-value store;
- packages (`[dependencies]` in vcwl.toml, `import "@kit"`) for sharing components, styles,
  server functions and assets: the basis for frameworks ([docs](docs/packages.md));
- components with child content (`slot`), web fonts (`font "X" from "fonts/x.woff2"`), and
  build-time prerendering of every route to static HTML for first paint and crawlers;
- 3D scenes (`scene { mesh(...) light(...) group { ... } }`) drawn with WebGL, with clickable
  meshes, an orbit camera and state-bound properties;
- tooling: `vcwl new | dev | build | check | test | fmt | run | compile | clean`.

## Quick start

Requirements: Rust (edition 2024) with the `wasm32-unknown-unknown` target
(`rustup target add wasm32-unknown-unknown`), Node.js for `vcwl run`/`vcwl test`, a browser with
WebAssembly GC (Chrome/Edge 119+, Firefox 120+, Safari 18.2+).

```sh
cargo build --release
./target/release/vcwl new hello
cd hello
../target/release/vcwl dev        # http://127.0.0.1:4400, rebuilds on save
../target/release/vcwl build      # static site in dist/
```

`dist/` can be served by any static host; see [deploying to GitHub Pages](docs/deploy.md).

## Commands

| command | does |
| --- | --- |
| `vcwl new <dir>` | create a starter app |
| `vcwl dev [--port N]` | dev build, local server, rebuild and reload on change, errors shown in the page |
| `vcwl build [--dev]` | compile the project to `dist/` (and `dist-server/` for server functions) |
| `vcwl serve [--port N]` | release build served with its server functions |
| `vcwl check [file]` | parse and type-check only |
| `vcwl test [paths]` | run `test_*` functions (default: `tests/**/*.vcwl`) |
| `vcwl fmt [--check] [paths]` | normalize indentation and whitespace |
| `vcwl run <file>` | compile a program and run its `main` in Node.js |
| `vcwl compile <file> [-o f]` | compile one file to `.wasm` |
| `vcwl clean` | remove the output directory |

### Dev tools

Dev builds (`vcwl dev`, `vcwl build --dev`) show a **VCWL** badge in the bottom-left corner with
the average frame cost. Click it (or press Alt+Shift+D) for a panel with the renderer's element
tree, the box, attributes and computed style of the selected element, a **Pick** mode that
selects the element under the pointer (the app does not receive those clicks), and frame
timings. Production builds do not include it.

## Documentation

- [Language reference](docs/language.md)
- [Packages and frameworks](docs/packages.md)
- [Architecture](docs/architecture.md) and [decision records](docs/adr/)
- [Security model](docs/security.md)
- [Benchmarks](docs/benchmarks.md)
- [Known limitations](docs/limitations.md)
- [Changelog](CHANGELOG.md)

## Repository layout

```
src/            compiler (lexer, parser, checker, lowering, codegen) and CLI
src/prelude.vcwl standard library, written in VCWL
render/         the renderer (Rust → render.wasm): element tree, styles, layout, paint, input
runtime/        browser/Node host (vcwl.js), boot and dev-reload scripts
examples/todo/  routed todo app with components, forms and tests
examples/tokenforge/  remake of the tokenforge website (fetch, charts, docs pages)
tests/          compiler tests (Rust), CLI tests, browser tests (puppeteer)
bench/          keyed-list benchmark against hand-written DOM code
docs/           reference, architecture, ADRs
```

## Development

```sh
cargo test --workspace                    # compiler, renderer, CLI and formatter tests (needs node)
(cd tests/browser && npm ci && npm test)  # headless Chrome tests (CHROME=/path/to/chrome)
node bench/run.mjs                        # benchmark (after the two steps above)
```
