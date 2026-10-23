import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/limits")));
});
after(async () => app?.close());

const texts = (t) => t.all("p").map((p) => t.text(p));

test("a component with 80 state values sums and updates them", async () => {
  const { page } = app;
  assert.ok(texts(await tree(page)).includes("sum 3160"));
  await click(page, "button:text('Bump')");
  await waitFor(page, (t) => texts(t).includes("sum 3162"));
});

test("nested state works and handlers read derived values after assigning", async () => {
  const { page } = app;
  let t = await tree(page);
  assert.ok(texts(t).includes("doubled 10"));
  assert.ok(texts(t).includes("seen 0"));
  await click(page, "button:text('Read')");
  await waitFor(page, (t) => texts(t).includes("seen 40"));
  t = await tree(page);
  assert.ok(texts(t).includes("doubled 40"));
});

test("style if wins over a plain style setting the same property", async () => {
  const { page } = app;
  const color = () => page.$eval("h1", (e) => getComputedStyle(e).color);
  assert.equal(await color(), "rgb(0, 0, 255)");
  await click(page, "button:text('Toggle')");
  await page.waitForFunction(() => getComputedStyle(document.querySelector("h1")).color === "rgb(255, 0, 0)");
  assert.equal(await color(), "rgb(255, 0, 0)");
});
