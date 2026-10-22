# Security model

VCWL aims for secure defaults in the generated application. WebAssembly isolates the module's
memory from the host, but that does not make an application secure: logic errors, unsafe use of
data and misconfigured hosting remain possible. This page lists what the toolchain does and does not
enforce.

## Enforced

- **No markup at run time.** The element tree lives in the renderer. The accessibility mirror creates
  DOM elements only from a fixed list of tags, sets text with `textContent`, copies only `aria-*`,
  `role`, `title`, `lang` and `placeholder` attributes (plus sanitized `href`), and never sets
  `src`, `style` attributes or event handlers. Text is painted or assigned, never parsed.
- **No script URLs.** Links are followed by the host after rewriting `javascript:` and other
  script-capable URLs to `about:invalid`; image sources are sanitized the same way.
- **No event-handler attributes.** The renderer refuses attributes starting with `on` and properties
  other than `value`, and rejects malformed command buffers (unknown nodes, cycles).
- **Capabilities.** An app declares the hosts it may contact. The build emits a
  Content-Security-Policy meta tag: `default-src 'none'`, scripts from `'self'` only, no styles
  (except, with prerendering, the one inline style sheet allowed by its SHA-256 hash), fonts from
  `'self'` only when the app declares fonts,
  `wasm-unsafe-eval` for the module, `connect-src` limited to `'self'` plus the declared hosts
  (HTTPS only). The host also refuses `fetch` URLs outside the capability before any request is
  made, and `fetch` without a network capability is a compile error.
- **Runtime validation of responses.** Static types are not trusted for data from the network:
  generated decoders check every value and report the path of the first mismatch.
- **Bounds and arithmetic checks.** Array indexing is bounds-checked; division by zero traps. A trap
  stops the application (no further handlers run) and dev builds show the error.
- **No re-entrancy.** The renderer queues events and the host delivers them one at a time, between
  calls into the application; events for elements removed in the meantime are dropped.

- **Prerendered HTML is generated, not templated.** It is serialized from the renderer's tree:
  text and attribute values are escaped, only a fixed list of attributes and `aria-*` is written,
  URLs are sanitized as at run time, tags are restricted to lowercase names, and bound style values
  are written only if they parse as finite numbers with a known unit.

- **Server functions run in a WebAssembly runtime (wasmtime)**, one fresh instance per call with
  256 MB of memory and 5 s of time at most, one call at a time. They reach the outside world only
  through the host functions the server defines (environment variables, the store, the log).
  Their arguments are validated against the declared types before the function runs. Calls must
  be `POST` with `content-type: application/json` (which other sites cannot send without a CORS
  preflight, never granted) and are refused when an `Origin` header names another host. The
  client module contains no server code. This isolates server code from the server process; it
  does not make the functions themselves correct, and they have no authentication built in: any
  visitor of the site can call them with any arguments of the right types.

## Not enforced or not yet implemented

- Server functions: no authentication, sessions, rate limiting or request size quota beyond
  1 MB; the store is a single JSON file meant for small data.

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
