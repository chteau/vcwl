# VCWL — Vibe Coded Web Language

A statically typed language for web applications. The compiler, written in Rust, compiles VCWL
to WebAssembly (with the GC proposal, Wasm 3.0) plus a static `index.html`, a CSS file and a small
JavaScript host. Application logic, views, state and styles are written in VCWL; JavaScript is
limited to the host that applies DOM commands and calls browser APIs.

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

## Status

Working end to end, tested in Node.js and headless Chrome:

- language: structs, enums with payloads, generics (monomorphized), pattern matching with
  exhaustiveness checking, immutable values with `var` locals, string interpolation, modules;
- components with props, state, derived values, typed events, conditional and keyed list
  rendering, hash routing and forms with native HTML elements;
- typed styles compiled to CSS classes, with responsive (`when viewport < 600px`) and state
  (`when hover`) variants;
- `fetch` with JSON validated at run time against the expected type, and network access limited
  by declared capabilities (enforced by the host and the Content-Security-Policy);
- tooling: `vcwl new | dev | build | check | test | fmt | run | compile | clean`.

## Quick start

Requirements: Rust (edition 2024), Node.js for `vcwl run`/`vcwl test`, a browser with WebAssembly GC
(Chrome/Edge 119+, Firefox 120+, Safari 18.2+).

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
| `vcwl build [--dev]` | compile the project to `dist/` |
| `vcwl check [file]` | parse and type-check only |
| `vcwl test [paths]` | run `test_*` functions (default: `tests/**/*.vcwl`) |
| `vcwl fmt [--check] [paths]` | normalize indentation and whitespace |
| `vcwl run <file>` | compile a program and run its `main` in Node.js |
| `vcwl compile <file> [-o f]` | compile one file to `.wasm` |
| `vcwl clean` | remove the output directory |

## Documentation

- [Language reference](docs/language.md)
- [Architecture](docs/architecture.md) and [decision records](docs/adr/)
- [Security model](docs/security.md)
- [Benchmarks](docs/benchmarks.md)
- [Known limitations](docs/limitations.md)
- [Changelog](CHANGELOG.md)

## Repository layout

```
src/            compiler (lexer, parser, checker, lowering, codegen) and CLI
src/prelude.vcwl standard library, written in VCWL
runtime/        browser/Node host (vcwl.js), boot and dev-reload scripts
examples/todo/  routed todo app with components, forms and tests
tests/          compiler tests (Rust), CLI tests, browser tests (puppeteer)
bench/          keyed-list benchmark against hand-written DOM code
docs/           reference, architecture, ADRs
```

## Development

```sh
cargo test                                # compiler, CLI and formatter tests (needs node)
(cd tests/browser && npm ci && npm test)  # headless Chrome tests (CHROME=/path/to/chrome)
node bench/run.mjs                        # benchmark (after the two steps above)
```
