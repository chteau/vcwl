// Implementations of the package's `extern fn`s (src/lib.vcwl): Supabase's REST APIs (GoTrue for
// auth, PostgREST for the database) over `fetch`, with the session kept where the app's `storage`
// capability allows (localStorage for `local`, sessionStorage for `session`, memory otherwise).

let config = null;
let session = null;
const KEY = "vcwl-supabase.session";

function store(caps) {
  const cap = caps?.storage ?? "none";
  return cap === "local" ? localStorage : cap === "session" ? sessionStorage : null;
}

function keep(caps, s) {
  session = s;
  const st = store(caps);
  if (!st) return;
  if (s) st.setItem(KEY, JSON.stringify(s));
  else st.removeItem(KEY);
}

function need() {
  if (!config) throw new Error("call supabase_init(url, anon_key) first");
  return config;
}

function withExpiry(s) {
  return { ...s, expires_at: s.expires_at ?? Math.floor(Date.now() / 1000) + Number(s.expires_in ?? 3600) };
}

async function call(caps, path, { method = "GET", body, headers = {}, auth = true } = {}) {
  const c = need();
  if (auth) await fresh(caps);
  const res = await fetch(c.url + path, {
    method,
    headers: {
      apikey: c.key,
      Authorization: `Bearer ${(auth && session?.access_token) || c.key}`,
      ...(body === undefined ? {} : { "Content-Type": "application/json" }),
      ...headers,
    },
    body: body === undefined ? undefined : typeof body === "string" ? body : JSON.stringify(body),
  });
  const text = await res.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!res.ok) {
    const msg = data?.error_description ?? data?.msg ?? data?.message ?? data?.error ?? `HTTP ${res.status}`;
    throw new Error(`supabase: ${msg}`);
  }
  return data;
}

// Refreshes the access token when it expires within a minute.
async function fresh(caps) {
  if (!session?.refresh_token || session.expires_at - 60 > Date.now() / 1000) return;
  try {
    const s = await call(caps, "/auth/v1/token?grant_type=refresh_token", {
      method: "POST",
      body: { refresh_token: session.refresh_token },
      auth: false,
    });
    keep(caps, withExpiry(s));
  } catch {
    keep(caps, null);
  }
}

const user = (u) => (u ? { id: u.id, email: u.email ?? null } : null);

export default {
  supabase_init(url, key) {
    config = { url: url.replace(/\/+$/, ""), key };
    // Back from an OAuth provider or a magic link: the tokens are in the fragment.
    const h = new URLSearchParams(location.hash.slice(1));
    if (h.get("access_token")) {
      keep(this.caps, withExpiry({
        access_token: h.get("access_token"),
        refresh_token: h.get("refresh_token"),
        expires_in: h.get("expires_in"),
        user: null,
      }));
      history.replaceState(history.state, "", location.pathname + location.search);
      call(this.caps, "/auth/v1/user").then((u) => keep(this.caps, { ...session, user: u }), () => {});
    } else {
      try {
        session = JSON.parse(store(this.caps)?.getItem(KEY) ?? "null");
      } catch {
        session = null;
      }
    }
    return !!session;
  },
  auth_user() {
    return user(session?.user);
  },
  auth_token() {
    return session?.access_token ?? null;
  },
  async auth_sign_up(email, password) {
    const r = await call(this.caps, "/auth/v1/signup", { method: "POST", body: { email, password }, auth: false });
    if (r.access_token) keep(this.caps, withExpiry(r));
    return user(r.user ?? r);
  },
  async auth_sign_in(email, password) {
    const r = await call(this.caps, "/auth/v1/token?grant_type=password", { method: "POST", body: { email, password }, auth: false });
    keep(this.caps, withExpiry(r));
    return user(r.user);
  },
  async auth_magic_link(email, redirectTo) {
    const q = redirectTo ? `?redirect_to=${encodeURIComponent(redirectTo)}` : "";
    await call(this.caps, `/auth/v1/otp${q}`, { method: "POST", body: { email, create_user: true }, auth: false });
    return null;
  },
  auth_oauth(provider, redirectTo) {
    const c = need();
    const q = new URLSearchParams({ provider, redirect_to: redirectTo || location.href });
    location.assign(`${c.url}/auth/v1/authorize?${q}`);
  },
  async auth_sign_out() {
    try {
      if (session) await call(this.caps, "/auth/v1/logout", { method: "POST" });
    } finally {
      keep(this.caps, null);
    }
    return null;
  },
  async db_select(table, query) {
    return call(this.caps, `/rest/v1/${encodeURIComponent(table)}${query ? `?${query}` : ""}`);
  },
  async db_insert(table, rows) {
    return call(this.caps, `/rest/v1/${encodeURIComponent(table)}`, { method: "POST", body: rows, headers: { Prefer: "return=representation" } });
  },
  async db_update(table, filter, patch) {
    return call(this.caps, `/rest/v1/${encodeURIComponent(table)}?${filter}`, { method: "PATCH", body: patch, headers: { Prefer: "return=representation" } });
  },
  async db_delete(table, filter) {
    return call(this.caps, `/rest/v1/${encodeURIComponent(table)}?${filter}`, { method: "DELETE", headers: { Prefer: "return=representation" } });
  },
  async db_rpc(fn, args) {
    return call(this.caps, `/rest/v1/rpc/${encodeURIComponent(fn)}`, { method: "POST", body: args || "{}" });
  },
  storage_public_url(bucket, path) {
    return `${need().url}/storage/v1/object/public/${encodeURIComponent(bucket)}/${path.split("/").map(encodeURIComponent).join("/")}`;
  },
};
