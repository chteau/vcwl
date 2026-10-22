// Minimal static file server for browser tests (serves `root` under an optional base path).
// `api(req, res, path)` may handle a request first by returning true.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";

const TYPES = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".wasm": "application/wasm", ".json": "application/json", ".svg": "image/svg+xml", ".png": "image/png" };

export function serve(root, base = "/", api = null) {
  const server = http.createServer((req, res) => {
    const url = new URL(req.url, "http://x");
    if (!url.pathname.startsWith(base)) {
      res.writeHead(404).end();
      return;
    }
    let rel = decodeURIComponent(url.pathname.slice(base.length)) || "index.html";
    if (api?.(req, res, rel)) return;
    if (rel.endsWith("/")) rel += "index.html";
    const file = path.join(root, path.normalize(rel));
    if (!file.startsWith(path.resolve(root))) {
      res.writeHead(403).end();
      return;
    }
    // Like static hosts: `/about` → `/about/`, unknown paths get 404.html if there is one.
    if (fs.statSync(file, { throwIfNoEntry: false })?.isDirectory()) {
      res.writeHead(301, { location: url.pathname + "/" }).end();
      return;
    }
    fs.readFile(file, (err, data) => {
      if (err) {
        fs.readFile(path.join(root, "404.html"), (err2, page) => (err2 ? res.writeHead(404).end() : res.writeHead(404, { "content-type": "text/html" }).end(page)));
        return;
      }
      res.writeHead(200, { "content-type": TYPES[path.extname(file)] ?? "application/octet-stream" }).end(data);
    });
  });
  return new Promise((resolve) => server.listen(0, "127.0.0.1", () => resolve({ server, url: `http://127.0.0.1:${server.address().port}${base}` })));
}
