import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/events")));
});
after(async () => app?.close());

const has = (t, s) => t.nodes.some((n) => n.text === s);

test("starts without events", async () => {
  const t = await tree(app.page);
  assert.ok(has(t, "last=none"));
  assert.ok(has(t, "count=0"));
});

test("window.vcwl.dispatch runs the app's `on js` handlers", async () => {
  await app.page.evaluate(() => window.vcwl.dispatch("chat", { from: "a", text: "hi" }));
  await waitFor(app.page, (t) => t.nodes.some((n) => n.text === "last=a: hi"));
  await waitFor(app.page, (t) => t.nodes.some((n) => n.text === "count=1"));
  await app.page.evaluate(() => window.vcwl.dispatch("tick", 5));
  await waitFor(app.page, (t) => t.nodes.some((n) => n.text === "count=6"));
});

test("a bad payload or an unknown event is ignored", async () => {
  await app.page.evaluate(() => {
    window.vcwl.dispatch("chat", { from: 1 });
    window.vcwl.dispatch("tick", "nope");
    window.vcwl.dispatch("nobody", 1);
  });
  await app.page.evaluate(() => window.vcwl.dispatch("chat", { from: "b", text: "ok" }));
  await waitFor(app.page, (t) => t.nodes.some((n) => n.text === "last=b: ok"));
  assert.ok(has(await tree(app.page), "count=7"));
  assert.equal(app.errors.filter(ignored).length, 2, JSON.stringify(app.errors));
});

const ignored = (m) => /^\[vcwl\] event "(chat|tick)" ignored/.test(m);

test("no console errors besides the ignored events' warnings", () =>
  assert.deepEqual(app.errors.filter((m) => !ignored(m)), []));
