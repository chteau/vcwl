import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo, launch } from "./harness.mjs";
import { serve } from "./serve.mjs";

let dist, app;
before(async () => {
  dist = build(path.join(repo, "tests/browser/fixtures/fonts"));
  app = await open(dist);
});
after(async () => app?.close());

test("the prerendered page shows before (and without) the application", async () => {
  const { server, url } = await serve(dist, "/");
  const browser = await launch();
  try {
    const page = await browser.newPage();
    const errors = [];
    page.on("console", (m) => errors.push(m.text()));
    await page.setRequestInterception(true);
    page.on("request", (r) => (r.url().endsWith(".js") ? r.abort() : r.continue()));
    await page.goto(url);
    await page.evaluate(() => document.fonts.ready);
    assert.equal(await page.$eval("#vcwl-pre h1", (e) => e.textContent), "Wide glyphs");
    assert.equal(await page.$eval("#vcwl-pre h1", (e) => getComputedStyle(e).fontSize), "40px");
    assert.ok(await page.evaluate(() => document.fonts.check("700 40px Tuffy")), "@font-face loaded");
    assert.deepEqual(errors.filter((e) => !e.includes("ERR_FAILED")), [], "no CSP violations");
  } finally {
    await browser.close();
    server.close();
  }
});

test("the first frame uses the web font, then replaces the prerendered page", async () => {
  const { page } = app;
  assert.equal(await page.$("#vcwl-pre"), null);
  assert.equal(await page.$eval("canvas", (c) => c.style.visibility), "");
  const t = await tree(page);
  const text = t.one("h1").children[0];
  const [web, fallback] = await page.evaluate((s) => {
    const c = document.createElement("canvas").getContext("2d");
    const w = (f) => ((c.font = f), c.measureText(s).width);
    return [w("700 40px Tuffy, serif"), w("700 40px serif")];
  }, text.text);
  assert.notEqual(Math.round(web), Math.round(fallback), "the fixture font differs from the fallback");
  assert.equal(Math.round(text.rect[2]), Math.round(web));
  assert.equal(await page.evaluate(() => window.vcwl.stats.frames > 0 && performance.getEntriesByName("vcwl:first-frame").length), 1);
});

test("the application works after the takeover", async () => {
  const { page } = app;
  await click(page, "button");
  await waitFor(page, (t) => t.text("button") === "Clicked 1");
});

test("no console errors", () => {
  assert.deepEqual(app.errors, []);
});
