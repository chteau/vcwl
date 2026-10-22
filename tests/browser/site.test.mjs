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
  assert.equal(t.all("article").length, 15);
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
