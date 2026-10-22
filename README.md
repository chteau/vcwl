<p align="center">
  <img src="site/public/logo.svg" width="148" alt="Vibbles, the VCWL mascot: a lime blob wearing sunglasses">
</p>

<h1 align="center">VCWL</h1>

<p align="center">
  <b>The Vibe Coded Web Language.</b><br>
  It is going to revolutionize web development. <i>(It is not. It compiles, though, which is worse.)</i>
</p>

<p align="center">
  <a href="https://vcwl.chteau.bzh/">Website</a> ·
  <a href="https://vcwl.chteau.bzh/docs/install/">Docs</a> ·
  <a href="https://github.com/chteau/vcwl/releases/latest">Download</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

VCWL is a statically typed language for web apps. The compiler, written in Rust, turns it into
WebAssembly (with the GC proposal). Your logic, views, state and styles are all VCWL. Then, for
reasons nobody can fully explain, the app doesn't use the browser DOM: a renderer, also written
in Rust and compiled to WebAssembly, lays out and paints everything onto a single `<canvas>`,
while an invisible DOM mirror keeps it accessible, selectable and crawlable.

It started as a joke and kept going. The joke is now load-bearing.

```vcwl
app "Counter"

component App {
    state count: Int = 0

    column(gap: 12px, align: center) {
        heading("Clicks: {count}", level: 1)
        button("+1") {
            style {
                radius: 12px
                corner: squircle
                transition: transform 400ms spring
                when hover {
                    transform: scale(1.08)
                }
            }
            on click {
                count += 1
            }
        }
    }
}
```

The [website](https://vcwl.chteau.bzh/) is itself written in VCWL, docs included.

## Install

```sh
curl -fsSL https://vcwl.chteau.bzh/install.sh | sh     # Linux, macOS
irm https://vcwl.chteau.bzh/install.ps1 | iex           # Windows (PowerShell)
```

Grab a binary from the [latest release](https://github.com/chteau/vcwl/releases/latest)
(Linux x86_64, macOS Apple silicon and Intel, Windows x64), or build it:

```sh
rustup target add wasm32-unknown-unknown
cargo install --git https://github.com/chteau/vcwl vcwl
```

Node.js 20+ is needed for `vcwl run`, `vcwl test` and prerendering. Apps run in browsers with
WebAssembly GC: Chrome/Edge 119+, Firefox 120+, Safari 18.2+.

```sh
vcwl new hello && cd hello
vcwl dev      # http://127.0.0.1:4400, rebuilds and reloads on save
vcwl build    # a static site in dist/
```

## What's in the box

- **A real language.** Structs, enums with payloads, generics, exhaustive pattern matching,
  optionals and `?`, string interpolation, modules, tests, and the pipe operator `x |> f(a)`.
- **Components.** Props, state, derived values, typed events, slots, keyed lists, routing (hash
  or real paths), forms with native controls. Accessibility mistakes are compile errors.
- **Typed styles.** Checked at compile time, with `when hover`, `when viewport < 600px`,
  `when appear` scroll reveals, and things CSS doesn't have: `corner: squircle | cut | scoop`,
  film `grain`, hand-drawn `sketch` borders, `spring` and `bounce` easing.
- **Data.** `fetch` decoded against the type you expect, network access limited by declared
  capabilities (and the CSP), and `server fn`s run by `vcwl serve` in a WebAssembly runtime.
- **3D.** `scene { mesh light group }` with WebGL: primitives, OBJ and glTF models with PBR
  materials and animations, shadows, translucency, clickable meshes. Nobody asked.
- **JavaScript interop.** `extern fn`, and `@web`, about 75 wrapped Web APIs (URL and history,
  OAuth popups, storage, Intl, Web Crypto, clipboard, geolocation...).
- **Packages.** A registry that is a git branch (like Wally), lock files, and a security
  review before anything gets installed.
- **Tools.** `vcwl fmt`, `vcwl lint`, `vcwl lsp`, VS Code and Zed extensions, an in-page
  debugger in dev builds, prerendering, sitemaps and Open Graph tags.

What it is not: fast on purpose, battle-tested, or a good idea for your employer's checkout.
See [known limitations](docs/limitations.md). Benchmarks, with their methods and caveats, are
in [docs/benchmarks.md](docs/benchmarks.md). WebAssembly sandboxes the module; it does not make
an application secure.

## Commands

| command | does |
| --- | --- |
| `vcwl new <dir>` | create a starter app |
| `vcwl dev [--port N]` | dev build, local server, reload on change, errors in the page |
| `vcwl build [--dev]` | compile to `dist/` (and `dist-server/` for server functions) |
| `vcwl serve [--port N]` | release build served with its server functions |
| `vcwl check` · `vcwl test` · `vcwl fmt` · `vcwl lint` | type-check, test, format, lint |
| `vcwl install` · `add` · `remove` · `publish` · `search` · `audit` | packages |
| `vcwl lsp` | language server on stdin/stdout |
| `vcwl run <file>` · `vcwl compile <file>` | run a program's `main` in Node.js, or compile one file |

## Documentation

- [The website's docs](https://vcwl.chteau.bzh/docs/install/), for a guided tour
- [Language reference](docs/language.md)
- [Packages](docs/packages.md) · [Editors](docs/editors.md) · [Deploying](docs/deploy.md)
- [Architecture](docs/architecture.md) and [decision records](docs/adr/)
- [Security model](docs/security.md) · [Benchmarks](docs/benchmarks.md) · [Limitations](docs/limitations.md)

## Repository layout

```
src/            compiler (lexer, parser, checker, lowering, codegen), CLI, LSP, package manager
src/prelude.vcwl standard library, written in VCWL
render/         the renderer (Rust → render.wasm): element tree, styles, layout, paint, input
runtime/        browser/Node host (vcwl.js), 3D, @web, boot and dev scripts
editors/        tree-sitter grammar, VS Code and Zed extensions
examples/       a todo app, a component kit, a remake of the tokenforge website
packages/       the registry's packages (supabase, oauth, is-odd, left-pad, ...), all untested
                because nobody tests anything in the vibe coding world
site/           the website, in VCWL
tests/          compiler, CLI, LSP and package tests (Rust), browser tests (puppeteer)
bench/          keyed-list benchmark against hand-written DOM code
docs/           reference, architecture, ADRs
```

## Development

```sh
cargo test --workspace                    # compiler, renderer, CLI and formatter tests (needs node)
(cd tests/browser && npm ci && npm test)  # headless Chrome tests (CHROME=/path/to/chrome)
node bench/run.mjs                        # benchmark (after the two steps above)
```

## License

[MIT](LICENSE). Vibbles the blob is also MIT. Be nice to Vibbles.
