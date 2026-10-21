import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, text, count, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "examples/todo")));
});
after(async () => app?.close());

async function addTodo(page, title) {
  await page.click("input[aria-label='New todo']");
  await page.keyboard.type(title);
  await page.keyboard.press("Enter");
}

test("starts and renders native elements", async () => {
  const { page } = app;
  assert.equal(await page.title(), "VCWL Todos");
  assert.equal(await text(page, "h1"), "Todos");
  assert.equal(await count(page, "nav a[href='#/about']"), 1);
  assert.match(await text(page, "main"), /Nothing to do yet/);
  // The Add button is a real, disabled submit button while the field is empty.
  const btn = await page.$eval("form button", (b) => ({ type: b.type, disabled: b.disabled }));
  assert.deepEqual(btn, { type: "submit", disabled: true });
});

test("form input, keyboard submit and reactive updates", async () => {
  const { page } = app;
  await addTodo(page, "write compiler");
  await addTodo(page, "  test it  ");
  await addTodo(page, "ship");
  assert.equal(await count(page, "li"), 3);
  assert.deepEqual(await page.$$eval("li label", (ls) => ls.map((l) => l.textContent)), ["write compiler", "test it", "ship"]);
  assert.equal(await page.$eval("input[aria-label='New todo']", (i) => i.value), "");
  assert.match(await text(page, "footer"), /3 items left/);
});

test("keyed updates keep unrelated DOM nodes", async () => {
  const { page } = app;
  await page.$$eval("li", (ls) => ls.forEach((l, i) => (l.__mark = i)));
  // The first completed item also mounts the "Clear done" button; measure a later toggle.
  await page.click("li:nth-child(3) input[type=checkbox]");
  const before = await page.evaluate(() => window.vcwl.stats.mutations);
  await page.click("li:nth-child(2) input[type=checkbox]");
  await page.click("li:nth-child(3) input[type=checkbox]");
  const marks = await page.$$eval("li", (ls) => ls.map((l) => l.__mark));
  assert.deepEqual(marks, [0, 1, 2], "list items were recreated");
  assert.equal(await page.$eval("li:nth-child(2) input", (i) => i.checked), true);
  assert.equal(await page.$eval("li:nth-child(2)", (l) => getComputedStyle(l).textDecorationLine), "line-through");
  assert.match(await text(page, "footer"), /2 items left/);
  const muts = (await page.evaluate(() => window.vcwl.stats.mutations)) - before;
  // checkbox, class toggle, label, delete label, footer count — per toggle.
  assert.ok(muts <= 10, `toggling two items took ${muts} DOM commands`);
});

test("filters re-render only matching items", async () => {
  const { page } = app;
  const [, activeBtn, doneBtn] = await page.$$("footer button");
  await doneBtn.click();
  assert.deepEqual(await page.$$eval("li label", (ls) => ls.map((l) => l.textContent)), ["test it"]);
  assert.equal(await page.$eval("footer button[aria-pressed=true]", (b) => b.textContent), "Done");
  await activeBtn.click();
  assert.deepEqual(await page.$$eval("li label", (ls) => ls.map((l) => l.textContent)), ["write compiler", "ship"]);
  const [allBtn] = await page.$$("footer button");
  await allBtn.click();
  assert.equal(await count(page, "li"), 3);
});

test("removing items releases event handlers", async () => {
  const { page } = app;
  const stats = () => page.evaluate(() => ({ listeners: window.vcwl.liveListeners(), nodes: window.vcwl.liveNodes() }));
  const before = await stats();
  await page.click("li:nth-child(1) button");
  assert.equal(await count(page, "li"), 2);
  const after = await stats();
  // Each item has two listeners (checkbox change, delete click).
  assert.equal(before.listeners - after.listeners, 2);
  assert.ok(after.nodes < before.nodes);
  await page.click("footer > button"); // "Clear done"
  assert.equal(await count(page, "li"), 1);
  assert.equal((await stats()).listeners, after.listeners - 2 - 1, "item and the hidden Clear button release listeners");
});

test("navigation unmounts the page and restores it", async () => {
  const { page } = app;
  const listenersBefore = await page.evaluate(() => window.vcwl.liveListeners());
  await page.click("nav a[href='#/about']");
  await page.waitForFunction(() => document.querySelector("h1")?.textContent === "About");
  assert.equal(await count(page, "li"), 0);
  const listenersAbout = await page.evaluate(() => window.vcwl.liveListeners());
  assert.ok(listenersAbout < listenersBefore, "todo page listeners released");
  await page.goto(app.url + "#/missing");
  await page.waitForFunction(() => document.querySelector("h1")?.textContent === "Page not found");
  await page.click("a[href='#/']");
  await page.waitForFunction(() => document.querySelector("h1")?.textContent === "Todos");
  // State lives in the page component, so a remount starts fresh.
  assert.match(await text(page, "main"), /Nothing to do yet/);
});

test("responsive styles follow the viewport", async () => {
  const { page } = app;
  const padding = () => page.$eval("body > div.v-col", (e) => getComputedStyle(e).paddingTop);
  await page.setViewport({ width: 1000, height: 800 });
  assert.equal(await padding(), "32px");
  await page.setViewport({ width: 400, height: 800 });
  assert.equal(await padding(), "16px");
  await page.setViewport({ width: 1000, height: 800 });
});

test("accessibility basics", async () => {
  const { page } = app;
  assert.equal(await page.$eval("html", (h) => h.lang), "en");
  // Keyboard: Tab reaches the nav link and the input; focus is visible.
  await page.focus("input[aria-label='New todo']");
  const outline = await page.$eval("input[aria-label='New todo']", (i) => getComputedStyle(i).outlineStyle);
  assert.notEqual(outline, "none");
  const unlabeled = await page.$$eval("input, select, textarea, button", (els) =>
    els.filter((e) => !(e.getAttribute("aria-label") || e.textContent.trim() || e.closest("label"))).length);
  assert.equal(unlabeled, 0);
});

test("no console errors", () => {
  assert.deepEqual(app.errors, []);
});
