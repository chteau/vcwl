# 8. Capabilities enforced by the CSP and the host

Status: accepted

## Context

Applications should only contact the servers they declare, and the declaration should be visible in
the source.

## Decision

`app` declares `capabilities { network: allow(...) }`. The build derives the Content-Security-Policy
(`connect-src`) from it, the host checks every `fetch` URL against it before issuing the request,
and using `fetch` without a capability is a compile error.

## Consequences

Two independent layers: a bug in the host check is still caught by the browser's CSP, and a
missing CSP (e.g. a page embedded elsewhere) still has the host check. The meta-tag CSP cannot
express every directive; HTTP headers remain the hosting provider's job.
