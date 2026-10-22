# vcwl-oauth

"Sign in with ___" for VCWL apps: Google, Discord, GitHub, Roblox, Microsoft, Apple, Twitch,
Spotify, GitLab, Facebook, Reddit, LinkedIn, Slack, X, and anything else that speaks OAuth 2.0.
Sign-in happens in a popup; your app gets a token and, if you want, a profile.

```toml
# vcwl.toml
[dependencies]
oauth = "^0.1"

[trust]
oauth = "0.1.0"   # it ships JavaScript, so the security review asks you to trust it
```

```vcwl
app "Vibes" {
    capabilities {
        network: allow("self", "discord.com")   // the provider's token and API hosts
        storage: local                           // keeps users signed in across reloads
    }
}

import "@oauth"

component App {
    state user: OAuthUser? = None

    on mount {
        oauth_provider("discord", "your-client-id", "")   // "" = default sign-in scopes
    }

    button("Sign in with Discord") {
        on click {
            fetch oauth_sign_in("discord") {
                Ok(_) => {
                    fetch oauth_user("discord") {
                        Ok(u) => {
                            user = Some(u)
                        }
                        Err(e) => {
                            print(e.message())
                        }
                    }
                }
                Err(e) => {
                    print(e.message())
                }
            }
        }
    }
}
```

Register your site's root (`https://your.site/`, or `http://localhost:<port>/` while developing)
as the redirect URI with each provider, or pick another with `oauth_redirect_uri`. The popup
lands there, the app's copy of this package passes the result back to the page that opened it,
and the popup closes.

| function | does |
| --- | --- |
| `oauth_provider(name, client_id, scopes) -> Bool` | configure a built-in provider; restores a stored token |
| `oauth_custom(name, authorize_url, token_url, userinfo_url, client_id, scopes, flow)` | any other provider |
| `oauth_redirect_uri(uri)` | the registered redirect URI (default: the site root) |
| `oauth_token_proxy(url)` | your endpoint for providers that need a client secret |
| `oauth_sign_in(provider) -> OAuthToken` (async) | the popup; call it from a click |
| `oauth_token(provider) -> String?` | the access token, while it has not expired |
| `oauth_user(provider) -> OAuthUser` (async) | `{provider, id, name, email?, avatar?}` |
| `oauth_sign_out(provider)` | forget the token (not revoked at the provider) |

## Providers

How each one hands out tokens, according to its documentation at the time of writing:

| provider | flow | hosts for `network` |
| --- | --- | --- |
| `google` | implicit | `openidconnect.googleapis.com` |
| `discord` | implicit | `discord.com` |
| `twitch` | implicit | `api.twitch.tv` |
| `facebook` | implicit | `graph.facebook.com` |
| `reddit` | implicit | `oauth.reddit.com` |
| `microsoft` | pkce (register the redirect as a "single-page application") | `login.microsoftonline.com`, `graph.microsoft.com` |
| `spotify` | pkce | `accounts.spotify.com`, `api.spotify.com` |
| `gitlab` | pkce | `gitlab.com` |
| `github` | proxy | `api.github.com` |
| `roblox` | proxy | `apis.roblox.com` |
| `apple` | proxy (no name or email: those need a form POST to a server) | — (the profile comes from the ID token) |
| `linkedin` | proxy | `api.linkedin.com` |
| `slack` | proxy | `slack.com` |
| `x` (or `twitter`) | proxy | `api.x.com` |

- **implicit**: the token comes back in the redirect. No secret, no exchange, no refresh token.
- **pkce**: the browser exchanges the code with a PKCE verifier. No secret.
- **proxy**: the provider wants a client secret (or does not let browsers call its token
  endpoint), so the code goes to your server, which exchanges it and returns the provider's
  JSON. Some providers' user APIs refuse browsers too; then `oauth_user` fails and your server
  has to fetch the profile.

A token proxy, e.g. as a Netlify or Cloudflare function (keep the secrets out of the app):

```js
const TOKEN_URLS = {
  github: "https://github.com/login/oauth/access_token",
  roblox: "https://apis.roblox.com/oauth/v1/token",
  // ...
};

export default async (req) => {
  const { provider, code, code_verifier, redirect_uri } = await req.json();
  const id = process.env[`${provider.toUpperCase()}_CLIENT_ID`];
  const secret = process.env[`${provider.toUpperCase()}_CLIENT_SECRET`];
  const body = new URLSearchParams({ grant_type: "authorization_code", code, redirect_uri, client_id: id, client_secret: secret });
  if (code_verifier) body.set("code_verifier", code_verifier);
  const res = await fetch(TOKEN_URLS[provider], {
    method: "POST",
    headers: { Accept: "application/json", "Content-Type": "application/x-www-form-urlencoded" },
    body,
  });
  return new Response(await res.text(), { status: res.status, headers: { "Content-Type": "application/json" } });
};
```

(X wants the client ID and secret as HTTP Basic auth instead; Apple's "secret" is a JWT you sign.)

## Caveats

- Tokens are not refreshed; when one expires, `oauth_token` returns `None` and the user signs in
  again.
- If the user closes the popup, `oauth_sign_in` only fails after five minutes: providers' pages
  can cut the popup off from your page, so closing it cannot be told apart from signing in.
- Tokens stored with `storage: local` are readable by any script on your page.

This package has no unit tests, on purpose. It is vibe coded. It signed in to a fake provider
once (PKCE, implicit and proxy flows) and has never met a real one. Tell us how that goes.
