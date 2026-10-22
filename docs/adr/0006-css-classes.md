# 6. Typed styles compiled to hashed CSS classes

Status: accepted; since [ADR 10](0010-canvas-renderer.md) the style sheet travels in `app.wasm`
and is applied by the renderer instead of the browser

## Context

Styles should be checked by the compiler and support responsive and interactive variants without
inline styles (which a strict CSP forbids).

## Decision

Each `style { ... }` block is type-checked and emitted once into `app.css` as a class named after a
hash of its contents. `when viewport < X` becomes a media query, `when hover` a pseudo-class,
`style if cond` toggles the class at run time.

## Consequences

No inline styles, so `style-src 'self'` works. Identical blocks share a class. Dynamic values
(computed at run time) are not supported in styles; use `style if` to switch between variants.
