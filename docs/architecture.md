# Architecture

```
.vcwl files ─▶ lexer ─▶ parser ─▶ checker ─▶ TIR ─▶ lowering ─▶ IR ─▶ opt ─▶ codegen ─▶ app.wasm
                                    │                  │                         │
                              check_ui/app       lower_ui (views)            wasmparser
                              style (CSS)        json (generated)            validation

In the browser:

app.wasm ──commands──▶ render.wasm (tree, styles, layout, paint, input) ──display list──▶ <canvas>
   ▲                          │
   └──── events (dispatch) ───┘        runtime/vcwl.js connects the two and owns the canvas
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
| renderer | `render/src/*.rs` (built to `render.wasm` by `build.rs`) | element tree, CSS engine, layout, painting, input |

The prelude (`src/prelude.vcwl`) is file 0 of every compilation and is written in VCWL; only it may
call `__` intrinsics, which lower to WebAssembly instructions or host imports.

## Data representation

WebAssembly GC types (one recursive type group): strings are `(array (mut i8))`, arrays are
`(array (mut T))`, structs are GC structs, enums are a base struct holding the tag with one
subtype per variant. Memory is managed by the engine's garbage collector; there is no allocator in
the module. Linear memory (2 pages, growable) only carries data between the module and the host.

## Browser protocol

```
page 0 (64 KiB)        command buffer
page 1 and above       I/O region (IO_BASE = 65536): strings passed to and from the host
```

Rendering writes commands into the buffer and calls the `flush(len)` import; the host copies the
bytes into the renderer, which applies them to its own element tree. Each command is an opcode,
integer operands and length-prefixed UTF-8 strings (4-byte aligned).

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

Exports: `start()`, `set_route(len)`, `dispatch(slot, len)` (events), `fetch_done(slot, kind,
status, len)`, `live_slots()`, `memory`, and `main`/`test_*` for programs. Imports: `print`,
`panic`, `fmt_f64`, `now`, `flush`, and `json_*` for decoding responses.

## Renderer

`render.wasm` is a Rust crate compiled for `wasm32-unknown-unknown` (linear memory, no
dependencies). Its only import is `host.measure(font, spacing, text) -> width`.

- **Tree** (`dom.rs`): nodes indexed by the ids the app allocates; each command marks the nodes it
  affects for restyle or relayout.
- **Styles** (`css.rs`, `style.rs`): parses the app's style sheet from the `vcwl.styles` custom
  section of `app.wasm` (single-class rules, pseudo-classes, media and container conditions), adds
  per-element defaults (headings, buttons, inputs, lists, ...) and inheritance. Only dirty nodes,
  and descendants whose inherited values changed, are restyled.
- **Layout** (`layout.rs`): `box-sizing: border-box` everywhere; block flow with inline formatting
  (words, atomic inline boxes, `text-align`, `white-space`); flexbox (direction, wrap, grow/shrink
  with min/max clamping, automatic minimum size, justify/align, auto margins, gaps); grid (track
  counts, `fill(min)`, explicit px/fr/auto tracks, column spans, stacks); absolute, fixed and relative
  positioning; scroll containers. Each node caches the inputs and result of its last layout, so
  unchanged subtrees cost a lookup.
- **Paint** (`paint.rs`): a display list of fills, strokes, text runs, clips, opacity, transforms,
  images, shadows and gradients, in paint order with `z-index`. Subtrees whose painted extent is
  outside the viewport are skipped. The painted boxes double as the hit-test list.
- **Input** (`engine.rs`): pointer, wheel, touch and keyboard input becomes events for the app
  (`click`, `dblclick`, `input`, `change`, `keydown`, `focus`, `blur`, `submit`) and default actions
  (checkbox toggling, form submission, link navigation, select pop-ups, scrolling). Events go to the
  host one at a time through `poll()`; an event whose element or listener disappeared in the
  meantime is dropped.

The host (`runtime/vcwl.js`) forwards browser events to the renderer, delivers events to the app,
replays display lists on the canvas each animation frame, and performs effects the renderer asks
for: title, navigation (URL hash, or the History API with `routing = "path"`), opening links, `fetch`, image loading, cursor, clipboard writes.


### Accessibility mirror

After each canvas paint the host asks the renderer for `semantics()`: a pre-order list of the
semantic elements and text runs with their viewport rectangles, states and `aria-*` attributes,
each with a hash. The host keeps one transparent `position: fixed` DOM element per entry over the
canvas (`pointer-events: none`), skips entries whose hash is unchanged, and reorders children with
a longest-increasing-subsequence pass. Keyboard focus lives on mirror elements; text fields are
edited natively and report value and selection back with `field_input`. See
[ADR 11](adr/0011-accessibility-mirror.md).

### Prerendering and take-over

With `prerender = true`, `vcwl build` runs `runtime/prerender.mjs` in Node: it instantiates
`app.wasm` and `render.wasm` with a stub text measurer, serves same-origin fetches from `dist/`,
and for each route calls the renderer's `prerender()` export, which serializes the tree as escaped
HTML plus the CSS for bound styles (`render/src/engine/html.rs`). The build wraps each route in a
section switched by `:target`, adds the CSS hash to the CSP and writes it into `index.html`.
With path routing each route is prerendered into its own page, and route links become relative
URLs (`../about/`; absolute under the site's base path in `404.html`).
In the browser the canvas stays hidden until its first frame; before that frame the host calls
`take_over(scrollY)`, so the renderer scrolls to the same place and shows `when appear` elements
in view without their intro transition. Then the static markup is removed.

### Server functions

`server fn` bodies are type-checked like other functions, but views and handlers cannot call
them; `fetch f(args)` in a handler becomes a `POST __vcwl/fn/f` whose body is the JSON array of
the arguments, decoded with `f`'s result type (`check_fetch`). After checking, the compiler
generates one entry per server function (`json.rs` `server_entry`): it decodes the arguments
from JSON handle 0, calls the function and passes the JSON of the result to the host's
`respond` (or `reject` on a mismatch). A second lowering pass exports only these entries, so
`server.wasm` holds the server functions and what they call, and `app.wasm` none of it.
`src/server.rs` runs `server.wasm` with wasmtime (GC proposal enabled): a fresh instance per
call from a pre-linked `InstancePre`, an epoch deadline, a memory limit, and host functions
mirroring the browser host's JSON and text I/O plus `env`, `store_get/set/delete`. See
[ADR 14](adr/0014-server-functions.md).

### 3D scenes

`scene`, `mesh`, `group` and `light` are ordinary elements of the renderer's tree, so state
bindings, events and diffing work unchanged. Layout treats the 3D elements as `display: none`;
painting emits one `SCENE` op (node id, box, corner radii) for the scene. The host asks the
renderer for `scene(id)`, a flat binary description (scene attributes, then each item's kind,
parent index, node id, clickability and attributes), and passes it to `runtime/scene3d.js`,
which is imported on first use. That module draws with WebGL into an offscreen canvas that the
host copies into the page canvas, clipped like other content; it keeps a requestAnimationFrame
loop only while something spins. A click without dragging renders a pick pass (one color per
clickable mesh) and calls the renderer's `activate_node`; dragging an `orbit` scene changes the
camera in the host only, as do the arrow keys while the scene or one of its meshes has focus.
Clickable meshes are focusable (Tab order follows the tree) and appear in the accessibility
mirror as buttons beside the scene's image; `abs()` gives 3D elements their scene's box. Models
(`.obj`, `.gltf`, `.glb`) and textures load asynchronously in `scene3d.js` and trigger a
repaint when ready. With `shadows`, a depth pass from the first directional light renders into
an RGBA-packed shadow map sampled with 3×3 PCF; translucent meshes draw after opaque ones, far
to near, without depth writes. See [ADR 15](adr/0015-built-in-3d.md).

### Transitions

`render/src/anim.rs`: when a restyle changes `opacity`, `transform`, `color` or a background color
of a node whose style has `transition`, an animation starts from the current painted value. At
each frame the animated values are written into the styles for painting and restored afterwards,
so style comparison and layout always see the target style. `when appear` rules match until the
element's flag `SEEN` is set, which happens the first frame its box is in the viewport.
## Components and reactivity

Each component instance is a GC struct: a dirty bitmask, one field per property/state/derived value,
node ids and child scopes. Conditional branches and list items are scopes of their own with
generated `mount`, `update`, `destroy`, `move` and `first` functions.

Dependencies are computed at compile time: every view expression knows which component values it
reads (bit *i* for value *i*). A handler marks the bits of the values it assigns; when it returns,
the component's `update` runs once and only re-evaluates expressions whose bits intersect the dirty
mask. There is no virtual tree diffing.

Keyed lists match new items to old fragments by key, destroy fragments whose key disappeared,
mount new ones, and move only fragments outside the longest increasing subsequence of old positions.

Content given for a component's slots is compiled as fragments of the user's scope. The user
stores, in the child's scope, its own scope and six functions over such a fragment (create
with the slot's value, mount, update, destroy, move, first node), with the fragment passed as
`anyref`; the child calls them wherever the slot appears, any number of times (inside `if` and
keyed `for`). When values the content reads change, the user sets `BIT_SLOT` in the child's
dirty mask, and each slot site refreshes its fragment.

Event listeners are registered by the host and identified by a *handler slot*; the module keeps the
slot table (scope and handler kind) in GC arrays and releases slots when elements are destroyed.
Pending `fetch` requests use the same table.

## Decisions

See [docs/adr](adr/).
