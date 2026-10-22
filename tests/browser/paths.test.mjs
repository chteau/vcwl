import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, ready, tree, click, waitFor, repo, launch } from "./harness.mjs";
import { serve } from "./serve.mjs";

let dist, app;
before(async () => {
  dist = build(path.join(repo, "tests/browser/fixtures/paths"));
  app = await open(dist, { base: "/site/" });
});
after(async () => app?.close());

const h1 = async (page) => (await tree(page)).text("h1");
const visit = async (page, url) => {
  const res = await page.goto(url);
  await ready(page);
  return res;
};

test("links lead to real URLs and the back button returns", async () => {
  const { page, url } = app;
  assert.equal(await h1(page), "Home page");
  await click(page, 'a:text("About")');
  await waitFor(page, (t) => t.text("h1") === "About page");
  assert.equal(page.url(), url + "about/");
  await page.goBack();
  await waitFor(page, (t) => t.text("h1") === "Home page");
  assert.equal(page.url(), url);
});

test("every route is a page of its own, and relative URLs are the app's files", async () => {
  const { page, url } = app;
  const res = await visit(page, url + "docs/intro/");
  assert.equal(res.status(), 200);
  await waitFor(page, (t) => t.text("p") === "from data.json");
  assert.equal(await h1(page), "Intro");
  assert.equal(await page.$eval('a[aria-current="true"]', (a) => a.textContent), "Intro");
  await visit(page, url + "about");
  assert.equal(page.url(), url + "about/", "redirected to the directory");
  assert.equal(await h1(page), "About page");
});

test("the mirror's links carry the real URLs", async () => {
  const { page, url } = app;
  await visit(page, url);
  const hrefs = await page.$$eval("a", (as) => as.map((a) => a.href));
  assert.ok(hrefs.includes(url + "docs/intro/"), hrefs.join(" "));
});

test("unknown paths get 404.html with the app's fallback route", async () => {
  const { page, url } = app;
  const res = await visit(page, url + "no/such/page");
  assert.equal(res.status(), 404);
  assert.equal(await h1(page), "Not found");
  await click(page, 'a:text("About")');
  await waitFor(page, (t) => t.text("h1") === "About page");
  assert.equal(page.url(), url + "about/");
});

test("old #/ links move to the path", async () => {
  const { page, url } = app;
  await visit(page, url + "#/about");
  assert.equal(page.url(), url + "about/");
  assert.equal(await h1(page), "About page");
});

test("the prerendered pages link to each other without JavaScript", async () => {
  const { server, url } = await serve(dist, "/site/");
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setJavaScriptEnabled(false);
    await page.goto(url + "docs/intro/");
    assert.equal(await page.$eval("#vcwl-pre h1", (e) => e.textContent), "Intro");
    assert.equal(await page.$eval("#vcwl-pre p", (e) => e.textContent), "from data.json");
    await Promise.all([page.waitForNavigation(), page.click('#vcwl-pre a[href="../../about/"]')]);
    assert.equal(page.url(), url + "about/");
    assert.equal(await page.$eval("#vcwl-pre h1", (e) => e.textContent), "About page");
    const canonical = await page.$eval('link[rel="canonical"]', (l) => l.href);
    assert.equal(canonical, "https://example.com/site/about/");
  } finally {
    await browser.close();
    server.close();
  }
});

test("no console errors", () => assert.deepEqual(app.errors.filter((e) => !e.includes("404")), []));
