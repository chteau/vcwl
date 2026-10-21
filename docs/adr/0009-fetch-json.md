# 9. fetch: browser networking, generated validators

Status: accepted

## Context

The spec requires typed HTTP with explicit errors and run-time validation of responses, using the
browser's networking. VCWL has no async functions or closures.

## Decision

- `fetch method(url[, body]) as T { arms }` is a statement in event handlers. The arms become a
  continuation function stored with the component scope in a handler slot.
- The host performs the request (`fetch`, 30 s timeout via `AbortSignal.timeout`), parses JSON with
  `JSON.parse`, and calls `fetch_done(slot, kind, status, len)`.
- The checker generates a decoder per target type as ordinary typed functions. Decoders read the
  parsed value through integer handles (`json_kind`, `json_field`, ...) and return
  `Result<T, String>` with the path of the first mismatch. Encoders for request bodies are generated
  the same way.

## Consequences

Parsing is native and fast; validation is compiled code that cannot be skipped. Generating the
validators as typed functions means the type checker and the normal lowering handle them, with no
special code generation. Arms cannot capture handler locals (there are no closures); components
removed before the response arrives discard it.
