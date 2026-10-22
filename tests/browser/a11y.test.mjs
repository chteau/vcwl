import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "examples/todo")));
});
after(async () => app?.close());

/** Flattens Chrome's accessibility tree into "role: name" lines. */
async function axLines(page) {
  const lines = [];
  const walk = (n) => {
    if (n.role !== "none" && n.role !== "generic" && (n.name || n.role !== "StaticText")) lines.push(`${n.role}: ${n.name ?? ""}`);
    (n.children ?? []).forEach(walk);
  };
  walk(await page.accessibility.snapshot());
  return lines;
}

test("assistive technology sees the page's structure", async () => {
  const { page } = app;
  const ax = await page.accessibility.snapshot();
  const find = (n, pred) => (pred(n) ? n : (n.children ?? []).map((c) => find(c, pred)).find(Boolean));
  const h1 = find(ax, (n) => n.role === "heading" && n.name === "Todos");
  assert.equal(h1?.level, 1);
  assert.ok(find(ax, (n) => n.role === "link" && n.name === "About"));
  assert.ok(find(ax, (n) => n.role === "textbox" && n.name === "New todo"));
  const add = find(ax, (n) => n.role === "button" && n.name === "Add");
  assert.equal(add?.disabled, true);
  assert.ok((await axLines(page)).some((l) => l.includes("Nothing to do yet")));
});

test("mirror elements sit where the canvas draws them", async () => {
  const { page } = app;
  const [x, y, w, h] = (await tree(page)).one("h1").rect;
  const r = await page.$eval("h1", (e) => e.getBoundingClientRect().toJSON());
  assert.deepEqual([r.x, r.y, r.width, r.height].map(Math.round), [x, y, w, h].map(Math.round));
});

test("text input goes through the browser (IME, characters without key presses)", async () => {
  const { page } = app;
  await click(page, "input[aria-label='New todo']");
  assert.equal(await page.evaluate(() => document.activeElement.getAttribute("aria-label")), "New todo");
  await page.keyboard.type("café");
  await page.keyboard.sendCharacter("…");
  await page.keyboard.sendCharacter("漢字");
  assert.equal((await tree(page)).one("input[aria-label='New todo']").value, "café…漢字");
  await page.keyboard.press("Enter");
  await waitFor(page, (t) => t.count("li") === 1);
  assert.equal((await tree(page)).one("input[aria-label='New todo']").value, "");
  assert.equal(await page.$eval("input[aria-label='New todo']", (e) => e.value), "");
});

test("screen reader activation reaches the application", async () => {
  const { page } = app;
  const name = (await axLines(page)).find((l) => l.startsWith("checkbox:"));
  assert.ok(name, "a checkbox is exposed");
  await page.$eval("li input[type=checkbox]", (e) => e.click());
  await waitFor(page, (t) => t.one("li input[type=checkbox]").checked === true);
  assert.equal(await page.$eval("li input[type=checkbox]", (e) => e.checked), true);
});

test("no console errors", () => {
  assert.deepEqual(app.errors, []);
});
