# 4. DOM access through a command buffer

Status: accepted

## Context

WebAssembly cannot call DOM APIs directly. Calling one import per DOM operation crosses the
boundary thousands of times for a large update.

## Decision

The module writes binary commands (opcode, integers, strings) into a buffer in linear memory and
calls `flush(len)` once per batch; `runtime/vcwl.js` applies them. Nodes are referred to by integer
ids. DOM events come back through one export, `dispatch(slot, len)`.

## Consequences

One boundary crossing per batch; the host stays small (about 450 lines) and has no application
logic. The host is also the single place where security checks on attributes and URLs happen.
Command strings are limited to 60,000 bytes.
