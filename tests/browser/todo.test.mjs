import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, text, count, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "examples/todo")));
});
after(async () => app?.close());

async function addTodo(page, title) {
  await click(page, "input[aria-label='New todo']");
  await page.keyboard.type(title);
  await page.keyboard.press("Enter");
}

const labels = async (page) => {
  const t = await tree(page);
  return t.all("li label").map((l) => t.text(l));
};

test("starts and renders into a single canvas", async () => {
  const { page } = app;
  assert.equal(await page.title(), "VCWL Todos");
  // The page's DOM is the canvas; the element tree lives in the renderer.
  assert.deepEqual(await page.$$eval("body *", (els) => els.map((e) => e.tagName)), ["CANVAS"]);
  const t = await tree(page);
  assert.equal(t.text("h1"), "Todos");
  assert.equal(t.count("nav a[href='#/about']"), 1);
  assert.match(t.text("main"), /Nothing to do yet/);
  const btn = t.one("form button");
  assert.equal(btn.attrs.type, "submit");
  assert.ok("disabled" in btn.attrs);
});

test("typing, keyboard submit and reactive updates", async () => {
  const { page } = app;
  await addTodo(page, "write compiler");
  await addTodo(page, "  test it  ");
  await addTodo(page, "ship");
  assert.deepEqual(await labels(page), ["write compiler", "test it", "ship"]);
  const t = await tree(page);
  assert.equal(t.one("input[aria-label='New todo']").value, "");
  assert.match(t.text("footer"), /3 items left/);
});

test("keyed updates keep unrelated nodes", async () => {
  const { page } = app;
  const ids = async () => (await tree(page)).all("li").map((l) => l.id);
  const before = await ids();
  // The first completed item also mounts the "Clear done" button; measure later toggles.
  await click(page, "li:nth-child(3) input");
  const commands = await page.evaluate(() => window.vcwl.commands());
  await click(page, "li:nth-child(2) input");
  await click(page, "li:nth-child(3) input");
  assert.deepEqual(await ids(), before, "list items were recreated");
  const t = await tree(page);
  assert.equal(t.one("li:nth-child(2) input").checked, true);
  assert.equal(t.one("li:nth-child(2) label").decoration, "LineThrough");
  assert.match(t.text("footer"), /2 items left/);
  const used = (await page.evaluate(() => window.vcwl.commands())) - commands;
  assert.ok(used <= 10, `toggling two items took ${used} commands`);
});

test("filters re-render only matching items", async () => {
  const { page } = app;
  await click(page, "footer button:text('Done')");
  assert.deepEqual(await labels(page), ["test it"]);
  assert.equal((await tree(page)).text("footer button[aria-pressed=true]"), "Done");
  await click(page, "footer button:text('Active')");
  assert.deepEqual(await labels(page), ["write compiler", "ship"]);
  await click(page, "footer button:text('All')");
  assert.equal(await count(page, "li"), 3);
});

test("removing items releases event handlers", async () => {
  const { page } = app;
  const stats = () => page.evaluate(() => ({ listeners: window.vcwl.liveListeners(), nodes: window.vcwl.liveNodes() }));
  const before = await stats();
  await click(page, "li:nth-child(1) button");
  assert.equal(await count(page, "li"), 2);
  const after = await stats();
  // Each item has two listeners (checkbox change, delete click).
  assert.equal(before.listeners - after.listeners, 2);
  assert.ok(after.nodes < before.nodes);
  await click(page, "footer > button");
  assert.equal(await count(page, "li"), 1);
  assert.equal((await stats()).listeners, after.listeners - 2 - 1, "item and the hidden Clear button release listeners");
});

test("navigation unmounts the page and restores it", async () => {
  const { page } = app;
  const listenersBefore = await page.evaluate(() => window.vcwl.liveListeners());
  await click(page, "nav a[href='#/about']");
  await waitFor(page, (t) => t.text("h1") === "About");
  assert.equal(await count(page, "li"), 0);
  assert.ok((await page.evaluate(() => window.vcwl.liveListeners())) < listenersBefore, "todo page listeners released");
  await page.goto(app.url + "#/missing");
  await waitFor(page, (t) => t.count("h1") && t.text("h1") === "Page not found");
  await click(page, "a[href='#/']");
  await waitFor(page, (t) => t.text("h1") === "Todos");
  // State lives in the page component, so a remount starts fresh.
  assert.match(await text(page, "main"), /Nothing to do yet/);
});

test("responsive styles follow the viewport", async () => {
  const { page } = app;
  const navTop = (top) => waitFor(page, (t) => t.one("nav").rect[1] === top);
  await page.setViewport({ width: 400, height: 800 });
  await navTop(16);
  await page.setViewport({ width: 1000, height: 800 });
  await navTop(32);
});

test("keyboard only: tab order, visible focus, activation", async () => {
  const { page } = app;
  await page.mouse.click(5, 790);
  await page.keyboard.press("Tab");
  assert.equal((await tree(page)).one(":focus").attrs.href, "#/");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Tab");
  let t = await tree(page);
  assert.equal(t.one(":focus").attrs["aria-label"], "New todo");
  await page.keyboard.type("by keyboard");
  await page.keyboard.press("Enter");
  assert.deepEqual(await labels(page), ["by keyboard"]);
  // Shift+Tab walks backwards; Enter on a link navigates.
  await page.keyboard.down("Shift");
  await page.keyboard.press("Tab");
  await page.keyboard.up("Shift");
  t = await tree(page);
  assert.equal(t.one(":focus").attrs.href, "#/about");
  await page.keyboard.press("Enter");
  await waitFor(page, (t) => t.text("h1") === "About");
  assert.equal(await page.$eval("html", (h) => h.lang), "en");
});

test("no console errors", () => {
  assert.deepEqual(app.errors, []);
});
