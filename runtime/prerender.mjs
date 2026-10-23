// Prerenders a built application (`vcwl build` with `prerender = true`): runs app.wasm with
// render.wasm in Node.js for each [route, way to the site root] pair and writes, per pair, the
// extra style rules and the HTML, each followed by a NUL character. The root (path routing)
// turns route links into real URLs; it is empty for hash routing. Same-origin GET requests are served
// from the output directory; other requests fail as if offline.
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

const [dist, routesJson] = process.argv.slice(1);
const { createHost, reader, OUT, DESCRIPTION_MARK } = await import(pathToFileURL(path.join(dist, "vcwl.js")));
const FETCH_OK = 0, FETCH_NETWORK = 1, FETCH_STATUS = 3, FETCH_INVALID = 4;
const BASE = "http://prerender.invalid/";
const app = await WebAssembly.compile(fs.readFileSync(path.join(dist, "app.wasm")));
const render = await WebAssembly.compile(fs.readFileSync(path.join(dist, "render.wasm")));
const styles = new Uint8Array(WebAssembly.Module.customSections(app, "vcwl.styles")[0] ?? new ArrayBuffer(0));
const decoder = new TextDecoder();

async function page(route, root) {
  const host = createHost({ print: () => {} });
  let pending = 0, x, appTitle, title, description;
  // Layout is the browser's job here; widths only need to be plausible.
  const r = (await WebAssembly.instantiate(render, { host: { measure: (fp, fl, spacing, tp, tl) => tl * 8 } })).exports;
  const put = (bytes) => {
    const ptr = r.alloc(bytes.length);
    new Uint8Array(r.memory.buffer, ptr, bytes.length).set(bytes);
    return bytes.length;
  };
  r.init(put(styles), 1280, 800);
  const call = (fn) => {
    fn(x);
    for (let n = r.poll(); n; n = r.poll()) {
      const b = reader(r.memory, r.out_ptr(), n);
      while (b.more()) {
        switch (b.int()) {
          case OUT.EMIT: {
            const slot = b.int(), kind = b.int();
            const payload = kind === 1 ? b.str() : kind === 2 ? b.int() : 0;
            x.dispatch(slot, kind === 1 ? host.writeIO(payload) : payload);
            break;
          }
          case OUT.FETCH: {
            const slot = b.int(), method = b.int(), hasBody = b.int();
            const url = b.str();
            b.str();
            fetchFile(slot, method === 0 && !hasBody ? url : null);
            break;
          }
          case OUT.OPEN:
            b.str();
            b.int();
            break;
          case OUT.LOAD_IMAGE:
            b.int();
            b.str();
            break;
          case OUT.TITLE: {
            const s = b.str();
            if (s.startsWith(DESCRIPTION_MARK)) description = s.slice(DESCRIPTION_MARK.length);
            else if (appTitle === undefined) appTitle = s;
            else title = s;
            break;
          }
          case OUT.NAVIGATE:
          case OUT.WARN:
          case OUT.CURSOR:
          case OUT.COPY:
            b.str();
            break;
          default:
            throw new Error("corrupt renderer output");
        }
      }
    }
  };
  function fetchFile(slot, raw) {
    pending++;
    let kind = FETCH_NETWORK, status = 0, value = "not available while prerendering";
    const url = raw === null ? null : new URL(raw, BASE);
    if (url?.origin === new URL(BASE).origin) {
      const file = path.resolve(dist, "." + decodeURIComponent(url.pathname));
      if (file.startsWith(path.resolve(dist) + path.sep) && fs.existsSync(file)) {
        try {
          [kind, status, value] = [FETCH_OK, 200, JSON.parse(fs.readFileSync(file, "utf8"))];
        } catch (e) {
          [kind, status, value] = [FETCH_INVALID, 200, `invalid JSON: ${e.message}`];
        }
      } else [kind, status, value] = [FETCH_STATUS, 404, "not found"];
    }
    setImmediate(() => {
      pending--;
      call((x) => {
        host.json.length = 0;
        let len = 0;
        if (kind === FETCH_OK) host.json.push(value);
        else len = host.writeIO(value);
        x.fetch_done(slot, kind, status, len);
        host.json.length = 0;
      });
    });
  }
  host.imports.flush = (len) => {
    const err = r.apply(put(new Uint8Array(host.memory.buffer, 0, len)));
    if (err) throw new Error(decoder.decode(new Uint8Array(r.memory.buffer, r.out_ptr(), err)));
  };
  const imports = new Proxy(host.imports, { get: (t, k) => t[k] ?? (() => 0) });
  const instance = await WebAssembly.instantiate(app, { vcwl: imports });
  host.memory = instance.exports.memory;
  x = instance.exports;
  call((x) => x.set_route(host.writeIO(route)));
  call((x) => x.start());
  for (let i = 0; pending && i < 100; i++) await new Promise((r) => setImmediate(r));
  const n = r.prerender(put(new TextEncoder().encode(root)));
  let out = decoder.decode(new Uint8Array(r.memory.buffer, r.out_ptr(), n));
  // The page's metadata (`meta(...)`) as rendered for this route.
  if (title !== undefined && title !== appTitle) out += `<title>${escape(title)}</title>`;
  if (description !== undefined) out += `<meta name="description" content="${escape(description)}">`;
  return out;
}

function escape(s) {
  return s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
}

for (const [route, root] of JSON.parse(routesJson)) process.stdout.write(`${await page(route, root)}\0`);
