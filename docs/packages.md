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

## The registry

A registry is the `registry` branch of a git repository, in the spirit of Wally's index
repository: version `v` of package `name` is the directory `packages/<name>/<v>/` on that branch,
holding the package's files. This repository's own `registry` branch holds the packages in
`packages/` (`is-even`, `is-odd`, which depends on it, `left-pad`, `strings-plus` and `vibes`).

```toml
# vcwl.toml of the app
registry = "https://github.com/someone/vcwl"   # any git URL, or a path relative to this file

[dependencies]
is-odd = "^1.0"        # a version requirement: a registry package
kit = "../kit"         # anything else: a path
```

```sh
vcwl search [word]       # packages, latest versions and descriptions
vcwl add is-odd          # adds `is-odd = "^<latest>"` and installs it
vcwl add left-pad@~1.0   # with a requirement
vcwl install             # installs everything vcwl.toml asks for
vcwl remove is-odd
vcwl publish [--dry-run] # from a package's directory
```

- Requirements: `^1.2.3` or `1.2.3` (same first non-zero part), `~1.2.3` (same minor), `=1.2.3`,
  `*`; missing parts count as 0. Versions are `major.minor.patch`, without pre-releases.
- Installing resolves one version per package across the whole tree (the newest that every
  requirement allows, or the locked one), copies the packages into `vcwl_packages/` (flat, so
  packages' own registry dependencies come from there too), and writes `vcwl.lock`. Commit the
  lock; `vcwl_packages/` belongs in `.gitignore`. Commands that build (`check`, `build`, `dev`,
  `serve`, `test`) install missing packages first.
- `vcwl publish` needs `name` (lowercase letters, digits, `-`) and `version` in vcwl.toml,
  checks that the package's `lib` compiles, and refuses versions already published and path
  dependencies. It commits `packages/<name>/<version>/` (every file except hidden ones, `dist`,
  `dist-server`, `vcwl_packages`, `target` and `vcwl.lock`) on top of the branch and pushes it,
  so publishing needs push access to the repository: that is the whole permission model.
- Everything goes through the `git` command. Fetched registry branches are cached as bare
  repositories in `$VCWL_HOME` (default `~/.cache/vcwl`); without network, the cached branch is
  used with a warning. `$VCWL_REGISTRY` overrides the manifest's `registry`.

## Security review

Every registry package is reviewed before it is installed, `vcwl publish` shows its author the
same review, and `vcwl audit` reviews the installed packages again. The review reads the
package's files for what it could do to the app that installs it:

| finding | level |
| --- | --- |
| reads server environment variables (`env`), where secrets live | danger |
| uses persistent store keys outside its own `<name>.` prefix (it could read or overwrite the app's data) | danger |
| `public/` files that run code on the app's origin (`.html`, `.js`, `.svg`, `.wasm`, ...) | danger |
| `public/` files that change how the site is served (`_headers`, `_redirects`, `CNAME`, `.well-known/`, service workers, ...) | danger |
| symbolic links | danger |
| server functions (code that runs on the app's server) | notice |
| requests (`fetch`), clipboard writes, navigation to other sites | notice |
| large or hidden files | notice |

A package with a danger is not installed until the app trusts that exact version, so an update
is reviewed again:

```toml
[trust]
spy = "1.0.0"
```

What a package can reach, which is what the review looks for: VCWL runs no package code when
installing (there are no install scripts). Package code runs where the app's code runs: in the
browser under the app's capabilities (a package cannot widen the `network` allow list or the
CSP); while prerendering, compiled into the app's WebAssembly and run by Node.js with only the
host functions VCWL provides; and on the server inside server functions, which can read
environment variables and the app's whole store but have no file system or outgoing network
access. Installing also refuses file paths that would leave the package's directory. The review
is a static reading of the source, not a proof that a package is harmless: a package can still
do anything the app itself is allowed to do (show misleading content, send data to hosts the app
already allows, waste CPU). Read what you install.

## Not yet

No pre-release versions, yanking, checksums or signatures (the registry is trusted as far as its
git history is); two versions of one package cannot coexist in an app; two packages with the
same exported name in one file are a compile error rather than namespaced.
