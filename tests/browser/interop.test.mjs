import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/interop")), { hash: "?code=abc#state=s1" });
});
after(async () => app?.close());

const texts = async () => (await tree(app.page)).all("p").map((p) => p.text ?? "");
const has = (t, s) => t.nodes.some((n) => n.text === s);

test("@web reads the URL's query and fragment parameters", async () => {
  const t = await tree(app.page);
  assert.ok(has(t, "code=abc state=s1"), JSON.stringify(await texts()));
  assert.ok(has(t, "encoded=a%20b%26c params=2"));
});

test("extern fns call the app's JavaScript module with JSON values", async () => {
  assert.ok(has(await tree(app.page), "sum=42 point=p7"));
});

test("async JavaScript functions are awaited with fetch; rejections are errors", async () => {
  await click(app.page, "button:text('Run')");
  // SHA-256("abc") in base64url, the PKCE way.
  await waitFor(app.page, (t) => t.nodes.some((n) => n.text === "hashed=ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0"));
  await waitFor(app.page, (t) => t.nodes.some((n) => n.text === "upper=VIBES failed=network error: nope"));
  assert.ok(has(await tree(app.page), "stored=v1"));
});

test("a popup comes back with the OAuth redirect URL", async () => {
  await click(app.page, "button:text('Sign in')");
  await waitFor(app.page, (t) => t.nodes.some((n) => /^back=http:\/\/127\.0\.0\.1:\d+\/callback\.html\?code=xyz$/.test(n.text ?? "")), 10000);
});

test("no console errors", () => assert.deepEqual(app.errors, []));
