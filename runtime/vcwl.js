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
// Browser host. The application module builds an element tree through a command buffer;
// the renderer module (render.wasm) styles, lays out and paints it into a display list that
// is replayed here on a <canvas>. The DOM holds only that canvas.

const METHODS = ["GET", "POST", "PUT", "PATCH", "DELETE"];
// Result kinds passed to `fetch_done`.
const FETCH_OK = 0, FETCH_NETWORK = 1, FETCH_TIMEOUT = 2, FETCH_STATUS = 3, FETCH_INVALID = 4;
// Work items produced by the renderer (`render/src/engine.rs`, `out`).
const OUT = { EMIT: 1, TITLE: 2, NAVIGATE: 3, OPEN: 4, WARN: 5, FETCH: 6, LOAD_IMAGE: 7, CURSOR: 8, COPY: 9 };
// Display list opcodes (`render/src/paint.rs`, `op`).
const PAINT = { FILL: 1, STROKE: 2, TEXT: 3, FONT: 4, CLIP: 5, RESTORE: 6, ALPHA: 7, TRANSFORM: 8, IMAGE: 9, SHADOW: 10, GRADIENT: 11, LINE: 12 };
const SAFE_URL = /^(?:(?:https?|mailto|tel):|[^a-z]|[a-z+.-]+(?:[^a-z+.\-:]|$))/i;

/** Rejects `javascript:` and other script-capable URLs. */
export function sanitizeUrl(value) {
  const v = String(value).trim();
  return SAFE_URL.test(v) ? v : "about:invalid#vcwl-blocked";
}

/** Reads words and strings from a little-endian buffer in `memory`. */
function reader(memory, ptr, len) {
  const view = new DataView(memory.buffer, ptr, len);
  const bytes = new Uint8Array(memory.buffer, ptr, len);
  let p = 0;
  return {
    more: () => p < len,
    int() {
      const v = view.getInt32(p, true);
      p += 4;
      return v;
    },
    u32() {
      const v = view.getUint32(p, true);
      p += 4;
      return v;
    },
    f32() {
      const v = view.getFloat32(p, true);
      p += 4;
      return v;
    },
    str() {
      const n = view.getInt32(p, true);
      const s = decoder.decode(bytes.subarray(p + 4, p + 4 + n));
      p += 4 + ((n + 3) & ~3);
      return s;
    },
  };
}

const colors = new Map();
function css(c) {
  let s = colors.get(c);
  if (s === undefined) {
    s = `rgba(${c >>> 24},${(c >>> 16) & 255},${(c >>> 8) & 255},${(c & 255) / 255})`;
    colors.set(c, s);
  }
  return s;
}

function shape(ctx, x, y, w, h, r) {
  ctx.beginPath();
  if (r[0] || r[1] || r[2] || r[3]) ctx.roundRect(x, y, w, h, r);
  else ctx.rect(x, y, w, h);
}

/**
 * Starts an application on `canvas`. `app` and `render` are compiled WebAssembly modules;
 * the app's styles travel in its `vcwl.styles` custom section.
 */
export async function createCanvasHost(canvas, app, render, opts = {}) {
  const doc = canvas.ownerDocument, win = doc.defaultView;
  const host = createHost(opts);
  const report = opts.onError ?? ((msg) => console.error(msg));
  const images = [];
  const fonts = [];
  let r = null, x = null, busy = false, scheduled = false, wakeTimer = 0;
  const queue = [];
  host.dead = false;
  host.stats = { flushes: 0, frames: 0, bytes: 0 };

  const measureCtx = (typeof OffscreenCanvas !== "undefined" ? new OffscreenCanvas(1, 1) : doc.createElement("canvas")).getContext("2d");
  let lastFont = "", lastSpacing = 0;
  const rstr = (ptr, len) => decoder.decode(new Uint8Array(r.memory.buffer, ptr, len));
  const renderImports = {
    host: {
      measure(fp, fl, spacing, tp, tl) {
        const font = rstr(fp, fl);
        if (font !== lastFont) measureCtx.font = lastFont = font;
        if (spacing !== lastSpacing) measureCtx.letterSpacing = (lastSpacing = spacing) + "px";
        return measureCtx.measureText(rstr(tp, tl)).width;
      },
    },
  };
  r = (await WebAssembly.instantiate(render, renderImports)).exports;

  /** Copies a string or bytes into the renderer's input buffer; returns the length. */
  function put(data) {
    const bytes = typeof data === "string" ? encoder.encode(data) : data;
    const ptr = r.alloc(bytes.length);
    new Uint8Array(r.memory.buffer, ptr, bytes.length).set(bytes);
    return bytes.length;
  }
  const out = (len) => reader(r.memory, r.out_ptr(), len);

  function fail(e) {
    if (host.dead) return;
    host.dead = true;
    let msg = describeError(e);
    if (e instanceof WebAssembly.RuntimeError) {
      try {
        const n = r.panic_message();
        if (n) msg = "renderer panic: " + decoder.decode(new Uint8Array(r.memory.buffer, r.out_ptr(), n));
      } catch {}
    }
    report(msg);
  }

  /** Calls into the application module; nested calls are queued. */
  host.call = (fn) => {
    if (host.dead) return;
    if (busy) {
      queue.push(fn);
      return;
    }
    busy = true;
    try {
      fn(x);
      drain();
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
    while (queue.length && !host.dead) host.call(queue.shift());
    schedule();
  };

  /** Runs a renderer entry point, then delivers the events it produced. */
  function input(fn) {
    if (host.dead) return;
    try {
      fn();
    } catch (e) {
      return fail(e);
    }
    host.call(() => {});
  }

  host.imports.flush = (len) => {
    host.stats.flushes++;
    host.stats.bytes += len;
    const n = put(new Uint8Array(host.memory.buffer, 0, len));
    const err = r.apply(n);
    if (err) throw new Error(new TextDecoder().decode(new Uint8Array(r.memory.buffer, r.out_ptr(), err)));
  };

  function drain() {
    for (let n = r.poll(); n; n = r.poll()) {
      const b = out(n);
      while (b.more()) {
        switch (b.int()) {
          case OUT.EMIT: {
            const slot = b.int(), kind = b.int();
            const payload = kind === 1 ? b.str() : kind === 2 ? b.int() : 0;
            const len = kind === 1 ? host.writeIO(payload) : payload;
            x.dispatch(slot, len);
            break;
          }
          case OUT.TITLE:
            doc.title = b.str();
            break;
          case OUT.NAVIGATE: {
            const path = b.str();
            if (opts.navigate) opts.navigate(path);
            else win.location.hash = "#" + path;
            break;
          }
          case OUT.OPEN: {
            const url = sanitizeUrl(b.str()), newTab = b.int();
            if (newTab) win.open(url, "_blank", "noopener");
            else win.location.assign(url);
            break;
          }
          case OUT.WARN:
            console.warn("[vcwl] " + b.str());
            break;
          case OUT.FETCH: {
            const slot = b.int(), method = METHODS[b.int()], hasBody = b.int();
            const url = b.str(), body = b.str();
            startFetch(slot, method, url, hasBody ? body : null);
            break;
          }
          case OUT.LOAD_IMAGE: {
            const id = b.int(), img = new Image();
            img.onload = () => {
              images[id] = img;
              input(() => r.image_ready(id, img.naturalWidth, img.naturalHeight));
            };
            img.src = sanitizeUrl(b.str());
            break;
          }
          case OUT.CURSOR:
            canvas.style.cursor = b.str();
            break;
          case OUT.COPY:
            win.navigator.clipboard?.writeText(b.str()).catch(() => {});
            break;
          default:
            throw new Error("corrupt renderer output");
        }
      }
    }
  }

  // ----- painting -----

  const ctx = canvas.getContext("2d");
  let dpr = 1;
  function resize() {
    dpr = win.devicePixelRatio || 1;
    const w = win.innerWidth, h = win.innerHeight;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    input(() => r.resize(w, h));
  }

  function schedule() {
    if (scheduled || host.dead || !r.dirty()) return;
    scheduled = true;
    win.requestAnimationFrame(frame);
  }

  function frame() {
    scheduled = false;
    if (host.dead) return;
    host.paintNow();
    clearTimeout(wakeTimer);
    // The caret blinks without input; come back when it toggles.
    const wake = r.next_wake(win.performance.now());
    if (wake >= 0) {
      wakeTimer = setTimeout(() => {
        if (!scheduled) {
          scheduled = true;
          win.requestAnimationFrame(frame);
        }
      }, wake);
    }
  }

  /** Paints now if anything changed (also used by tests and benchmarks). */
  host.paintNow = () => {
    let n;
    try {
      n = r.frame(win.performance.now());
    } catch (e) {
      return fail(e);
    }
    if (n) {
      last = new Uint8Array(r.memory.buffer, r.out_ptr(), n).slice();
      replay();
    }
  };

  let last = new Uint8Array(0);
  function replay() {
    const mem = { buffer: last.buffer };
    paint(reader(mem, 0, last.length));
  }

  function paint(b) {
    host.stats.frames++;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.globalAlpha = 1;
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.textBaseline = "middle";
    ctx.lineCap = "butt";
    const r4 = () => [b.f32(), b.f32(), b.f32(), b.f32()];
    while (b.more()) {
      switch (b.u32()) {
        case PAINT.FILL: {
          const [x0, y0, w, h] = r4();
          ctx.fillStyle = css(b.u32());
          shape(ctx, x0, y0, w, h, r4());
          ctx.fill();
          break;
        }
        case PAINT.STROKE: {
          const [x0, y0, w, h] = r4();
          ctx.strokeStyle = css(b.u32());
          const lw = (ctx.lineWidth = b.f32());
          const radii = r4(), dash = b.u32();
          ctx.setLineDash(dash === 1 ? [lw * 3, lw * 2] : dash === 2 ? [lw, lw] : []);
          shape(ctx, x0, y0, w, h, radii);
          ctx.stroke();
          ctx.setLineDash([]);
          break;
        }
        case PAINT.TEXT: {
          const tx = b.f32(), ty = b.f32(), color = b.u32(), f = fonts[b.u32()];
          if (ctx.font !== f[0]) ctx.font = f[0];
          ctx.letterSpacing = f[1];
          ctx.fillStyle = css(color);
          ctx.fillText(b.str(), tx, ty);
          break;
        }
        case PAINT.FONT: {
          const id = b.u32(), spacing = b.f32();
          fonts[id] = [b.str(), spacing + "px"];
          break;
        }
        case PAINT.CLIP: {
          const [x0, y0, w, h] = r4();
          ctx.save();
          shape(ctx, x0, y0, w, h, r4());
          ctx.clip();
          break;
        }
        case PAINT.RESTORE:
          ctx.restore();
          break;
        case PAINT.ALPHA:
          ctx.save();
          ctx.globalAlpha *= b.f32();
          break;
        case PAINT.TRANSFORM:
          ctx.save();
          ctx.transform(b.f32(), b.f32(), b.f32(), b.f32(), b.f32(), b.f32());
          break;
        case PAINT.IMAGE: {
          const img = images[b.u32()];
          const [dx, dy, dw, dh] = r4(), [sx, sy, sw, sh] = r4();
          if (img) ctx.drawImage(img, sx, sy, sw, sh, dx, dy, dw, dh);
          break;
        }
        case PAINT.SHADOW: {
          const [x0, y0, w, h] = r4(), radii = r4();
          const ox = b.f32(), oy = b.f32(), blur = b.f32(), color = css(b.u32());
          // Draw the shape far off-screen so only its shadow lands in place.
          const far = 100000;
          ctx.save();
          ctx.shadowColor = color;
          ctx.shadowBlur = blur * dpr;
          ctx.shadowOffsetX = (ox + far) * dpr;
          ctx.shadowOffsetY = oy * dpr;
          ctx.fillStyle = "#000";
          shape(ctx, x0 - far, y0, w, h, radii);
          ctx.fill();
          ctx.restore();
          break;
        }
        case PAINT.GRADIENT: {
          const [x0, y0, w, h] = r4(), radii = r4();
          const g = ctx.createLinearGradient(b.f32(), b.f32(), b.f32(), b.f32());
          for (let i = b.u32(); i > 0; i--) g.addColorStop(Math.min(1, Math.max(0, b.f32())), css(b.u32()));
          ctx.fillStyle = g;
          shape(ctx, x0, y0, w, h, radii);
          ctx.fill();
          break;
        }
        case PAINT.LINE: {
          const [x1, y1, x2, y2] = r4();
          ctx.lineWidth = b.f32();
          ctx.strokeStyle = css(b.u32());
          ctx.beginPath();
          ctx.moveTo(x1, y1);
          ctx.lineTo(x2, y2);
          ctx.stroke();
          break;
        }
        default:
          throw new Error("corrupt display list");
      }
    }
    if (host.error) drawError(ctx, win.innerWidth, win.innerHeight, host.error);
  }

  // ----- network -----

  /** Resolves `raw` against the page and checks it against the app's network capability. */
  function allowedUrl(raw) {
    let url;
    try {
      url = new URL(raw, doc.baseURI);
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

  // ----- input -----

  const pos = (e) => [e.clientX, e.clientY];
  let pointerFocus = false;
  canvas.addEventListener("pointerdown", (e) => {
    pointerFocus = true;
    canvas.focus({ preventScroll: true });
    pointerFocus = false;
    canvas.setPointerCapture?.(e.pointerId);
    e.preventDefault();
    input(() => r.pointer_down(...pos(e), e.pointerType === "touch" ? 1 : 0, win.performance.now()));
  });
  canvas.addEventListener("pointermove", (e) => input(() => r.pointer_move(...pos(e))));
  canvas.addEventListener("pointerup", (e) => input(() => r.pointer_up(...pos(e), win.performance.now())));
  canvas.addEventListener("pointercancel", () => input(() => r.leave()));
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    const k = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? win.innerHeight : 1;
    input(() => r.wheel(...pos(e), e.deltaX * k, e.deltaY * k));
  }, { passive: false });
  canvas.addEventListener("keydown", (e) => {
    if (e.isComposing) return;
    let used = 0;
    input(() => (used = r.key(put(e.key), e.shiftKey, e.ctrlKey || e.metaKey, e.altKey, win.performance.now())));
    if (used) e.preventDefault();
  });
  doc.addEventListener("paste", (e) => {
    if (doc.activeElement !== canvas) return;
    const text = e.clipboardData?.getData("text/plain");
    if (text) {
      e.preventDefault();
      input(() => r.insert_text(put(text)));
    }
  });
  canvas.addEventListener("focus", () => {
    if (!pointerFocus) input(() => r.enter(0));
  });
  canvas.addEventListener("blur", () => input(() => r.leave()));
  win.addEventListener("resize", resize);
  const dark = win.matchMedia("(prefers-color-scheme: dark)"), motion = win.matchMedia("(prefers-reduced-motion: reduce)");
  const prefs = () => input(() => r.preferences(dark.matches ? 1 : 0, motion.matches ? 1 : 0));
  dark.addEventListener?.("change", prefs);
  motion.addEventListener?.("change", prefs);

  // ----- start -----

  const styles = WebAssembly.Module.customSections(app, "vcwl.styles")[0];
  r.init(put(new Uint8Array(styles ?? new ArrayBuffer(0))), win.innerWidth, win.innerHeight);
  prefs();
  const instance = await WebAssembly.instantiate(app, { vcwl: host.imports });
  host.memory = instance.exports.memory;
  x = host.exports = instance.exports;
  resize();

  host.tree = () => {
    host.paintNow();
    const n = r.dump();
    return JSON.parse(decoder.decode(new Uint8Array(r.memory.buffer, r.out_ptr(), n)));
  };
  host.liveNodes = () => r.live_nodes();
  host.liveListeners = () => r.live_listeners();
  host.commands = () => r.counter(0);
  host.moves = () => r.counter(1);
  /** Focuses the canvas without moving focus to the first control. */
  host.focus = () => {
    pointerFocus = true;
    canvas.focus({ preventScroll: true });
    pointerFocus = false;
  };
  host.setRoute = (path) => host.call((x) => x.set_route(host.writeIO(path)));
  host.showError = (msg) => {
    host.error = msg;
    replay();
  };
  return host;
}

/** Draws an error box over the bottom of the canvas (development builds). */
function drawError(ctx, w, h, msg) {
  ctx.save();
  ctx.font = "13px ui-monospace,monospace";
  const lines = msg.split("\n").flatMap((l) => {
    const out = [];
    for (let s = l; ; ) {
      let n = s.length;
      while (n > 1 && ctx.measureText(s.slice(0, n)).width > w - 64) n--;
      out.push(s.slice(0, n));
      s = s.slice(n);
      if (!s) break;
    }
    return out;
  }).slice(0, 40);
  const bh = lines.length * 18 + 24, y = Math.max(8, h - bh - 16);
  ctx.fillStyle = "#2b0b0b";
  ctx.strokeStyle = "#ff6b6b";
  ctx.beginPath();
  ctx.roundRect(16, y, w - 32, bh, 8);
  ctx.fill();
  ctx.stroke();
  ctx.fillStyle = "#ffd7d7";
  ctx.textBaseline = "top";
  lines.forEach((l, i) => ctx.fillText(l, 32, y + 12 + i * 18));
  ctx.restore();
}

/** Shows `msg` on the page's canvas (development builds and load failures). */
export function showError(doc, msg) {
  const win = doc.defaultView;
  if (win.vcwl?.showError) return win.vcwl.showError(msg && "VCWL " + msg);
  let canvas = doc.querySelector("canvas");
  if (!canvas) {
    canvas = doc.createElement("canvas");
    canvas.style.cssText = "position:fixed;inset:0;width:100vw;height:100vh";
    doc.body.append(canvas);
  }
  canvas.width = win.innerWidth;
  canvas.height = win.innerHeight;
  const ctx = canvas.getContext("2d");
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  if (msg) drawError(ctx, canvas.width, canvas.height, "VCWL " + msg);
}

function routeFromLocation(loc) {
  const h = decodeURIComponent(loc.hash.replace(/^#/, ""));
  return h.startsWith("/") ? h : "/";
}

async function compile(url) {
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`cannot load ${url.pathname}: HTTP ${resp.status}`);
  if (WebAssembly.compileStreaming && resp.headers.get("content-type")?.startsWith("application/wasm")) return WebAssembly.compileStreaming(resp);
  return WebAssembly.compile(await resp.arrayBuffer());
}

/** Loads `app.wasm` and `render.wasm` next to this script and starts the application. */
export async function boot(doc = document) {
  const win = doc.defaultView;
  let cfg = {};
  const report = (msg) => {
    console.error("[vcwl] " + msg);
    if (cfg.dev) showError(doc, msg);
  };
  const canvas = doc.createElement("canvas");
  canvas.tabIndex = 0;
  canvas.style.cssText = "position:fixed;inset:0;width:100vw;height:100vh;display:block;outline:none;touch-action:none";
  doc.body.append(canvas);
  let host;
  try {
    const [app, render] = await Promise.all([compile(new URL("app.wasm", import.meta.url)), compile(new URL("render.wasm", import.meta.url))]);
    const config = WebAssembly.Module.customSections(app, "vcwl.config")[0];
    if (config) cfg = JSON.parse(decoder.decode(config));
    host = await createCanvasHost(canvas, app, render, { onError: report, caps: cfg.caps });
  } catch (e) {
    const unsupported = typeof WebAssembly === "undefined" || e instanceof WebAssembly.CompileError;
    console.error(e);
    showError(doc, unsupported ? `this browser cannot run the application (WebAssembly GC is required): ${e.message}` : `failed to load: ${e.message}`);
    return null;
  }
  win.vcwl = host;
  host.call((x) => x.set_route(host.writeIO(routeFromLocation(win.location))));
  host.call((x) => x.start());
  win.addEventListener("hashchange", () => host.setRoute(routeFromLocation(win.location)));
  host.focus();
  return host;
}
