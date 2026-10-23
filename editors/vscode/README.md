# VCWL for VS Code

Language support for [VCWL](https://vcwl.chteau.bzh/), the Vibe Coded Web Language: a
statically typed language that compiles web apps to WebAssembly and draws them on a canvas.
It looks like Rust. It is not Rust.

![VCWL](https://vcwl.chteau.bzh/og.png)

## Features

- Syntax highlighting for `.vcwl` files.
- Compile errors and lints as you type.
- Completion of keywords, elements and style properties.
- Format Document (`vcwl fmt`).

Everything but the highlighting comes from the compiler's language server (`vcwl lsp`).

## Requirements

Install the `vcwl` compiler so it is on your PATH:

```sh
curl -fsSL https://vcwl.chteau.bzh/install.sh | sh     # Linux, macOS
irm https://vcwl.chteau.bzh/install.ps1 | iex           # Windows (PowerShell)
```

## Settings

- `vcwl.path`: the `vcwl` binary to run (default: `vcwl` from your PATH).

## Links

- [Documentation](https://vcwl.chteau.bzh/docs/install)
- [Source and issues](https://github.com/chteau/vcwl)
