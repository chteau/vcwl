import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/slots")), { viewport: { width: 1000, height: 1400 } });
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
    ["framed", "Close"],
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
  await waitFor(page, (t) => t.count("article") === 4);
  // The card's Close button and the slot's Add button.
  assert.equal(await page.evaluate(() => window.vcwl.liveListeners()), before - 2);
});

test("named slots place each block where the component puts it, following the user's state", async () => {
  const t = await tree(app.page);
  const parts = (s) => ["header", "footer"].map((tag) => t.text(s.children.find((c) => c.tag === tag))).concat(t.text(s));
  const [a, b] = t.all("section").map(parts);
  // n is 1 after the earlier test's click.
  assert.deepEqual(a, ["Head", "foot 1", "Headbody 1foot 1"]);
  assert.deepEqual(b, ["only head", "no footer", "only headno footer"]);
});

const names = async (page) => {
  const t = await tree(page);
  return t.all("ul")[0].children.filter((c) => c.tag === "li").map((li) => t.text(li));
};
const hasText = (t, s) => t.nodes.some((n) => n.text === s);

test("a slot passes each item to the user's content, which follows the user's state", async () => {
  const { page } = app;
  // n is 1 after the earlier test's click.
  assert.deepEqual(await names(page), ["ann 1", "bob 1", "cy 1"]);
  await click(page, "li:nth-child(2) button");
  await waitFor(page, (t) => hasText(t, "picked bob"));
});

test("content given for a slot in a keyed list moves and goes with its item", async () => {
  const { page } = app;
  const before = await page.evaluate(() => window.vcwl.liveListeners());
  await click(page, "button:text('Rotate')");
  await waitFor(page, (t) => t.text("li:nth-child(1)") === "cy 1");
  assert.deepEqual(await names(page), ["cy 1", "ann 1", "bob 1"]);
  const t = await tree(page);
  assert.deepEqual(t.all("ol")[0].children.filter((c) => c.tag === "li").map((li) => t.text(li)), ["u cy", "u ann", "u bob"], "unkeyed rows get the new values");
  await click(page, "li:nth-child(2) button");
  await waitFor(page, (t) => hasText(t, "picked ann"));
  await click(page, "button:text('Drop')");
  await waitFor(page, (t) => t.count("li") === 7);
  assert.deepEqual(await names(page), ["cy 1", "ann 1"]);
  assert.equal(await page.evaluate(() => window.vcwl.liveListeners()), before - 1, "the dropped row's handler is released");
});

test("content in a child's list updates when the user's state changes", async () => {
  const { page } = app;
  await click(page, "button:text('Inc')");
  await waitFor(page, (t) => t.text("li:nth-child(1)") === "cy 2");
  assert.deepEqual(await names(page), ["cy 2", "ann 2"]);
});

test("slot content updates only the parts that read a changed value", async () => {
  const { page } = app;
  const before = await page.evaluate(() => window.vcwl.commands());
  await click(page, "button:text('M')");
  await waitFor(page, (t) => hasText(t, "m1"));
  const spent = (await page.evaluate(() => window.vcwl.commands())) - before;
  // One text per row of the second list; the texts reading `n` are left alone.
  assert.equal(spent, 3);
});

test("a component can forward its slot into a child's content", async () => {
  const t = await tree(app.page);
  assert.equal(t.text(t.all("article").at(-1)), "framedinside 2Close");
});

test("a slot inside `if` at a component's root mounts and unmounts its content", async () => {
  const { page } = app;
  assert.ok(!hasText(await tree(page), "folded 2"));
  await click(page, "button:text('Show')");
  await waitFor(page, (t) => hasText(t, "folded 2"));
  await click(page, "button:text('Inc')");
  await waitFor(page, (t) => hasText(t, "folded 3"));
  await click(page, "button:text('Show')");
  await waitFor(page, (t) => !hasText(t, "folded 3"));
});

test("no console errors", () => {
  assert.deepEqual(app.errors, []);
});
