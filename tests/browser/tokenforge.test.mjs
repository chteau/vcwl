// The tokenforge site remake (examples/tokenforge): data loading, routing, anchors, bound styles,
// clipboard and the accessibility mirror on a larger app.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "examples/tokenforge")), { viewport: { width: 1280, height: 900 } });
  await waitFor(app.page, (t) => t.count("h1") === 1);
});
after(async () => app?.close());

const labelled = (t, prefix) => t.nodes.filter((n) => n.attrs?.["aria-label"]?.startsWith(prefix));

test("renders the fetched benchmark data", async () => {
  const t = await tree(app.page);
  assert.equal(t.text("h1"), "TokenForge");
  assert.match(t.text("[id=numbers]"), /Measured on 37 real tasks/);
  assert.match(t.text("[id=numbers]"), /51%fewer tokens/);
  assert.equal(await app.page.title(), "TokenForge");
  const meta = await app.page.$eval("meta[name=description]", (m) => m.content);
  assert.match(meta, /trims what every request re-sends/);
});

test("bound styles size the bars from the data", async () => {
  const t = await tree(app.page);
  const bars = labelled(t, "request ");
  assert.equal(bars.length, 17);
  const h = (n) => n.rect[3];
  // Request 17 of the clean session is the largest point; request 1 is about 18k of ~100k.
  assert.ok(h(bars[0]) > 0 && h(bars[0]) < h(bars[16]) / 3, `${h(bars[0])} vs ${h(bars[16])}`);
});

test("in-page anchors scroll to their section", async () => {
  await click(app.page, "header a[href='#install']");
  await waitFor(app.page, (t) => Math.abs(t.one("[id=install]").rect[1]) < 2);
});

test("routes, current link and scroll reset", async () => {
  await click(app.page, "nav a[href='#/benchmark']");
  await waitFor(app.page, (t) => t.text("h1") === "Clean Claude Code vs TokenForge");
  const t = await tree(app.page);
  assert.equal(t.one("nav a[href='#/benchmark']").attrs["aria-current"], "true");
  assert.equal(t.one("nav a[href='#/']").attrs["aria-current"], "false");
  assert.ok(t.one("h1").rect[1] > 0 && t.one("h1").rect[1] < 200, "page starts at the top");
  assert.match(t.text("main"), /Vite front page/);
  const ax = await app.page.accessibility.snapshot();
  const find = (n, pred) => (pred(n) ? n : (n.children ?? []).map((c) => find(c, pred)).find(Boolean));
  assert.ok(find(ax, (n) => n.role === "navigation" && n.name === "Main"));
  assert.ok(find(ax, (n) => n.role === "heading" && n.level === 1 && n.name === "Clean Claude Code vs TokenForge"));
});

test("copy buttons write to the clipboard", async () => {
  await app.page.browserContext().overridePermissions(app.url, ["clipboard-read", "clipboard-write", "clipboard-sanitized-write"]);
  await click(app.page, "nav a[href='#/docs/install']");
  await waitFor(app.page, (t) => t.text("h1") === "Install & first start");
  await click(app.page, "button[aria-label='Copy the commands']");
  await waitFor(app.page, (t) => t.text("button[aria-label='Copy the commands']") === "Copied");
  const text = await app.page.evaluate(() => navigator.clipboard.readText());
  assert.equal(text, "/plugin marketplace add chteau/tokenforge\n/plugin install tokenforge@tokenforge");
});

test("no console errors", () => {
  assert.deepEqual(app.errors, []);
});
