# 14. Server functions compiled to a separate module, run by wasmtime

Status: accepted

## Context

Apps could only call existing HTTP APIs. A full-stack app needs code that runs on a server
(secrets, persistence) with the same types as the client, in the same language, and without
shipping server code to browsers.

## Decision

- `server fn` in VCWL. The client calls one with `fetch f(args) { arms }`, reusing the typed
  `fetch` machinery: arguments are encoded with the generated JSON encoders, the result decoded
  with the generated decoders. A direct call outside server code is a compile error.
- The compiler emits a second module, `server.wasm`, whose exports are generated entries
  (`__fn_<name>`) that validate the arguments, call the function and encode the result.
- `vcwl serve` (and `vcwl dev`) run it with wasmtime inside the Rust CLI, one instance per call,
  with time and memory limits. The server's host functions mirror the browser host's, plus
  `env` and a key-value store kept in a JSON file.

Alternatives: running `server.wasm` in Node.js (already needed for `vcwl run`/tests) would avoid
a dependency but put the server host in JavaScript; an interpreter of the compiler's IR would be
a second execution engine to keep correct. wasmtime (Bytecode Alliance, actively maintained)
implements the GC proposal VCWL needs; it is built with only the runtime, Cranelift and GC
features, adding about 30 s to a clean debug build.

## Consequences

- End-to-end types between client and server with no schema or code generation by the user.
- The client module carries only URLs of server functions; the server code stays on the server.
- Calls are serialized and stateless apart from the store: simple and predictable, not scalable.
- The `vcwl` binary grows by the runtime; a static-only deployment (GitHub Pages) still works
  for apps without server functions.
