# 13. Optional path routing with a page per route

Status: accepted (extends [ADR 7](0007-hash-routing.md))

## Context

Hash URLs (`#/about`) work on any static host but look odd, are second-class for crawlers and
cannot have a page of their own per route. Static hosts such as GitHub Pages cannot rewrite
unknown paths to one `index.html`, but they serve `dir/index.html` for `/dir/` and `404.html` for
anything missing.

## Decision

`routing = "path"` in vcwl.toml. The element tree still links routes as `#/path`; only the edges
change: the host maps them to `<root>/path/` URLs with `pushState`/`popstate` (the root is the
directory of `vcwl.js`, so no base path is configured for the app itself), the mirror and the
prerendered HTML write real URLs, and the build writes `path/index.html` for every route with
relative asset paths plus a `404.html` with absolute ones (base taken from `url`). Relative URLs
in the app (fetch, images, fonts) resolve against the root, not the page.

## Consequences

- Real URLs on static hosting, a prerendered page per route, canonical links and a sitemap.
- Unknown paths return HTTP 404 with the app's fallback route rendered, which is accurate.
- Route paths are restricted to URL- and file-name-safe characters.
- Hash routing stays the default.
