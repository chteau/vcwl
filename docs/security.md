# Security model

VCWL aims for secure defaults in the generated application. WebAssembly isolates the module's
memory from the host, but that does not make an application secure: logic errors, unsafe use of
data and misconfigured hosting remain possible. This page lists what the toolchain does and does not
enforce.

## Enforced

- **No markup at all.** The element tree lives in the renderer; nothing the application produces
  reaches the browser DOM, which contains only the canvas. Text is painted, never parsed.
- **No script URLs.** Links are followed by the host after rewriting `javascript:` and other
  script-capable URLs to `about:invalid`; image sources are sanitized the same way.
- **No event-handler attributes.** The renderer refuses attributes starting with `on` and properties
  other than `value`, and rejects malformed command buffers (unknown nodes, cycles).
- **Capabilities.** An app declares the hosts it may contact. The build emits a
  Content-Security-Policy meta tag: `default-src 'none'`, scripts from `'self'` only, no styles,
  `wasm-unsafe-eval` for the module, `connect-src` limited to `'self'` plus the declared hosts
  (HTTPS only). The host also refuses `fetch` URLs outside the capability before any request is
  made, and `fetch` without a network capability is a compile error.
- **Runtime validation of responses.** Static types are not trusted for data from the network:
  generated decoders check every value and report the path of the first mismatch.
- **Bounds and arithmetic checks.** Array indexing is bounds-checked; division by zero traps. A trap
  stops the application (no further handlers run) and dev builds show the error.
- **No re-entrancy.** The renderer queues events and the host delivers them one at a time, between
  calls into the application; events for elements removed in the meantime are dropped.

## Not enforced or not yet implemented

- The canvas renderer exposes nothing to assistive technology; see
  [limitations](limitations.md).
- A meta-tag CSP cannot set `frame-ancestors`, reporting or sandboxing; configure HTTP headers on
  the server when possible.
- `storage` capabilities are parsed but there is no storage API yet.
- Requests are not cancelled when a component is removed (the response is discarded).
- No request headers, credentials control or CSRF tokens yet; requests use the browser defaults
  (`same-origin` credentials).
- Same-origin requests are allowed by `connect-src 'self'` for loading `app.wasm` even without a
  network capability; the host still refuses `fetch` calls in that case.
- The dev server (`vcwl dev`) is meant for local development only: it binds to 127.0.0.1 and has no
  authentication.
