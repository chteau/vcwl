import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { build, open, ready, tree, click, waitFor, repo } from "./harness.mjs";

let dist, app;
before(async () => {
  dist = build(path.join(repo, "tests/browser/fixtures/meta"));
  app = await open(dist);
});
after(async () => app?.close());

const description = (page) => page.$eval('meta[name="description"]', (m) => m.content);

test("each route sets the page title and description", async () => {
  const { page, url } = app;
  assert.equal(await page.title(), "Home 0");
  assert.equal(await description(page), "The home page");
  await click(page, 'a:text("About")');
  await waitFor(page, (t) => t.text("h1") === "About page");
  assert.equal(page.url(), url + "about/");
  assert.equal(await page.title(), "About us");
  assert.equal(await description(page), "Who we are");
  await page.goBack();
  await waitFor(page, (t) => t.text("h1") === "Home page");
  assert.equal(await page.title(), "Home 0");
  assert.equal(await description(page), "The home page");
});

test("the title follows the values it shows", async () => {
  const { page } = app;
  await click(page, 'button:text("Visit")');
  await page.waitForFunction(() => document.title === "Home 1", { timeout: 5000 });
});

test("a page loaded at a route has that route's title", async () => {
  const { page, url } = app;
  await page.goto(url + "about/");
  await ready(page);
  assert.equal(await page.title(), "About us");
  await page.goto(url);
  await ready(page);
});

test("prerendered pages carry their route's metadata", () => {
  const about = fs.readFileSync(path.join(dist, "about/index.html"), "utf8");
  assert.ok(about.includes("<title>About us</title>"), about);
  assert.ok(about.includes('<meta name="description" content="Who we are">'), about);
  const home = fs.readFileSync(path.join(dist, "index.html"), "utf8");
  assert.ok(!home.includes("About us"));
});

test("texts longer than one command buffer render fully", async () => {
  const { page } = app;
  await click(page, 'button:text("Big")');
  await waitFor(page, (t) => t.text("p").length > 0);
  const text = (await tree(page)).text("p");
  assert.equal(text.length, 200003);
  assert.equal(text, "0123456789".repeat(20000) + "END");
});

test("no console errors", () => assert.deepEqual(app.errors.filter((e) => !e.includes("404")), []));
