# VCWL language reference

This describes the language as implemented. Programs are UTF-8 files ending in `.vcwl`.

## Lexical structure

- Comments: `// line` and `/* block */`.
- Newlines end statements (`;` also works). An expression continues on the next line when the line
  ends with a binary operator or `=`, or when the next line starts with `.`.
- Literals: `42`, `-7`, `3.5`, `true`, `false`, `"text {expr}"` (interpolation; `\{` for a literal
  brace; string literals are not allowed inside `{}`), lengths `12px`, `1.5rem`, `50%`, durations
  `200ms`, colors `#rgb`, `#rrggbb`, `#rrggbbaa`. Length, duration and color literals only appear in styles.

## Types

| type | values | representation |
| --- | --- | --- |
| `Int` | 64-bit signed, wrapping arithmetic | `i64` |
| `Float` | IEEE 754 double | `f64` |
| `Bool` | `true`, `false` | `i32` |
| `String` | immutable UTF-8 text | GC byte array |
| `()` | unit | — |
| `[T]` | immutable array | GC array |
| `T?` | `Option<T>`: `Some(v)` or `None` | enum |
| `Result<T, E>` | `Ok(v)` or `Err(e)` | enum |
| user structs and enums | | GC structs |

Integer division truncates toward zero; division by zero traps. `Float` to `Int` conversion
(`f.to_int()`) saturates. There are no implicit numeric conversions.

All values are immutable. Only `var` locals and component `state` can be reassigned. Updating a
struct creates a new one:

```vcwl
let moved = Point { ..p, x: p.x + 1 }
let longer = items.push(4)        // arrays return new arrays
```

## Declarations

```vcwl
import "./geometry.vcwl"          // relative path; brings the file's `export`ed items into scope
import "@kit"                     // a package's main file (see docs/packages.md)
import "@kit/src/forms.vcwl"      // another file of a package

export struct Point {
    x: Int,
    y: Int,
}

export enum Shape {
    Circle(Float),
    Rect(Float, Float),
    Empty,
}

struct Box<T> {
    value: T,
}

export fn area(s: Shape) -> Float {
    match s {
        Circle(r) => 3.14159 * r * r,
        Rect(w, h) => w * h,
        Shape.Empty => 0.0,
    }
}
```

- Items are private to their file unless marked `export`. Local definitions shadow the prelude.
- Generic functions and types are checked once and compiled per instantiation.
- Variants can be written unqualified (`Circle(2.0)`) or qualified (`Shape.Empty`).
- The last expression of a block is its value; `return` exits early.

## Statements and expressions

```vcwl
let x = 1                         // immutable binding (type inferred)
var total: Int = 0                // mutable local
total += x                        // =, +=, -=, *=, /=
if a < b { a } else { b }         // if is an expression
while cond { ... }
for item in items { ... }         // arrays
for i in 0..10 { ... }            // ranges: 0..10 (exclusive), 0..=10 (inclusive)
break / continue / return value
match value { pattern => expr, ... }
```

Patterns: literals (`0`, `"x"`, `true`), bindings (`n`), `_`, variants with sub-patterns
(`Some(Circle(r))`). Matches must be exhaustive; the error names a missing case.

Operators by precedence: `|>`, `||`, `&&`, comparisons (`== != < <= > >=`), `+ -`, `* / %`, unary
`- !`, then calls, `.field`, `.method()`, `[index]`. `+` concatenates strings. `==` works on
numbers, booleans, strings and enums without payloads. Indexing is bounds-checked.

`x |> f(a, b)` is `f(x, a, b)` and `x |> f` is `f(x)`: the pipe passes its left side as the
first argument of a function. It binds loosest, and a line may start with it:

```vcwl
let score = input
    |> trim_all
    |> parse_or(0)
    |> clamp(0, 100)
```

`x?` unwraps `Ok(v)` / `Some(v)`; on `Err(e)` / `None` it returns that from the enclosing function,
which must return a `Result` with the same error type (or an optional):

```vcwl
fn total(parts: [String]) -> Result<Int, String> {
    var sum = 0
    for p in parts {
        sum += parse(p)?
    }
    Ok(sum)
}
```

## Methods (prelude)

Methods are prelude functions found by the receiver's type (`s.len()` calls `str_len(s)`).

- `String`: `len`, `is_empty`, `slice(start, end)`, `index_of`, `contains`, `starts_with`,
  `ends_with`, `trim`, `to_upper`, `to_lower`, `repeat`, `split`, `parse_int`.
- `Int`: `to_string`, `to_float`, `abs`, `min`, `max`, `clamp`.
- `Float`: `to_string`, `to_int`, `floor`, `ceil`, `round`, `sqrt`, `abs`, `min`, `max`.
- `Bool`: `to_string`.
- `[T]`: `len`, `is_empty`, `get` (→ `T?`), `first`, `last`, `push`, `set`, `remove_at`, `concat`,
  `slice`, `reverse`; `[String]`: `join`; `[Int]`: `sum`.
- `T?`: `is_some`, `is_none`, `unwrap_or`. `Result`: `is_ok`, `is_err`, `unwrap_or`, `ok`.
- `HttpError`: `message`.

Built-in functions: `print(String)`, `assert(Bool, String)`, `navigate(String)` and
`copy(String)` (writes to the clipboard); the last two only in event handlers.

## Programs and tests

`vcwl run file.vcwl` runs `fn main()`. In test files, every `fn test_*()` without parameters is a
test; it fails if it panics (`assert`, out-of-bounds index, division by zero).

## Applications

The entry file of a web app declares `app` and a root component named `App`:

```vcwl
app "My app" {
    capabilities {
        network: allow("self", "api.example.com")
        storage: none
    }
}
```

`app "Title"` alone declares no capabilities.

### Components

```vcwl
export component TodoItem(todo: Todo, compact: Bool = false) {
    event toggle(Int)              // events the parent can handle
    event remove

    state editing: Bool = false    // mutable, local to this instance
    let label = todo.title.trim()  // derived value, recomputed when its inputs change

    item {
        checkbox(label: label, checked: todo.done) {
            on change(v) {
                emit toggle(todo.id)
            }
        }
    }
}
```

Using a component: `TodoItem(todo: t)` with named properties (properties with defaults can be
omitted), and handlers for its events as children:

```vcwl
TodoItem(todo: t) {
    on toggle(id) {
        todos = toggle(todos, id)
    }
}
```

Properties are read-only. State changes only in event handlers; assigning in a view is a compile
error. After a handler finishes, the views that depend on changed values update in one batch.

A component can take child content with one `slot` in its view. Content given at the use site is
placed there; it belongs to the parent, so it reads the parent's state and its handlers run in the
parent:

```vcwl
component Card(title: String) {
    event close

    article {
        heading(title, level: 2)
        slot
        button("Close") {
            on click {
                emit close
            }
        }
    }
}

Card(title: "Inbox") {
    text("{unread} unread")      // parent state; updates when `unread` changes
    on close {                   // handlers for Card's events sit next to the content
        open = false
    }
}
```

Named slots take several pieces of content. `slot(name)` marks where each goes; at the use site,
`slot(name) { ... }` blocks fill them, in any order, and the remaining content goes to the
unnamed `slot`. A slot that is not filled stays empty:

```vcwl
component Panel {
    section {
        header {
            slot(header)
        }
        slot
        footer {
            slot(footer)
        }
    }
}

Panel {
    slot(header) {
        heading("Inbox", level: 3)
    }
    text("{unread} unread")
}
```

A slot can pass a value to the content filling it. The component declares the slot's type with
`slot name(Type)` and gives the value with `slot(name, value)`; the user names it with
`slot(name, x) { ... }`. Slots may sit inside `if`, `match` and `for`: the content is created
once per place the slot appears, so a list component can let its user draw each row:

```vcwl
component Names(names: [String]) {
    slot row(String)

    list {
        for s in names key s {
            item {
                slot(row, s)
            }
        }
    }
}

Names(names: people) {
    slot(row, who) {
        button("{who}") {
            on click {
                picked = who     // the user's state and handlers, as with other slots
            }
        }
    }
}
```

A slot's body is its fallback, shown when the user gives no content for it:
`slot(footer) { text("no footer") }`. A component can also forward its own slot into a child's
content (`Card { slot }`); there, `slot(name)` directly inside the child's content fills the
child's slot instead.

Each slot name (and the unnamed slot) appears at most once in a component's view. Passing content
for a slot the component does not have, filling one twice, naming the value of a slot that passes
none, or leaving out the value of one that does, is a compile error.

### Views

- Text: `text("Hello {name}")`.
- Layout: `column`, `row`, `stack`, `grid`, `box`, `scroll` with layout arguments `gap`, `align`,
  `justify`, `padding`, `wrap`, `columns`, `width`, `height`, `grow`.
- Semantic elements: `section`, `header`, `footer`, `nav`, `main`, `article`, `aside`, `list`,
  `ordered_list`, `item`, `paragraph`, `span`, `strong`, `code`, `heading(text, level: 1..6)`,
  `divider`.
- Every element takes `label:` (accessible name), `title:` and `id:`. `link(text, href: "#id")`
  scrolls to the element with that `id` (smoothly, unless reduced motion is preferred); `#/path`
  links are routes.
- Controls: `button(text, disabled:, pressed:, submit:)`, `link(text, to: "/route", current:)` or
  `link(text, href: url)` (`current: true` marks the link for the current page), `image(src:, alt:)` (`alt` required), `input(value:, label:,
  placeholder:, kind:)`, `textarea(value:, label:)`, `checkbox(label:, checked:)`,
  `select(options:, value:, label:)` (`label` is required on every form control), `form`
  (submits without reloading).
- Control flow: `if cond { ... } else { ... }`, `match value { pattern => view }`,
  `for item in items key item.id { ... }` (keys must be unique; dev builds warn on duplicates).
- Routing: `routes { "/" => Home() "/about" => About() _ => NotFound() }` uses the URL hash
  (`#/about`) by default, or real paths with `routing = "path"` in vcwl.toml
  ([deploy](deploy.md#real-urls-routing--path)). `route` is the current path; `link(to: "/about")`
  links to a route in either mode.

Events: `on click`, `on input(v)`, `on change(v)`, `on submit`, `on focus`, `on blur`,
`on dblclick`, and `on mount` in a component body.

### Fonts

```vcwl
font "Tuffy" from "fonts/tuffy-bold.ttf" weight 700
font "Inter" from "fonts/inter.woff2" weight 100..900   // variable font
font "Inter" from "fonts/inter-italic.woff2" italic
```

The file must exist under `public/` and be `.woff2`, `.woff`, `.ttf` or `.otf`. Styles then use
the family by name (`font: "Tuffy", sans-serif`). The page preloads the files; the host loads
them before the first frame (waiting at most 3 s, then falling back) and re-measures text when a
font arrives later.

### Styles

```vcwl
button("Save") {
    style {
        padding: 8px 12px
        radius: 6px
        background: #b6ff6c
        when hover {
            background: #9be04f
        }
        when viewport < 600px {
            padding: 6px 8px
        }
    }
    style Primary                  // a named `style Primary { ... }` declared at top level
    style if busy {                // applied while `busy` is true
        opacity: 0.5
    }
    style bind width: progress %   // one property from a number computed at run time
}
```

`style bind <property>: <expr> <unit>` sets a property with a single length (`width`, `height`,
`left`, `size`, `gap`, ...; the unit is `%`, `px`, `rem`, `em`, `vw` or `vh`) or a single number
(`opacity`, `grow`, `shrink`) from an `Int` or `Float` expression. It updates when the expression's
inputs change, and applies after the element's style blocks.

Property values are checked at compile time (units, colors, keywords). Properties: `color`,
`background`, `width`, `height`, `min-width`, `max-width`, `min-height`, `max-height`, `padding`,
`padding-x`, `padding-y`, `margin`, `margin-x`, `margin-y`, `gap`, `row-gap`, `column-gap`,
`border`, `radius`, `font`, `size`, `weight`, `line-height`, `letter-spacing`, `italic`,
`text-align`, `decoration`, `case`, `white-space`, `direction`, `align`, `align-self`, `justify`,
`wrap`, `grow`, `shrink`, `basis`, `display`, `columns`, `rows`, `span`, `position`, `top`,
`right`, `bottom`, `left`, `inset`, `z`, `overflow`, `overflow-x`, `overflow-y`, `opacity`,
`transform`, `transition`, `cursor`, `shadow`, `fit`, `container`, `user-select`, `list-style`.

Beyond CSS (drawn by the canvas renderer; browsers ignore them in prerendered pages):

- `corner: round | squircle | cut | scoop`: the shape of the `radius` corners (smooth
  superellipse-like curves, straight bevels, or concave scoops); backgrounds, borders, shadows
  and clipping all follow it.
- `grain: 0.4`: film grain over the background (0 to 1).
- `sketch: 2px #222`: a hand-drawn border instead of `border` (wobbly, overshooting double
  strokes that stay the same from frame to frame).
- `spring` and `bounce` easings for `transition`: a damped spring that overshoots and settles,
  and a ball bouncing to rest (`transition: transform 600ms spring`).

`when` conditions: `viewport < | <= | > | >= length`, `container ...`, `hover`, `focus`,
`focus-within`, `active`, `disabled`, `checked`, `appear`, `dark`, `light`, `reduced-motion`,
`print`. `when appear` styles apply until the element first scrolls into view; with a
`transition`, the element then animates to its normal style (a scroll reveal):

```vcwl
style {
    transition: opacity 600ms ease-out, transform 600ms ease-out
    when appear {
        opacity: 0
        transform: translateY(16px)
    }
}
```

Each style block becomes a class named after a hash of its contents. The renderer supports the
properties above with these exceptions: `transition` animates `opacity`, `transform`, `color` and
`background` colors (other properties change at once, and nothing animates when reduced motion is
preferred), `position: sticky` behaves as `relative`, and `display: inline` elements take no
padding, border or background.

### Fetch

```vcwl
on click {
    loading = true
    fetch get("/api/items") as [Item] {
        Ok(items) => {
            list = items
            loading = false
        }
        Err(Status(404, _)) => {
            error = "not found"
        }
        Err(e) => {
            error = e.message()
        }
    }
}
```

- Methods: `get`, `post`, `put`, `patch`, `delete`; `post/put/patch` take an optional body,
  `fetch post(url, value) as T`, encoded as JSON.
- Decodable and encodable types: `Int`, `Float`, `Bool`, `String`, `()`, arrays, optionals,
  structs of these, and enums without payloads (as their variant name). Missing or `null` fields
  decode as `None`; unknown fields are ignored; integers must be exact.
- The arms receive `Result<T, HttpError>` where
  `enum HttpError { Network(String), Timeout, Status(Int, String), Decode(String) }`.
- The arms run when the response arrives, after the handler returned. They can read and assign
  component values but not the handler's local variables. If the component was removed in the
  meantime, the response is discarded.
- `fetch` requires a `network` capability; the host refuses other origins.

### Server functions

```vcwl
struct Entry {
    name: String,
    text: String,
}

server fn sign(name: String, text: String) -> [Entry] {
    assert(text.len() > 0, "empty message")
    let all = load<[Entry]>("entries").unwrap_or([]).push(Entry { name: name, text: text })
    save("entries", all)
    all
}

component App {
    state list: [Entry] = []

    button("Sign") {
        on click {
            fetch sign("Ada", "hello") {
                Ok(entries) => {
                    list = entries
                }
                Err(e) => {}
            }
        }
    }
}
```

- A `server fn` runs on the server ([`vcwl serve`](deploy.md#full-stack-vcwl-serve)). The
  client calls it from an event handler with `fetch name(args) { arms }`: the arguments are sent
  as JSON, the result arrives as `Result<T, HttpError>` like any `fetch`. Calling it directly
  outside server code is a compile error.
- Parameters and results use the same JSON types as `fetch`; the server checks the arguments at
  run time (a mismatch is HTTP 400), so a client cannot pass values of the wrong shape.
- Only server functions can use `env(name) -> String?` (environment variables), and the
  persistent store: `save(key, value)`, `load<T>(key) -> T?` (`None` when missing or of another
  shape) and `remove(key)`.
- Server functions can call ordinary functions and each other. Their code, and what they call,
  is compiled into `dist-server/server.wasm` only; `app.wasm` contains just the call's URL.
- A panic, trap or call running longer than 5 s is HTTP 500 for the client; the details go to
  the server's log.

### 3D scenes

```vcwl
component Viewer {
    state hits: Int = 0

    scene(label: "A clickable box", camera: vec3(0.0, 1.5, 5.0), orbit: true) {
        light(kind: "ambient", intensity: 0.3)
        light(position: vec3(3.0, 5.0, 4.0))
        mesh(shape: "box", label: "Box", rotation: vec3(20.0, 30.0, 0.0), color: "#d16a47") {
            on click {
                hits += 1
            }
        }
        group(position: vec3(1.6, 0.0, 0.0), spin: vec3(0.0, 90.0, 0.0)) {
            mesh(shape: "torus", color: "#b6ff6c")
        }
    }
}
```

- `scene` is a block element (300px high by default, styled like any element) that draws a 3D
  view with WebGL. `label` is required: it is the scene's accessible name (`role="img"`).
  Arguments: `camera` and `target` (`Vec3`, default `vec3(0, 1.5, 5)` looking at the origin),
  `fov` in degrees (50), `background` (a CSS color), `orbit` (dragging, or the arrow keys when
  the scene or one of its meshes has focus, turns the camera around the target), `shadows`
  (meshes cast shadows from the first directional or point light).
- Inside a scene only `mesh`, `light` and `group` are allowed, and they are allowed nowhere
  else. Positions are `Vec3` values (`vec3(x, y, z)`, y up); rotations and `spin` are in
  degrees and degrees per second around x, y and z.
- `mesh`: `shape` (`box`, `sphere`, `plane`, `cylinder`, `cone`, `torus`) or `model` (a
  Wavefront `.obj`, glTF `.gltf` or binary `.glb` file of the app, centered and scaled to fit
  a unit cube; glTF materials are shaded as metallic-roughness materials with their base
  color, metallic-roughness, normal, emissive and occlusion maps, and glTF node animations
  play), `animation` (the name of the glTF animation to loop; the first one by default,
  `"none"` for the rest pose), `size` (scale, default 1 unit), `position`,
  `rotation`, `spin`, `color`, `texture` (an image; multiplied by `color`, white by default),
  `opacity` (0 to 1; overlapping translucent meshes blend in depth order per pixel, up to
  four surfaces deep), `shininess` (32; 0 for matte; not for glTF materials), `wireframe`,
  and `on click`, fired when a
  click without dragging hits the mesh.
- A mesh with `on click` requires a `label`: keyboard users reach it with Tab and press Enter
  or Space, and screen readers see it as a button named by the label. The focused mesh is
  highlighted.
- `group` moves, rotates (`rotation`, `spin`) and scales (`scale`) its children together.
- `light`: `kind` (`directional`, the default, `point` or `ambient`), `position`, `color`,
  `intensity`. A scene without lights gets an ambient and a directional light.
- Arguments can be bound to state; changing it redraws the scene. Spinning and model animations
  stop when the user prefers reduced motion. Prerendered HTML shows an empty box where the scene will be.

### JavaScript interop

An `extern fn` has no body: it is implemented in JavaScript. Its parameters and result must be
types that serialize (like server function arguments); they cross the boundary as JSON.

```vcwl
extern fn shout(text: String) -> String
extern fn wait_for(ms: Int) -> Bool
```

- An ordinary call (`shout("hi")`) is synchronous: the JavaScript function must return its
  result directly.
- A function returning a Promise is called from an event handler with `fetch`, like a server
  function: `fetch wait_for(500) { Ok(b) => ... Err(e) => ... }`. A rejection reaches `Err` with
  its message. `fetch rows("todos") as [Todo] { ... }` decodes the result as another type than
  the declared one, checked when it arrives like a response (server functions' results cannot
  be retyped).
- `to_json(value)` encodes any serializable value as JSON text, for JavaScript functions that
  take JSON.
- Extern functions run in the browser: calling one from a `server fn` is an error.

The app names its module in `vcwl.toml` with `js = "src/interop.js"`; packages can too (their
modules are loaded first, so the app's wins on equal names). It is copied to `dist` and loaded
before the app starts; its default export (an object) and its named exports are
the implementations, by name. `this` is `{ caps }`, the app's capabilities.

#### `@web`

`import "@web"` brings about 75 extern functions over the browser's Web APIs (the list with
their documentation is `runtime/web.vcwl`):

- URL and history: `location`, `origin`, `pathname`, `url_hash`, `url_param` (query, then
  fragment), `url_params`, `replace_url`, `push_url`, `history_back`, `redirect`, `reload`,
  `encode_uri`, `decode_uri`, `with_params`.
- Windows and messages: `open_window`, `popup_redirect` (async: resolves with the popup's URL
  once it returns to this origin, for OAuth), `next_message` (async, `postMessage`),
  `post_to_opener`, `close_window`, `has_opener`.
- Storage, checked against the `storage` capability (`local` allows both stores and cookies,
  `session` only sessionStorage): `local_get/set/remove`, `session_get/set/remove`,
  `cookie_get/set`.
- Time and Intl: `epoch_ms`, `now_iso`, `time_zone`, `format_date`, `format_number`,
  `format_currency`, `relative_time`, `format_list`, `sleep` (async).
- Web Crypto and encoding: `random`, `random_int`, `uuid`, `random_token`, `sha256_base64url`
  and `sha256_hex` (async), `base64_encode/decode`, `jwt_payload` (decoded, not verified).
- Page and device: `title`, `set_title`, `user_agent`, `language(s)`, `online`, `media_matches`,
  viewport size, `device_pixel_ratio`, `vibrate`, `speak`, `download`, `console_*`, `alert`,
  `confirm`, `prompt`, and async `read_clipboard`, `write_clipboard`, `share`, `notify`, `geolocation`,
  `fullscreen`, `exit_fullscreen`, `battery_level`.

An OAuth PKCE sign-in, for instance: `random_token(32)` for the verifier, `fetch
sha256_base64url(verifier)` for the challenge, `fetch popup_redirect(authorize_url, "")`, then
read `code` from the returned URL. Anything missing from `@web`: declare an `extern fn` and
implement it in the app's module.
