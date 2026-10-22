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

Operators by precedence: `||`, `&&`, comparisons (`== != < <= > >=`), `+ -`, `* / %`, unary
`- !`, then calls, `.field`, `.method()`, `[index]`. `+` concatenates strings. `==` works on
numbers, booleans, strings and enums without payloads. Indexing is bounds-checked.

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
  (`#/about`), so it works on static hosts. `route` is the current path.

Events: `on click`, `on input(v)`, `on change(v)`, `on submit`, `on focus`, `on blur`,
`on dblclick`, and `on mount` in a component body.

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
