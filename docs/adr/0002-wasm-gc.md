# 2. Target WebAssembly GC instead of linear memory

Status: accepted

## Context

VCWL values (strings, arrays, structs, enums) need automatic memory management. Options: a
linear-memory allocator with reference counting or a tracing GC compiled into the module, or the
GC proposal (Wasm 3.0) where the engine manages structs and arrays.

## Decision

Use WasmGC types for every heap value. Linear memory is kept only as a byte channel to the host.

## Consequences

- No allocator or collector in the module, so smaller output and no leaks from reference cycles.
- Requires browsers with WasmGC (Chrome/Edge 119+, Firefox 120+, Safari 18.2+); older browsers get
  an explicit error.
- Strings cross the host boundary by copying into linear memory.
