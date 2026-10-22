# Deploying to GitHub Pages

`vcwl build` writes a static site to `dist/` (`index.html`, `app.wasm`, `render.wasm`, `vcwl.js`,
`boot.js`, `.nojekyll`), and copies the project's `public/` folder (images, data, icons) into it
as is. All paths are relative and routing uses the URL hash by default, so the site works
from any sub-path such as `https://user.github.io/repo/`.

## Full stack (`vcwl serve`)

Apps with server functions (`server fn`) need a server: `vcwl serve [--port N] [--host ADDR]`
makes a release build and serves `dist/` together with the functions in
`dist-server/server.wasm` (default `127.0.0.1:8080`). `vcwl dev` runs them too, reloaded after
each rebuild. `save`/`load` keep their data in `data/store.json` (`store = "..."` in
vcwl.toml), written atomically after each call that changes it.

The site's static files still work on any static host; only the function calls need `vcwl serve`.
Behind a reverse proxy, forward everything (or at least `/__vcwl/fn/`) to it; it speaks plain
HTTP/1.1 and should not be exposed without TLS in front.

## Real URLs (`routing = "path"`)

```toml
routing = "path"
url = "https://user.github.io/repo/"   # needed for 404.html under a sub-path; adds a sitemap
```

Routes then have real URLs (`/repo/docs/intro/` instead of `/repo/#/docs/intro`). The build writes
one page per route (`docs/intro/index.html`), so static hosts serve every route directly, and a
`404.html` that GitHub Pages and most static hosts return for unknown paths; it starts the app on
its fallback route (`_ =>`). Links use the History API, the back button works, and old `#/` links
are moved to their path. With `url` set, each page gets a canonical link and `sitemap.xml` lists
them. Combined with `prerender = true`, every page holds its own prerendered HTML.

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

## Page metadata

`vcwl.toml` can describe the page for search engines and link previews:

```toml
description = "One sentence about the site."
theme_color = "#d16a47"
icon = "logo.svg"          # a file in public/
image = "og.png"           # preview image, a file in public/
url = "https://user.github.io/repo/"   # makes og:url and an absolute og:image
```

They become `<meta name="description">`, Open Graph and Twitter card tags, `theme-color` and the
page icon.

## Prerendering

```toml
prerender = true
```

`vcwl build` then runs the app once per route at build time (with Node.js, which must be
installed) and writes the resulting element tree into `index.html` as plain HTML with a generated
style sheet. Browsers show it before any WebAssembly has loaded, and crawlers that do not run
JavaScript index the full text of every route. Routes are switched with CSS `:target`, so the
static page works without JavaScript. When the canvas has drawn its first frame it replaces the
static page, keeping the scroll position. See [ADR 12](adr/0012-prerendering.md).

Prerendering waits for fetches to same-origin files in `dist/` (they are read from disk); requests
to other hosts are not made at build time and render as their loading state.
