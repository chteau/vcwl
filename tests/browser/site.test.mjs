import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "site")), { viewport: { width: 1200, height: 5200 } });
});
after(async () => app?.close());

test("the project site renders, satire and disclaimer included", async () => {
  const t = await tree(app.page);
  assert.ok(t.nodes.some((n) => n.text === "VCWL is going to revolutionize web development."));
  assert.ok(t.nodes.some((n) => n.text?.startsWith("// satire.")));
  assert.equal(t.all("article").length, 16);
});

test("the terminal types its lines one by one", async () => {
  await waitFor(app.page, (t) => t.nodes.some((n) => n.text === "  * it has not"), 10000);
});

test("the hot take generator draws a take and a score through @web", async () => {
  await click(app.page, "button:text('Generate hot take')");
  await waitFor(app.page, (t) => t.nodes.some((n) => /^\d+\/100 · (?!unmeasured)/.test(n.text ?? "")));
});

test("a touch flick keeps scrolling after the finger lifts", async () => {
  const phone = await open(path.join(repo, "site/dist"), { hash: "docs/tour/", viewport: { width: 390, height: 800 } });
  try {
    await flick(phone.page);
  } finally {
    await phone.close();
  }
});

test("phones get a menu and a folded docs page list; canvas fits the screen", async () => {
  const phone = await open(path.join(repo, "site/dist"), {
    hash: "docs/styles/",
    viewport: { width: 390, height: 800, isMobile: true, hasTouch: true },
  });
  try {
    const { page } = phone;
    // A wide prerendered page must not leave the canvas laid out wider than the screen.
    assert.equal(await page.evaluate(() => document.querySelector("canvas").width), 390);
    assert.equal(shown(await tree(page), "a:text('Quick start')"), 0);
    await click(page, "button[aria-label='Documentation pages']");
    await waitFor(page, (t) => shown(t, "a:text('Quick start')") === 1);
    assert.equal(shown(await tree(page), "a:text('packages')"), 0);
    await click(page, "button[aria-label='Menu']");
    await waitFor(page, (t) => shown(t, "a:text('packages')") === 1);
    const [x, y, w, h] = (await tree(page)).all("a:text('packages')").find((n) => n.rect[2] > 0).rect;
    await page.mouse.click(x + w / 2, y + h / 2);
    await waitFor(page, (t) => t.text("h1") === "Packages");
    assert.equal(shown(await tree(page), "a:text('github ↗')"), 0, "the menu closes after a pick");
  } finally {
    await phone.close();
  }
});

test("dragging over painted text selects it and Ctrl+C copies it", async () => {
  const { page } = app;
  await page.evaluate(() => {
    window.copied = [];
    navigator.clipboard.writeText = async (t) => window.copied.push(t);
  });
  const [x, y] = (await tree(page)).nodes.find((n) => n.text?.startsWith("The Vibe Coded")).rect;
  await page.mouse.move(x + 1, y + 8);
  await page.mouse.down();
  await page.mouse.move(x + 150, y + 8, { steps: 4 });
  await page.mouse.up();
  await page.keyboard.down("Control");
  await page.keyboard.press("c");
  await page.keyboard.up("Control");
  await page.waitForFunction(() => window.copied[0]?.startsWith("The Vibe"), { timeout: 5000 });
  await page.mouse.click(x - 30, y - 30);
});

/** How many elements matching `sel` are laid out (hidden ones stay in the tree with no size). */
const shown = (t, sel) => t.all(sel).filter((n) => n.rect[2] > 0).length;

async function flick(page) {
  const cdp = await page.createCDPSession();
  await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: true, maxTouchPoints: 1 });
  const y = async () => (await tree(page)).one("h1").rect[1];
  const touch = (type, py) =>
    cdp.send("Input.dispatchTouchEvent", { type, touchPoints: type === "touchEnd" ? [] : [{ x: 200, y: py }] });
  const top = await y();
  await touch("touchStart", 700);
  for (let py = 660; py >= 420; py -= 40) {
    await touch("touchMove", py);
    await new Promise((r) => setTimeout(r, 16));
  }
  await touch("touchEnd");
  const released = await y();
  assert.ok(released < top - 200, `dragged: ${top} -> ${released}`);
  await new Promise((r) => setTimeout(r, 600));
  const after = await y();
  assert.ok(after < released - 40, `kept going: ${released} -> ${after}`);
}
