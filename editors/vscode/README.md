# VCWL for VS Code

Syntax highlighting for `.vcwl` files, and through `vcwl lsp`: compile errors and lints as you
type, completion of keywords, elements and style properties, and Format Document (`vcwl fmt`).

Install the compiler (`cargo install --path .` in the VCWL repository) so that `vcwl` is on your
PATH, or set `vcwl.path`. Then, from this directory:

```sh
npx @vscode/vsce package     # makes vcwl-0.1.0.vsix
code --install-extension vcwl-0.1.0.vsix
```

or open this folder in VS Code and press F5 to try it in an Extension Development Host.
