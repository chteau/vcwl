# Known limitations

Current, deliberate restrictions of the implementation. Each is a candidate for future work.

## Language

- No `?` operator, closures or first-class functions, traits/interfaces, tuples, or maps.
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

## Platform

- Requires WebAssembly GC (Chrome/Edge 119+, Firefox 120+, Safari 18.2+); there is no fallback.
- `vcwl run` and `vcwl test` need Node.js; tests run outside the browser (no DOM in tests).
- No source maps or debugger integration; runtime errors name the WebAssembly function.
- No package manager or third-party libraries.
