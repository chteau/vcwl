# Architecture

```
.vcwl files ─▶ lexer ─▶ parser ─▶ checker ─▶ TIR ─▶ lowering ─▶ IR ─▶ opt ─▶ codegen ─▶ app.wasm
                                    │                  │                         │
                              check_ui/app       lower_ui (views)            wasmparser
                              style (CSS)        json (generated)            validation
```

| stage | file | role |
| --- | --- | --- |
| lexer | `src/lexer.rs` | tokens with a "newline before" flag (newlines end statements) |
| parser | `src/parser.rs` | recursive descent to `src/ast.rs` |
| checker | `src/check.rs`, `src/check_ui.rs`, `src/app.rs` | name resolution, bidirectional inference with unification, exhaustiveness, component and view rules, capabilities |
| styles | `src/style.rs` | typed property checking, CSS generation |
| JSON | `src/json.rs` | per-type decoders/encoders for `fetch`, generated as typed functions |
| TIR | `src/tir.rs` | typed tree; generic functions still mention type parameters |
| lowering | `src/lower.rs`, `src/lower_ui.rs` | monomorphization; data layout; components to mount/update/destroy functions |
| IR | `src/ir.rs` | a tree close to WebAssembly (structured control flow, labels) |
| opt | `src/opt.rs` | constant folding, removal of unreachable functions and imports |
| codegen | `src/codegen.rs` | binary encoding with `wasm-encoder`; validated with `wasmparser` |
| project | `src/project.rs`, `src/devserver.rs` | static output, CSP, dev server |

The prelude (`src/prelude.vcwl`) is file 0 of every compilation and is written in VCWL; only it may
call `__` intrinsics, which lower to WebAssembly instructions or host imports.

## Data representation

WebAssembly GC types (one recursive type group): strings are `(array (mut i8))`, arrays are
`(array (mut T))`, structs are GC structs, enums are a base struct holding the tag with one
subtype per variant. Memory is managed by the engine's garbage collector; there is no allocator in
the module. Linear memory (2 pages, growable) only carries data between the module and the host.

## Browser protocol

```
page 0 (64 KiB)        DOM command buffer
page 1 and above       I/O region (IO_BASE = 65536): strings passed to and from the host
```

The module never touches the DOM. Rendering writes commands into the buffer and calls the
`flush(len)` import, which applies them. Each command is an opcode, integer operands and
length-prefixed UTF-8 strings (4-byte aligned).

| opcode | command | opcode | command |
| --- | --- | --- | --- |
| 1 | create element | 10 | set `checked` |
| 2 | create text | 11 | listen (event → handler slot) |
| 3 | create comment (anchor) | 12 | set class |
| 4 | insert before | 13 | toggle class |
| 5 | remove | 14 | set document title |
| 6 | set text | 15 | navigate |
| 7 | set attribute | 16 | dev warning |
| 8 | remove attribute | 17 | clear children |
| 9 | set `value` | 18 | fetch |

Exports: `start()`, `set_route(len)`, `dispatch(slot, len)` (DOM events), `fetch_done(slot, kind,
status, len)`, `live_slots()`, `memory`, and `main`/`test_*` for programs. Imports: `print`,
`panic`, `fmt_f64`, `now`, `flush`, and `json_*` for decoding responses.

## Components and reactivity

Each component instance is a GC struct: a dirty bitmask, one field per property/state/derived value,
DOM node ids and child scopes. Conditional branches and list items are scopes of their own with
generated `mount`, `update`, `destroy`, `move` and `first` functions.

Dependencies are computed at compile time: every view expression knows which component values it
reads (bit *i* for value *i*). A handler marks the bits of the values it assigns; when it returns,
the component's `update` runs once and only re-evaluates expressions whose bits intersect the dirty
mask. There is no virtual DOM.

Keyed lists match new items to old fragments by key, destroy fragments whose key disappeared,
mount new ones, and move only fragments outside the longest increasing subsequence of old positions.

Event listeners are registered by the host and identified by a *handler slot*; the module keeps the
slot table (scope and handler kind) in GC arrays and releases slots when elements are destroyed.
Pending `fetch` requests use the same table.

## Decisions

See [docs/adr](adr/).
