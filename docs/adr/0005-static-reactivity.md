# 5. Compile-time dependency masks instead of a virtual DOM

Status: accepted

## Context

Components must update the DOM efficiently when state changes.

## Decision

The compiler computes, for every view expression, the set of component values it reads, as a bit
mask. Handlers record which values they assign. After a handler, `update` re-evaluates only the
expressions whose masks intersect the dirty mask and emits commands for values that changed. Keyed
lists reconcile by key and move only items outside the longest increasing subsequence.

## Consequences

No diffing of a virtual tree and no runtime dependency tracking. Limits: 62 values per component
(i64 mask with two reserved bits), and derived values update after the handler returns.
