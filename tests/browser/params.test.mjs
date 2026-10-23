import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, ready, tree, click, waitFor, repo } from "./harness.mjs";

let dist, app;
before(async () => {
  dist = build(path.join(repo, "tests/browser/fixtures/params"));
  app = await open(dist);
});
after(async () => app?.close());

const h1 = async (page) => (await tree(page)).text("h1");
const visit = async (page, url) => {
  const res = await page.goto(url);
  await ready(page);
  return res;
};

test("a link to a param route shows its parameter", async () => {
  const { page, url } = app;
  assert.equal(await h1(page), "Home page");
  await click(page, 'a:text("Post 42")');
  await waitFor(page, (t) => t.text("h1") === "Post 42");
  assert.equal(page.url(), url + "post/42/");
  await page.goBack();
  await waitFor(page, (t) => t.text("h1") === "Home page");
});

test("a prerendered param URL loads directly", async () => {
  const { page, url } = app;
  const res = await visit(page, url + "post/hello/");
  assert.equal(res.status(), 200);
  assert.equal(await h1(page), "Post hello");
});

test("a param URL that was not prerendered boots from 404.html", async () => {
  const { page, url } = app;
  await visit(page, url + "post/7");
  await waitFor(page, (t) => t.text("h1") === "Post 7");
});

test("unknown paths show the fallback", async () => {
  const { page, url } = app;
  const res = await visit(page, url + "nope");
  assert.equal(res.status(), 404);
  assert.equal(await h1(page), "Not found");
});

test("no console errors", () => assert.deepEqual(app.errors.filter((e) => !e.includes("404")), []));
