# vcwl-supabase

Supabase from VCWL: email/password, magic-link and OAuth sign-in, the database through
PostgREST, and public storage URLs. Your backend is someone else's Postgres now.

```toml
# vcwl.toml
[dependencies]
supabase = "^0.1"

[trust]
supabase = "0.1.0"   # it ships JavaScript, so the security review asks you to trust it
```

```vcwl
app "Todos" {
    capabilities {
        network: allow("self", "your-project.supabase.co")
        storage: local        // keeps users signed in across reloads
    }
}

import "@supabase"

struct Todo {
    id: Int,
    title: String,
    done: Bool,
}

component App {
    state todos: [Todo] = []

    on mount {
        supabase_init("https://your-project.supabase.co", "your-anon-key")
        fetch db_select("todos", "select=*&order=id") as [Todo] {
            Ok(rows) => {
                todos = rows
            }
            Err(e) => {
                print(e.message())
            }
        }
    }
    ...
}
```

The functions marked async in `src/lib.vcwl` are called with `fetch`. Database calls return
whatever you ask for with `as`; the rows are checked against that type when they arrive.

| function | does |
| --- | --- |
| `supabase_init(url, anon_key) -> Bool` | connect; restore the session (storage, or the URL after OAuth / magic link) |
| `auth_user()`, `auth_token()` | the signed-in user, the access token |
| `auth_sign_up`, `auth_sign_in` (async) | email and password |
| `auth_magic_link(email, redirect_to)` (async) | email a sign-in link |
| `auth_oauth(provider, redirect_to)` | go to GitHub, Google... and come back signed in |
| `auth_sign_out()` (async) | sign out |
| `db_select(table, query)` (async) | `as [Row]`; `query` is PostgREST: `select=*&done=eq.false&order=id` |
| `db_insert(table, to_json(row))` (async) | returns the inserted rows |
| `db_update(table, "id=eq.3", to_json(patch))` (async) | returns the updated rows |
| `db_delete(table, "id=eq.3")` (async) | returns the deleted rows |
| `db_rpc(name, args_json)` (async) | call a Postgres function |
| `storage_public_url(bucket, path)` | URL of a file in a public bucket |

Access tokens are refreshed when they are about to expire. Protect your tables with row level
security: the anon key is public, and so is everything it can read. For the demo in
`examples/supabase`:

```sql
create table todos (
  id bigint generated always as identity primary key,
  user_id uuid not null default auth.uid() references auth.users,
  title text not null,
  done boolean not null default false
);
alter table todos enable row level security;
create policy "own todos" on todos for all using (auth.uid() = user_id);
```

Not covered: realtime subscriptions, file uploads, MFA. This package has no unit tests, on
purpose. It is vibe coded. It worked against a fake Supabase once and that was enough for us.
