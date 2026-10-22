# 10. Our own element tree, layout and painting instead of the browser DOM

Status: accepted (supersedes the DOM half of [ADR 4](0004-command-buffer.md) and the delivery
of styles in [ADR 6](0006-css-classes.md))

## Context

The goal of the project changed from "native browser semantics first" to an experiment: how much of
the web platform can VCWL provide itself? Until now the JavaScript host applied the command buffer
to the DOM, and the browser did style, layout, painting and input.

Options considered:

- **Keep the DOM.** Smallest and most accessible, but not the experiment.
- **`taffy` for layout.** A maintained Rust flexbox/grid/block engine. A probe with flexbox, grid
  and block layout compiled to 206 KB of WebAssembly (73 KB gzip) on its own, more than ten times
  the previous app plus host, and it would still need text layout and our CSS model on top.
- **Our own renderer in Rust, compiled to WebAssembly.** More code to write and test, sized to the
  subset of CSS the compiler can emit.

## Decision

A second WebAssembly module, `render.wasm`, built from `render/` (a Rust crate with no
dependencies), implements:

- the element tree, built from the same command buffer as before (`render/src/dom.rs`);
- a CSS engine for the compiler's style sheet, carried in a `vcwl.styles` custom section of
  `app.wasm`, plus built-in defaults per element (`css.rs`, `style.rs`);
- layout: block flow, inline text with wrapping, flexbox, grid, stacks, absolute/fixed/relative
  positioning, scroll containers (`layout.rs`);
- painting into a display list, replayed by the host on one `<canvas>` (`paint.rs`);
- input: hit testing, hover/active/focus, Tab order, clicks, double clicks, keyboard activation,
  text editing with selection, checkboxes, select pop-ups, links, wheel and touch scrolling
  (`engine.rs`).

The browser is still used for text measurement and glyph drawing (canvas `measureText` and
`fillText`), image decoding, `fetch`, the clipboard and the URL hash. The page's DOM holds only the
canvas. Commands, styles, layout and events never touch DOM elements.

Style, layout and paint are incremental: commands mark the nodes they change; unchanged subtrees
keep their computed style and cached layout; painting skips subtrees outside the viewport.

## Consequences

- **Accessibility is lost.** Screen readers see one canvas. There is no accessibility tree, no
  find-in-page, no browser text selection outside fields, no autofill, no spellcheck, no input
  method (IME) composition, and mobile virtual keyboards do not open. Keyboard navigation and visible
  focus are implemented by the renderer. See [limitations](../limitations.md).
- **Size grows.** `render.wasm` is 237 KB (107 KB gzip), shared by every app; the host grows from 5
  to 8 KB gzip. The DOM version shipped about 11 KB gzip in total for the todo example.
- **Behaviour is ours to test.** The renderer has native unit tests (`cargo test -p vcwl-render`)
  with a fixed-width text measurer; browser tests drive it with real mouse and keyboard events and
  read its tree through `window.vcwl.tree()`.
- **Security surface shrinks.** No markup, attributes or URLs reach the DOM; link targets are still
  sanitized before navigation, and the CSP no longer needs `style-src`.
