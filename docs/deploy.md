# Deploying to GitHub Pages

`vcwl build` writes a static site to `dist/` (`index.html`, `app.wasm`, `app.css`, `vcwl.js`,
`boot.js`, `.nojekyll`). All paths are relative and routing uses the URL hash, so the site works
from any sub-path such as `https://user.github.io/repo/`.

GitHub Pages serves `.wasm` files as `application/wasm`, which lets the browser compile the module
while it downloads. Other hosts should do the same; the host falls back to a non-streaming
compile otherwise.

## With GitHub Actions

`.github/workflows/pages.yml`:

```yaml
name: pages
on:
  push:
    branches: [main]
permissions:
  contents: read
  pages: write
  id-token: write
jobs:
  deploy:
    runs-on: ubuntu-latest
    environment:
      name: github-pages
      url: ${{ steps.deployment.outputs.page_url }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo install --git <url of the vcwl repository> vcwl
      - run: vcwl build
      - uses: actions/upload-pages-artifact@v3
        with:
          path: dist
      - id: deployment
        uses: actions/deploy-pages@v4
```

Then select "GitHub Actions" as the Pages source in the repository settings.

## By hand

Commit the contents of `dist/` to a `gh-pages` branch and select it as the Pages source. Keep the
`.nojekyll` file so that files are served unchanged.
