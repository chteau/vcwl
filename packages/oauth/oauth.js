// Implementations of the package's `extern fn`s (src/lib.vcwl): OAuth 2.0 sign-in in a popup.
// The popup comes back to the app's own origin, where this module (loaded there too) hands the
// redirect's parameters to the opener over a BroadcastChannel and closes the popup. Tokens are
// kept where the app's `storage` capability allows (localStorage for `local`, sessionStorage for
// `session`, memory otherwise).

// Endpoints, sign-in scopes and how the code becomes a token, per the providers' documentation.
// "pkce": the browser exchanges the code; "implicit": the token comes back in the redirect;
// "proxy": the code goes to the app's `oauth_token_proxy`, which knows the client secret.
const PROVIDERS = {
  apple: {
    authorize: "https://appleid.apple.com/auth/authorize",
    token: "https://appleid.apple.com/auth/token",
    // Name and email scopes need a form POST to a server, which a popup cannot read.
    scopes: "",
    flow: "proxy",
    pkce: false,
    params: { response_mode: "query" },
  },
  discord: {
    authorize: "https://discord.com/oauth2/authorize",
    userinfo: "https://discord.com/api/users/@me",
    scopes: "identify email",
    flow: "implicit",
    user: (u) => ({
      id: u.id,
      name: u.global_name ?? u.username,
      email: u.email,
      avatar: u.avatar && `https://cdn.discordapp.com/avatars/${u.id}/${u.avatar}.png`,
    }),
  },
  facebook: {
    authorize: "https://www.facebook.com/v19.0/dialog/oauth",
    userinfo: "https://graph.facebook.com/me?fields=id,name,email,picture",
    scopes: "public_profile email",
    flow: "implicit",
    user: (u) => ({ id: u.id, name: u.name, email: u.email, avatar: u.picture?.data?.url }),
  },
  github: {
    authorize: "https://github.com/login/oauth/authorize",
    token: "https://github.com/login/oauth/access_token",
    userinfo: "https://api.github.com/user",
    scopes: "read:user user:email",
    flow: "proxy",
    user: (u) => ({ id: u.id, name: u.name ?? u.login, email: u.email, avatar: u.avatar_url }),
  },
  gitlab: {
    authorize: "https://gitlab.com/oauth/authorize",
    token: "https://gitlab.com/oauth/token",
    userinfo: "https://gitlab.com/api/v4/user",
    scopes: "read_user",
    flow: "pkce",
    user: (u) => ({ id: u.id, name: u.name ?? u.username, email: u.email, avatar: u.avatar_url }),
  },
  google: {
    authorize: "https://accounts.google.com/o/oauth2/v2/auth",
    userinfo: "https://openidconnect.googleapis.com/v1/userinfo",
    scopes: "openid email profile",
    flow: "implicit",
  },
  linkedin: {
    authorize: "https://www.linkedin.com/oauth/v2/authorization",
    token: "https://www.linkedin.com/oauth/v2/accessToken",
    userinfo: "https://api.linkedin.com/v2/userinfo",
    scopes: "openid profile email",
    flow: "proxy",
  },
  microsoft: {
    authorize: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
    token: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
    userinfo: "https://graph.microsoft.com/oidc/userinfo",
    scopes: "openid profile email User.Read",
    flow: "pkce",
  },
  reddit: {
    authorize: "https://www.reddit.com/api/v1/authorize",
    userinfo: "https://oauth.reddit.com/api/v1/me",
    scopes: "identity",
    flow: "implicit",
    user: (u) => ({ id: u.id, name: u.name, avatar: u.icon_img?.split("?")[0] }),
  },
  roblox: {
    authorize: "https://apis.roblox.com/oauth/v1/authorize",
    token: "https://apis.roblox.com/oauth/v1/token",
    userinfo: "https://apis.roblox.com/oauth/v1/userinfo",
    scopes: "openid profile",
    flow: "proxy",
  },
  slack: {
    authorize: "https://slack.com/openid/connect/authorize",
    token: "https://slack.com/api/openid.connect.token",
    userinfo: "https://slack.com/api/openid.connect.userInfo",
    scopes: "openid profile email",
    flow: "proxy",
  },
  spotify: {
    authorize: "https://accounts.spotify.com/authorize",
    token: "https://accounts.spotify.com/api/token",
    userinfo: "https://api.spotify.com/v1/me",
    scopes: "user-read-email user-read-private",
    flow: "pkce",
    user: (u) => ({ id: u.id, name: u.display_name ?? u.id, email: u.email, avatar: u.images?.[0]?.url }),
  },
  twitch: {
    authorize: "https://id.twitch.tv/oauth2/authorize",
    userinfo: "https://api.twitch.tv/helix/users",
    scopes: "user:read:email",
    flow: "implicit",
    headers: (c) => ({ "Client-Id": c.client_id }),
    user: (r) => {
      const u = r.data[0];
      return { id: u.id, name: u.display_name, email: u.email, avatar: u.profile_image_url };
    },
  },
  x: {
    authorize: "https://x.com/i/oauth2/authorize",
    token: "https://api.x.com/2/oauth2/token",
    userinfo: "https://api.x.com/2/users/me?user.fields=profile_image_url",
    scopes: "users.read tweet.read",
    flow: "proxy",
    user: (r) => ({ id: r.data.id, name: r.data.name, avatar: r.data.profile_image_url }),
  },
};
PROVIDERS.twitter = PROVIDERS.x;

// OpenID Connect claims, the default shape of a profile.
const oidcUser = (u) => ({ id: u.sub, name: u.name ?? u.preferred_username ?? u.nickname ?? u.sub, email: u.email, avatar: u.picture });

const CHANNEL = "vcwl-oauth";
const STATE = "vcwl-oauth.";
const configs = {};
const tokens = {};
let redirectUri = null;
let proxy = null;

// In the popup, back from the provider: hand the parameters to the opener and close.
{
  const params = new URLSearchParams(location.search);
  for (const [k, v] of new URLSearchParams(location.hash.slice(1))) params.set(k, v);
  if (params.get("state")?.startsWith(STATE) && typeof BroadcastChannel === "function") {
    const ch = new BroadcastChannel(CHANNEL);
    ch.postMessage(Object.fromEntries(params));
    ch.close();
    history.replaceState(history.state, "", location.pathname);
    window.close();
  }
}

function store(caps) {
  const cap = caps?.storage ?? "none";
  return cap === "local" ? localStorage : cap === "session" ? sessionStorage : null;
}

function keep(caps, provider, t) {
  tokens[provider] = t;
  const st = store(caps);
  if (!st) return;
  if (t) st.setItem(`${CHANNEL}.${provider}`, JSON.stringify(t));
  else st.removeItem(`${CHANNEL}.${provider}`);
}

function restore(caps, provider) {
  try {
    tokens[provider] = JSON.parse(store(caps)?.getItem(`${CHANNEL}.${provider}`) ?? "null");
  } catch {
    tokens[provider] = null;
  }
  return !!live(provider);
}

function live(provider) {
  const t = tokens[provider];
  return t && (!t.expires_at || t.expires_at > Date.now() / 1000) ? t : null;
}

function need(provider) {
  const c = configs[provider];
  if (!c) throw new Error(`oauth: call oauth_provider("${provider}", client_id, scopes) first`);
  return c;
}

const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const random = (n) => b64url(crypto.getRandomValues(new Uint8Array(n)));
const challenge = async (verifier) => b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));

// Waits for the popup's redirect parameters with the given state.
function redirected(state, popup) {
  return new Promise((resolve, reject) => {
    const ch = new BroadcastChannel(CHANNEL);
    // A provider page with Cross-Origin-Opener-Policy cuts the popup off from this window
    // (`popup.closed` then reads true while it is still open), so only a timeout gives up.
    const timer = setTimeout(() => done(new Error("oauth: sign-in timed out")), 5 * 60 * 1000);
    const done = (err, value) => {
      clearTimeout(timer);
      ch.close();
      err ? reject(err) : resolve(value);
    };
    ch.onmessage = (e) => {
      if (e.data?.state !== state) return;
      popup?.close();
      if (e.data.error) done(new Error(`oauth: ${e.data.error_description ?? e.data.error}`.replace(/\+/g, " ")));
      else done(null, e.data);
    };
  });
}

async function post(url, body, headers) {
  const res = await fetch(url, { method: "POST", headers: { Accept: "application/json", ...headers }, body });
  const text = await res.text();
  let data;
  try {
    data = JSON.parse(text);
  } catch {
    data = Object.fromEntries(new URLSearchParams(text));
  }
  if (!res.ok || data.error) throw new Error(`oauth: ${data.error_description ?? data.error ?? `HTTP ${res.status}`}`);
  return data;
}

function token(provider, r, scopes) {
  return {
    provider,
    access_token: r.access_token,
    token_type: r.token_type ?? "Bearer",
    scope: r.scope ?? scopes,
    expires_at: r.expires_in ? Math.floor(Date.now() / 1000) + Number(r.expires_in) : 0,
    refresh_token: r.refresh_token ?? null,
    id_token: r.id_token ?? null,
  };
}

function jwtClaims(jwt) {
  const part = jwt.split(".")[1].replace(/-/g, "+").replace(/_/g, "/");
  return JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(part), (ch) => ch.charCodeAt(0))));
}

function configure(caps, name, c) {
  configs[name] = c;
  return restore(caps, name);
}

export default {
  oauth_provider(name, clientId, scopes) {
    const p = PROVIDERS[name];
    if (!p) throw new Error(`oauth: unknown provider "${name}" (use oauth_custom)`);
    return configure(this.caps, name, { ...p, client_id: clientId, scopes: scopes || p.scopes });
  },
  oauth_custom(name, authorize, tokenUrl, userinfo, clientId, scopes, flow) {
    if (!["pkce", "implicit", "proxy"].includes(flow)) throw new Error(`oauth: flow must be "pkce", "implicit" or "proxy", not "${flow}"`);
    return configure(this.caps, name, { authorize, token: tokenUrl, userinfo, client_id: clientId, scopes, flow });
  },
  oauth_redirect_uri(uri) {
    redirectUri = uri;
  },
  oauth_token_proxy(url) {
    proxy = url;
  },
  async oauth_sign_in(provider) {
    const c = need(provider);
    if (c.flow === "proxy" && !proxy) throw new Error(`oauth: ${provider} needs a client secret, so set oauth_token_proxy(url) first`);
    // Opened first, while the click still counts: popup blockers ignore later awaits.
    const popup = window.open("", CHANNEL, "popup,width=500,height=700");
    if (!popup) throw new Error("oauth: the popup was blocked (call oauth_sign_in from a click)");
    const redirect = redirectUri ?? `${location.origin}/`;
    const state = STATE + random(16);
    const q = new URLSearchParams({ client_id: c.client_id, redirect_uri: redirect, state, ...c.params });
    if (c.scopes) q.set("scope", c.scopes);
    let verifier = null;
    if (c.flow === "implicit") {
      q.set("response_type", "token");
    } else {
      q.set("response_type", "code");
      if (c.pkce !== false) {
        verifier = random(48);
        q.set("code_challenge", await challenge(verifier));
        q.set("code_challenge_method", "S256");
      }
    }
    popup.location.href = `${c.authorize}?${q}`;
    const r = await redirected(state, popup);
    let t;
    if (c.flow === "implicit") {
      t = r;
    } else if (c.flow === "pkce") {
      const body = new URLSearchParams({ grant_type: "authorization_code", code: r.code, redirect_uri: redirect, client_id: c.client_id, code_verifier: verifier });
      t = await post(c.token, body, { "Content-Type": "application/x-www-form-urlencoded" });
    } else {
      const body = JSON.stringify({ provider, code: r.code, code_verifier: verifier, redirect_uri: redirect });
      t = await post(proxy, body, { "Content-Type": "application/json" });
    }
    if (!t.access_token) throw new Error(`oauth: ${provider} returned no access token`);
    const out = token(provider, t, c.scopes);
    keep(this.caps, provider, out);
    return out;
  },
  oauth_token(provider) {
    return live(provider)?.access_token ?? null;
  },
  async oauth_user(provider) {
    const c = need(provider);
    const t = live(provider);
    if (!t) throw new Error(`oauth: not signed in with ${provider}`);
    let u;
    if (c.userinfo) {
      const res = await fetch(c.userinfo, { headers: { Authorization: `Bearer ${t.access_token}`, Accept: "application/json", ...c.headers?.(c) } });
      if (!res.ok) throw new Error(`oauth: ${provider} user info: HTTP ${res.status}`);
      u = (c.user ?? oidcUser)(await res.json());
    } else if (t.id_token) {
      u = oidcUser(jwtClaims(t.id_token));
    } else {
      throw new Error(`oauth: ${provider} has no user info endpoint and returned no ID token`);
    }
    return { provider, id: String(u.id), name: u.name ?? String(u.id), email: u.email ?? null, avatar: u.avatar ?? null };
  },
  oauth_sign_out(provider) {
    keep(this.caps, provider, null);
  },
};
