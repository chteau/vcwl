# Packages and frameworks

A package is a VCWL project whose files other projects import. It can hold anything a project
can: components (with slots and events), named styles, functions, types, server functions,
fonts, and static files. A "framework" is a package that gives apps their structure: layout
components, conventions, and server functions they call.

## Using a package

```toml
# vcwl.toml of the app
entry = "src/main.vcwl"

[dependencies]
kit = "../kit"          # path to the package, relative to this file
```

```vcwl
import "@kit"                    // the package's `lib` file
import "@kit/src/extra.vcwl"     // or any of its files
```

`@name` imports the file named by `lib` in the package's own vcwl.toml (default `src/lib.vcwl`)
and brings its `export`ed items into scope like any import. Packages declare their own
`[dependencies]`; an `@name` import in a package file resolves against the package's manifest.
`vcwl dev` rebuilds when a package's files change.

At build time, each package's `public/` directory is copied into `dist/` before the app's own,
so an app can override any of those files. A package's server functions are compiled into the
app's `dist-server/server.wasm` like the app's own; their store keys share one store, so
prefix them (`"kit.visits"`).

## Writing one

```toml
# vcwl.toml of the package
name = "kit"
lib = "src/lib.vcwl"
```

`examples/kit` is a small package (a page layout, a card with a slot, a button with an event,
a server-side visit counter) and `examples/kit/demo` an app built from it:

```sh
cd examples/kit/demo && vcwl serve
```

Only `export`ed items are visible to apps. Imports are not re-exported: an app imports each file
it uses (or the package keeps its public items in its `lib` file).

## Not yet

Packages are local paths: there is no registry, version resolution or lock file, and two
packages with the same exported name in one file are a compile error rather than namespaced.
