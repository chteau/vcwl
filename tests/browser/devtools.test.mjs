import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, repo } from "./harness.mjs";

let app;
before(async () => {
  const dist = build(path.join(repo, "tests/browser/fixtures/devtools"), ["--dev"]);
  app = await open(dist);
  await app.page.waitForFunction(() => window.vcwlDevtools);
});
after(async () => app?.close());

const shadow = (page, fn, ...args) => page.evaluate((src, ...a) => new Function("root", "...a", src)(document.querySelector("vcwl-devtools").shadowRoot, ...a), fn, ...args);

test("the badge opens a panel with the element tree", async () => {
  const { page } = app;
  await shadow(page, "root.querySelector('.badge').click()");
  const rows = await shadow(page, "return [...root.querySelectorAll('.tree .row')].map((r) => r.textContent)");
  assert.ok(rows.some((r) => r.includes("h1")), rows.join("\n"));
  assert.ok(rows.some((r) => r.includes('"Inspect me"')), rows.join("\n"));
});

test("pick selects the element under the pointer and shows its box and style", async () => {
  const { page } = app;
  const t = await tree(page);
  const button = t.one("button");
  await shadow(page, "[...root.querySelectorAll('header button')].find((b) => b.textContent === 'Pick').click()");
  const [x, y, w, h] = button.rect;
  await page.mouse.move(x + w / 2, y + h / 2);
  await page.mouse.down();
  await page.mouse.up();
  const title = await shadow(page, "return root.querySelector('.details h3').textContent");
  assert.equal(title, `button #${button.id}`);
  const style = await shadow(page, "return [...root.querySelectorAll('.details tr')].map((r) => r.textContent).join('|')");
  assert.match(style, /padding8px 16px 8px 16px/);
  assert.equal((await tree(page)).text("button"), "Count 0", "the app did not get the click");
  const hl = await shadow(page, "const s = root.querySelector('.hl').style; return [s.display, parseFloat(s.width)]");
  assert.deepEqual(hl, ["block", w]);
});

test("the performance tab reports frame timings", async () => {
  const { page } = app;
  await shadow(page, "[...root.querySelectorAll('header button')].find((b) => b.textContent === 'Performance').click()");
  const text = await shadow(page, "return root.querySelector('.perf').textContent");
  assert.match(text, /render \/ frame[\d.]+ ms/);
  assert.match(text, /elements\d+/);
});

test("Alt+Shift+D toggles the panel", async () => {
  const { page } = app;
  const isOpen = () => shadow(page, "return root.querySelector('.panel').classList.contains('open')");
  const before = await isOpen();
  await page.keyboard.down("Alt");
  await page.keyboard.down("Shift");
  await page.keyboard.press("KeyD");
  await page.keyboard.up("Shift");
  await page.keyboard.up("Alt");
  assert.equal(await isOpen(), !before);
});

test("no console errors", () => assert.deepEqual(app.errors.filter((e) => !e.includes("404")), []));
