import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/slots")));
});
after(async () => app?.close());

const cards = async (page) => {
  const t = await tree(page);
  return t.all("article").map((a) => a.children.filter((c) => c.tag).map((c) => t.text(c)));
};

test("slot content renders where the component puts its slot", async () => {
  assert.deepEqual(await cards(app.page), [
    ["Counter", "Count 0", "Add", "Close"],
    ["Empty", "Close"],
    ["Item 1", "Close"],
    ["Item 2", "Close"],
  ]);
  assert.equal((await tree(app.page)).text("article:nth-child(3)"), "Item 1n is 0Close");
});

test("slot content follows the state of the component that wrote it", async () => {
  const { page } = app;
  await click(page, "button:text('Add')");
  await waitFor(page, (t) => t.text("p") === "Count 1");
  assert.match((await tree(page)).text("article:nth-child(4)"), /n is 1/);
});

test("slot content moves with its keyed component", async () => {
  const { page } = app;
  await click(page, "button:text('Reverse')");
  await waitFor(page, (t) => t.text("article:nth-child(3) h2") === "Item 2");
  assert.equal((await tree(page)).text("article:nth-child(3)"), "Item 2n is 1Close");
});

test("removing the component releases the content's handlers", async () => {
  const { page } = app;
  const before = await page.evaluate(() => window.vcwl.liveListeners());
  await click(page, "article:nth-child(1) button:text('Close')");
  await waitFor(page, (t) => t.count("article") === 3);
  // The card's Close button and the slot's Add button.
  assert.equal(await page.evaluate(() => window.vcwl.liveListeners()), before - 2);
});

test("no console errors", () => {
  assert.deepEqual(app.errors, []);
});
