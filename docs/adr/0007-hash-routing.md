# 7. Hash-based routing

Status: accepted

## Context

Apps must work on static hosts such as GitHub Pages, which cannot rewrite unknown paths to
`index.html`.

## Decision

Routes match the URL fragment (`#/about`). `link(to: "/about")` renders `href="#/about"`;
`hashchange` updates the reactive `route` value.

## Consequences

Works on any static host and under any sub-path without configuration. URLs contain `#`; history-API
routing could be added later for hosts that support rewrites.
