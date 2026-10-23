// Implementations of `@web` (runtime/web.vcwl): each export is called with the VCWL
// arguments (decoded from JSON) and returns a JSON-compatible value, or a Promise of one for
// the functions used with `fetch`. `this` is { caps } (the app's capabilities).

const params = () => {
  const out = [];
  for (const [name, value] of new URLSearchParams(location.search)) out.push({ name, value });
  for (const [name, value] of new URLSearchParams(location.hash.slice(1))) out.push({ name, value });
  return out;
};

// The app's `storage` capability: `local` allows both stores (and cookies), `session` only
// sessionStorage.
function storage(kind, caps) {
  const cap = caps?.storage ?? "none";
  if (cap === "none" || (kind === "local" && cap !== "local")) throw new Error(`the app's \`storage\` capability is \`${cap}\` (declare \`storage: ${kind}\`)`);
  return kind === "local" ? localStorage : sessionStorage;
}

const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const utf8 = new TextEncoder();
const sha256 = async (text) => new Uint8Array(await crypto.subtle.digest("SHA-256", utf8.encode(text)));
const loc = (locale) => locale || undefined;

// Open WebSockets by id, with the messages sent before they opened.
const sockets = new Map();
let nextSocket = 1;

export default {
  // URL, history, navigation
  location: () => location.href,
  origin: () => location.origin,
  pathname: () => location.pathname,
  url_hash: () => location.hash.slice(1),
  url_param: (name) => params().find((p) => p.name === name)?.value ?? null,
  url_params: params,
  replace_url: (url) => history.replaceState(history.state, "", url),
  push_url: (url) => history.pushState(null, "", url),
  history_back: () => history.back(),
  redirect: (url) => location.assign(url),
  reload: () => location.reload(),
  encode_uri: (s) => encodeURIComponent(s),
  decode_uri: (s) => {
    try {
      return decodeURIComponent(s);
    } catch {
      return null;
    }
  },
  with_params: (url, ps) => {
    const u = new URL(url, location.href);
    for (const p of ps) u.searchParams.append(p.name, p.value);
    return u.href;
  },

  // windows and messages
  open_window: (url, name, features) => !!window.open(url, name, features),
  popup_redirect: (url, features) =>
    new Promise((resolve, reject) => {
      const w = window.open(url, "vcwl-popup", features || "popup,width=520,height=640");
      if (!w) return reject(new Error("the popup was blocked"));
      const timer = setInterval(() => {
        if (w.closed) {
          clearInterval(timer);
          return reject(new Error("the popup was closed"));
        }
        let href;
        try {
          href = w.location.href; // throws while on another origin
        } catch {
          return;
        }
        if (href && href !== "about:blank" && new URL(href).origin === location.origin) {
          clearInterval(timer);
          w.close();
          resolve(href);
        }
      }, 200);
    }),
  next_message: (origin) =>
    new Promise((resolve) => {
      const on = (e) => {
        if (origin !== "*" && e.origin !== origin) return;
        removeEventListener("message", on);
        resolve(typeof e.data === "string" ? e.data : JSON.stringify(e.data));
      };
      addEventListener("message", on);
    }),
  post_to_opener: (message, target) => {
    if (!window.opener) return false;
    window.opener.postMessage(message, target);
    return true;
  },
  close_window: () => window.close(),
  has_opener: () => !!window.opener,

  // WebSockets
  websocket: (url, event) => {
    const id = nextSocket++;
    const ws = new WebSocket(new URL(url, location.href).href.replace(/^http/, "ws"));
    const entry = { ws, pending: [] };
    sockets.set(id, entry);
    ws.addEventListener("open", () => {
      for (const text of entry.pending) ws.send(text);
      entry.pending = [];
    });
    ws.addEventListener("message", (e) => {
      if (typeof e.data === "string") window.vcwl?.dispatch(event, e.data);
    });
    ws.addEventListener("close", () => sockets.delete(id));
    return id;
  },
  ws_send: (id, text) => {
    const s = sockets.get(id);
    if (!s) return;
    if (s.ws.readyState === WebSocket.CONNECTING) s.pending.push(text);
    else if (s.ws.readyState === WebSocket.OPEN) s.ws.send(text);
  },
  ws_close: (id) => {
    sockets.get(id)?.ws.close();
    sockets.delete(id);
  },

  // storage
  local_get(key) {
    return storage("local", this.caps).getItem(key);
  },
  local_set(key, value) {
    storage("local", this.caps).setItem(key, value);
  },
  local_remove(key) {
    storage("local", this.caps).removeItem(key);
  },
  session_get(key) {
    return storage("session", this.caps).getItem(key);
  },
  session_set(key, value) {
    storage("session", this.caps).setItem(key, value);
  },
  session_remove(key) {
    storage("session", this.caps).removeItem(key);
  },
  cookie_get(name) {
    storage("local", this.caps);
    const hit = document.cookie.split("; ").find((c) => c.slice(0, c.indexOf("=")) === encodeURIComponent(name));
    return hit === undefined ? null : decodeURIComponent(hit.slice(hit.indexOf("=") + 1));
  },
  cookie_set(name, value, maxAge) {
    storage("local", this.caps);
    document.cookie = `${encodeURIComponent(name)}=${encodeURIComponent(value)}; max-age=${maxAge}; path=/; samesite=lax${location.protocol === "https:" ? "; secure" : ""}`;
  },

  // time, dates, numbers
  epoch_ms: () => Date.now(),
  now_iso: () => new Date().toISOString(),
  time_zone: () => Intl.DateTimeFormat().resolvedOptions().timeZone,
  format_date: (ms, locale, dateStyle, timeStyle) => new Intl.DateTimeFormat(loc(locale), { dateStyle: dateStyle || undefined, timeStyle: timeStyle || undefined }).format(new Date(ms)),
  format_number: (n, locale, digits) => new Intl.NumberFormat(loc(locale), { maximumFractionDigits: digits }).format(n),
  format_currency: (amount, currency, locale) => new Intl.NumberFormat(loc(locale), { style: "currency", currency }).format(amount),
  relative_time: (amount, unit, locale) => new Intl.RelativeTimeFormat(loc(locale), { numeric: "auto" }).format(amount, unit),
  format_list: (items, locale) => new Intl.ListFormat(loc(locale), { type: "conjunction" }).format(items),
  sleep: (ms) => new Promise((r) => setTimeout(() => r(null), ms)),

  // randomness, hashing, encoding
  random: () => crypto.getRandomValues(new Uint32Array(1))[0] / 2 ** 32,
  random_int: (low, high) => low + Math.floor((crypto.getRandomValues(new Uint32Array(1))[0] / 2 ** 32) * (high - low + 1)),
  uuid: () => crypto.randomUUID(),
  random_token: (bytes) => b64url(crypto.getRandomValues(new Uint8Array(bytes))),
  sha256_base64url: async (text) => b64url(await sha256(text)),
  sha256_hex: async (text) => [...(await sha256(text))].map((b) => b.toString(16).padStart(2, "0")).join(""),
  base64_encode: (text) => btoa(String.fromCharCode(...utf8.encode(text))),
  base64_decode: (text) => {
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(Uint8Array.from(atob(text), (c) => c.charCodeAt(0)));
    } catch {
      return null;
    }
  },
  jwt_payload: (token) => {
    try {
      const part = token.split(".")[1].replace(/-/g, "+").replace(/_/g, "/");
      return new TextDecoder().decode(Uint8Array.from(atob(part), (c) => c.charCodeAt(0)));
    } catch {
      return null;
    }
  },

  // the page and the device
  title: () => document.title,
  set_title: (t) => void (document.title = t),
  user_agent: () => navigator.userAgent,
  language: () => navigator.language,
  languages: () => [...navigator.languages],
  online: () => navigator.onLine,
  media_matches: (q) => matchMedia(q).matches,
  viewport_width: () => innerWidth,
  viewport_height: () => innerHeight,
  device_pixel_ratio: () => devicePixelRatio,
  vibrate: (ms) => !!navigator.vibrate?.(ms),
  speak: (text, locale) => {
    const u = new SpeechSynthesisUtterance(text);
    if (locale) u.lang = locale;
    speechSynthesis.speak(u);
  },
  download: (filename, text, mime) => {
    const a = document.createElement("a");
    a.href = URL.createObjectURL(new Blob([text], { type: mime || "text/plain" }));
    a.download = filename;
    a.click();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  },
  console_log: (m) => console.log(m),
  console_warn: (m) => console.warn(m),
  console_error: (m) => console.error(m),
  alert: (m) => alert(m),
  confirm: (m) => confirm(m),
  prompt: (m, d) => prompt(m, d),
  read_clipboard: () => navigator.clipboard.readText(),
  fetch_text: async (url) => {
    const r = await fetch(url);
    if (!r.ok) throw new Error(`HTTP ${r.status}`);
    return r.text();
  },
  write_clipboard: async (text) => (await navigator.clipboard.writeText(text), null),
  share: async (title, text, url) => {
    if (!navigator.share) throw new Error("sharing is not supported by this browser");
    await navigator.share({ title, text, url: url || undefined });
    return null;
  },
  notify: async (title, body) => {
    if (typeof Notification === "undefined") return false;
    if ((await Notification.requestPermission()) !== "granted") return false;
    new Notification(title, { body });
    return true;
  },
  geolocation: () =>
    new Promise((resolve, reject) =>
      navigator.geolocation.getCurrentPosition(
        (p) => resolve({ latitude: p.coords.latitude, longitude: p.coords.longitude, accuracy: p.coords.accuracy }),
        (e) => reject(new Error(e.message)),
      ),
    ),
  fullscreen: async () => (await document.documentElement.requestFullscreen(), null),
  exit_fullscreen: async () => (await document.exitFullscreen(), null),
  battery_level: async () => {
    if (!navigator.getBattery) throw new Error("the Battery Status API is not supported by this browser");
    return (await navigator.getBattery()).level;
  },
};
