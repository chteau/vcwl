// VCWL host runtime.
//
// Loads a compiled VCWL module and provides the small set of host operations it
// imports. Application logic lives in WebAssembly; this file only bridges to
// browser (or Node.js) APIs. It contains no application code.

const IO_BASE = 65536;
const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: false });

export class VcwlPanic extends Error {
  constructor(message) {
    super(message);
    this.name = "VcwlPanic";
  }
}

/** Creates the import object and late-bound accessors shared by every host flavour. */
export function createHost(opts = {}) {
  const host = {
    memory: null,
    print: opts.print ?? ((s) => console.log(s)),
    readIO(len) {
      return decoder.decode(new Uint8Array(host.memory.buffer, IO_BASE, len));
    },
    writeIO(str) {
      const bytes = encoder.encode(str);
      const need = IO_BASE + bytes.length;
      if (need > host.memory.buffer.byteLength) {
        host.memory.grow(Math.ceil((need - host.memory.buffer.byteLength) / 65536));
      }
      new Uint8Array(host.memory.buffer, IO_BASE, bytes.length).set(bytes);
      return bytes.length;
    },
  };
  host.imports = {
    print: (len) => host.print(host.readIO(len)),
    panic: (len) => {
      throw new VcwlPanic(host.readIO(len));
    },
    fmt_f64: (x) => host.writeIO(formatFloat(x)),
    now: () => (globalThis.performance ?? Date).now(),
    // JSON values parsed by the host, read by generated decoders through integer handles.
    // Handle 0 is the response being delivered; handles are dropped after delivery.
    json_kind: (h) => jsonKind(json[h]),
    json_bool: (h) => (json[h] ? 1 : 0),
    json_num: (h) => json[h],
    json_str: (h) => host.writeIO(json[h]),
    json_len: (h) => json[h].length,
    json_index: (h, i) => json.push(json[h][i]) - 1,
    json_field: (h, len) => {
      const key = host.readIO(len), obj = json[h];
      return json.push(Object.hasOwn(obj, key) ? obj[key] : undefined) - 1;
    },
    json_num_text: (x) => host.writeIO(JSON.stringify(x)),
    json_quote: (len) => host.writeIO(JSON.stringify(host.readIO(len))),
  };
  const json = (host.json = []);
  return host;
}

/** -1 missing, 0 null, 1 boolean, 2 number, 3 string, 4 array, 5 object. */
function jsonKind(v) {
  if (v === undefined) return -1;
  if (v === null) return 0;
  switch (typeof v) {
    case "boolean": return 1;
    case "number": return 2;
    case "string": return 3;
    default: return Array.isArray(v) ? 4 : 5;
  }
}

/** Shortest round-trip decimal text; integral values keep a `.0` suffix. */
export function formatFloat(x) {
  if (Number.isInteger(x) && Math.abs(x) < 1e21) return x.toFixed(1);
  return String(x);
}

export async function instantiate(bytes, host, extraImports = {}) {
  const { instance } = await WebAssembly.instantiate(bytes, { vcwl: { ...host.imports, ...extraImports } });
  host.memory = instance.exports.memory;
  return instance;
}

/** Describes a runtime failure (trap or panic) for display. */
export function describeError(e) {
  if (e instanceof VcwlPanic) return `panic: ${e.message}`;
  if (e instanceof WebAssembly.RuntimeError) return `runtime error: ${e.message}`;
  return `host error: ${e && e.message ? e.message : e}`;
}

// ---------------------------------------------------------------------------
// Browser host: applies DOM commands produced by the WebAssembly module.

const OP = {
  CREATE: 1, TEXT: 2, COMMENT: 3, INSERT: 4, REMOVE: 5, SET_TEXT: 6, SET_ATTR: 7, REMOVE_ATTR: 8,
  SET_PROP: 9, SET_CHECKED: 10, LISTEN: 11, CLASS: 12, TOGGLE_CLASS: 13, TITLE: 14, NAVIGATE: 15,
  WARN: 16, CLEAR: 17, FETCH: 18,
};

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE"];
// Result kinds passed to `fetch_done`.
const FETCH_OK = 0, FETCH_NETWORK = 1, FETCH_TIMEOUT = 2, FETCH_STATUS = 3, FETCH_INVALID = 4;

const URL_ATTRS = new Set(["href", "src", "action", "formaction"]);
const SAFE_URL = /^(?:(?:https?|mailto|tel):|[^a-z]|[a-z+.-]+(?:[^a-z+.\-:]|$))/i;

/** Rejects `javascript:` and other script-capable URLs in URL attributes. */
export function sanitizeUrl(value) {
  const v = String(value).trim();
  return SAFE_URL.test(v) ? v : "about:invalid#vcwl-blocked";
}

/**
 * Creates a DOM host bound to `root`. Returns the host plus helpers used by tests.
 * `dispatch(slot, len)` is supplied once the module is instantiated.
 */
export function createDomHost(root, opts = {}) {
  const host = createHost(opts);
  const nodes = [root];
  const nodeIds = new WeakMap();
  const listeners = new Map(); // id -> [type, fn][]
  let exports = null;
  let busy = false;
  const queue = [];
  host.dead = false;
  host.stats = { mutations: 0, flushes: 0 };

  const report = opts.onError ?? ((msg) => console.error(msg));
  const win = root.ownerDocument.defaultView;

  /** Resolves `raw` against the page and checks it against the app's network capability. */
  function allowedUrl(raw) {
    let url;
    try {
      url = new URL(raw, root.ownerDocument.baseURI);
    } catch {
      return [null, `invalid URL "${raw}"`];
    }
    const hosts = opts.caps?.network ?? [];
    if (url.origin === win.location.origin && hosts.includes("self")) return [url, null];
    if (url.protocol === "https:" && hosts.includes(url.host)) return [url, null];
    return [null, `${url.origin} is not allowed by the app's network capability`];
  }

  /** Performs a request with the browser's fetch and reports the outcome to `fetch_done`. */
  async function startFetch(slot, method, raw, body) {
    const finish = (kind, status, value) =>
      host.call((x) => {
        const json = host.json;
        json.length = 0;
        let len = 0;
        if (kind === FETCH_OK) json.push(value);
        else len = host.writeIO(value);
        try {
          x.fetch_done(slot, kind, status, len);
        } finally {
          json.length = 0;
        }
      });
    const [url, refused] = allowedUrl(raw);
    if (refused) return finish(FETCH_NETWORK, 0, refused);
    const init = { method, headers: { accept: "application/json" }, signal: AbortSignal.timeout(opts.fetchTimeout ?? 30000) };
    if (body !== null) {
      init.body = body;
      init.headers["content-type"] = "application/json";
    }
    let resp, text;
    try {
      resp = await win.fetch(url, init);
      text = await resp.text();
    } catch (e) {
      return finish(e.name === "TimeoutError" ? FETCH_TIMEOUT : FETCH_NETWORK, resp?.status ?? 0, e.message);
    }
    if (!resp.ok) return finish(FETCH_STATUS, resp.status, text.slice(0, 4096));
    if (text.trim() === "") return finish(FETCH_OK, resp.status, undefined);
    let value;
    try {
      value = JSON.parse(text);
    } catch (e) {
      return finish(FETCH_INVALID, resp.status, `invalid JSON: ${e.message}`);
    }
    finish(FETCH_OK, resp.status, value);
  }

  function fail(e) {
    host.dead = true;
    report(describeError(e));
  }

  /** Calls into WebAssembly, deferring events raised synchronously by DOM updates. */
  host.call = (fn) => {
    if (host.dead) return;
    if (busy) {
      queue.push(fn);
      return;
    }
    busy = true;
    try {
      fn(exports);
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
    while (queue.length && !host.dead) host.call(queue.shift());
  };

  function forget(node) {
    const walker = node.ownerDocument.createTreeWalker(node, 0xffffffff);
    for (let n = node; n; n = walker.nextNode()) {
      const id = nodeIds.get(n);
      if (id === undefined) continue;
      const ls = listeners.get(id);
      if (ls) {
        for (const [type, fn] of ls) n.removeEventListener(type, fn);
        listeners.delete(id);
      }
      nodes[id] = undefined;
    }
  }

  function listen(id, spec, slot) {
    const el = nodes[id];
    const [type, payload] = spec.split(":");
    const fn = (ev) => {
      if (payload === "prevent") ev.preventDefault();
      host.call((x) => {
        let len = 0;
        if (payload === "value") len = host.writeIO(el.value);
        else if (payload === "key") len = host.writeIO(ev.key);
        else if (payload === "checked") len = el.checked ? 1 : 0;
        x.dispatch(slot, len);
      });
    };
    el.addEventListener(type, fn);
    if (!listeners.has(id)) listeners.set(id, []);
    listeners.get(id).push([type, fn]);
  }

  host.imports.flush = (len) => {
    const mem = host.memory.buffer;
    const view = new DataView(mem);
    const bytes = new Uint8Array(mem);
    let p = 0;
    const int = () => {
      const v = view.getInt32(p, true);
      p += 4;
      return v;
    };
    const str = () => {
      const n = int();
      const s = decoder.decode(bytes.subarray(p, p + n));
      p += (n + 3) & ~3;
      return s;
    };
    const doc = root.ownerDocument;
    host.stats.flushes++;
    while (p < len) {
      const op = int();
      host.stats.mutations++;
      switch (op) {
        case OP.CREATE: {
          const id = int();
          const el = doc.createElement(str());
          nodes[id] = el;
          nodeIds.set(el, id);
          break;
        }
        case OP.TEXT: {
          const id = int();
          const t = doc.createTextNode(str());
          nodes[id] = t;
          nodeIds.set(t, id);
          break;
        }
        case OP.COMMENT: {
          const id = int();
          const c = doc.createComment("");
          nodes[id] = c;
          nodeIds.set(c, id);
          break;
        }
        case OP.INSERT: {
          const parent = nodes[int()], node = nodes[int()], anchor = int();
          parent.insertBefore(node, anchor ? nodes[anchor] : null);
          break;
        }
        case OP.REMOVE: {
          const node = nodes[int()];
          forget(node);
          node.remove();
          break;
        }
        case OP.SET_TEXT: {
          const node = nodes[int()];
          node.data = str();
          break;
        }
        case OP.SET_ATTR: {
          const el = nodes[int()];
          const name = str(), value = str();
          if (/^on/i.test(name)) throw new Error(`refusing to set event-handler attribute ${name}`);
          el.setAttribute(name, URL_ATTRS.has(name) ? sanitizeUrl(value) : value);
          break;
        }
        case OP.REMOVE_ATTR: {
          const el = nodes[int()];
          el.removeAttribute(str());
          break;
        }
        case OP.SET_PROP: {
          const el = nodes[int()];
          const name = str(), value = str();
          if (name !== "value") throw new Error(`refusing to set property ${name}`);
          if (el.value !== value) el.value = value;
          break;
        }
        case OP.SET_CHECKED: {
          const el = nodes[int()];
          el.checked = int() !== 0;
          break;
        }
        case OP.LISTEN: {
          const id = int(), slot = int();
          listen(id, str(), slot);
          break;
        }
        case OP.CLASS: {
          const el = nodes[int()];
          el.className = str();
          break;
        }
        case OP.TOGGLE_CLASS: {
          const el = nodes[int()];
          const on = int() !== 0;
          el.classList.toggle(str(), on);
          break;
        }
        case OP.TITLE:
          doc.title = str();
          break;
        case OP.NAVIGATE: {
          const path = str();
          if (opts.navigate) opts.navigate(path);
          else doc.defaultView.location.hash = "#" + path;
          break;
        }
        case OP.WARN:
          console.warn("[vcwl] " + str());
          break;
        case OP.FETCH: {
          const slot = int(), method = METHODS[int()], hasBody = int();
          const url = str(), body = str();
          startFetch(slot, method, url, hasBody ? body : null);
          break;
        }
        case OP.CLEAR: {
          const el = nodes[int()];
          for (const c of [...el.childNodes]) forget(c);
          el.replaceChildren();
          break;
        }
        default:
          throw new Error(`corrupt command buffer (opcode ${op} at ${p - 4})`);
      }
    }
  };

  host.attach = (instance) => {
    exports = host.exports = instance.exports;
  };
  host.liveNodes = () => nodes.reduce((n, x, i) => n + (i > 0 && x !== undefined ? 1 : 0), 0);
  host.liveListeners = () => [...listeners.values()].reduce((n, l) => n + l.length, 0);
  host.setRoute = (path) => host.call((x) => x.set_route(host.writeIO(path)));
  return host;
}

function routeFromLocation(loc) {
  const h = decodeURIComponent(loc.hash.replace(/^#/, ""));
  return h.startsWith("/") ? h : "/";
}

/** Shows a runtime error on the page (dev builds) using safe text insertion. */
/** Shows `msg` in a fixed overlay (development builds); returns the overlay element. */
export function showError(doc, msg) {
  const box = doc.createElement("pre");
  box.setAttribute("role", "alert");
  box.style.cssText = "position:fixed;inset:auto 1rem 1rem 1rem;z-index:2147483647;margin:0;padding:1rem;" +
    "background:#2b0b0b;color:#ffd7d7;border:1px solid #ff6b6b;border-radius:8px;font:13px/1.4 ui-monospace,monospace;white-space:pre-wrap";
  box.textContent = "VCWL " + msg;
  doc.body.append(box);
  return box;
}

/** Loads `app.wasm` next to this script and starts the application in `document.body`. */
export async function boot(doc = document) {
  const cfgEl = doc.getElementById("vcwl-config");
  const cfg = cfgEl ? JSON.parse(cfgEl.textContent) : {};
  const report = (msg) => {
    console.error("[vcwl] " + msg);
    if (cfg.dev) showError(doc, msg);
  };
  const root = doc.body;
  const host = createDomHost(root, { onError: report, caps: cfg.caps });
  try {
    const url = new URL(cfg.wasm ?? "app.wasm", import.meta.url);
    const resp = await fetch(url);
    if (!resp.ok) throw new Error(`cannot load ${url.pathname}: HTTP ${resp.status}`);
    let result;
    if (WebAssembly.instantiateStreaming && resp.headers.get("content-type")?.startsWith("application/wasm")) {
      result = await WebAssembly.instantiateStreaming(resp, { vcwl: host.imports });
    } else {
      result = await WebAssembly.instantiate(await resp.arrayBuffer(), { vcwl: host.imports });
    }
    host.memory = result.instance.exports.memory;
    host.attach(result.instance);
  } catch (e) {
    const unsupported = typeof WebAssembly === "undefined" || e instanceof WebAssembly.CompileError;
    report(unsupported ? `this browser cannot run the application (WebAssembly GC is required): ${e.message}` : `failed to load: ${e.message}`);
    return null;
  }
  const win = doc.defaultView;
  host.call((x) => x.set_route(host.writeIO(routeFromLocation(win.location))));
  host.call((x) => x.start());
  win.addEventListener("hashchange", () => host.setRoute(routeFromLocation(win.location)));
  win.vcwl = host;
  return host;
}
