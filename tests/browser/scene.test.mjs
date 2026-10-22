import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  const dist = build(path.join(repo, "tests/browser/fixtures/scene"));
  app = await open(dist);
  await waitForColor(app.page, "box");
});
after(async () => app?.close());

// Shaded versions of the fixture's mesh colors: the red channel dominates for the box,
// blue for the sphere.
const KINDS = {
  box: "r > 110 && r > g * 1.5 && r > b * 1.5",
  sphere: "b > 120 && b > r * 1.6 && b > g * 1.2",
};

/** Count and centroid of the pixels of one mesh inside the scene's box. */
async function find(page, kind) {
  const scene = (await tree(page)).one("scene");
  return page.evaluate(
    (rect, test) => {
      const c = document.querySelector("canvas");
      const k = c.width / c.getBoundingClientRect().width;
      const [x0, y0, w0, h0] = rect.map((v) => Math.round(v * k));
      const d = c.getContext("2d").getImageData(x0, y0, w0, h0).data;
      const match = new Function("r", "g", "b", `return ${test}`);
      let n = 0, sx = 0, sy = 0;
      for (let i = 0; i < d.length; i += 4) {
        if (!match(d[i], d[i + 1], d[i + 2])) continue;
        const p = i / 4;
        n++;
        sx += p % w0;
        sy += Math.floor(p / w0);
      }
      return { n, x: (x0 + sx / n) / k, y: (y0 + sy / n) / k };
    },
    scene.rect,
    KINDS[kind],
  );
}

async function waitForColor(page, kind, pred = (f) => f.n > 200, timeout = 10000) {
  const end = Date.now() + timeout;
  for (;;) {
    const f = await find(page, kind);
    if (pred(f)) return f;
    if (Date.now() > end) throw new Error(`timed out waiting for the ${kind}: ${JSON.stringify(f)}`);
    await new Promise((r) => setTimeout(r, 50));
  }
}

test("the scene draws its meshes into the page canvas", async () => {
  const box = await find(app.page, "box");
  const sphere = await find(app.page, "sphere");
  assert.ok(box.n > 200 && sphere.n > 200, JSON.stringify({ box, sphere }));
  assert.ok(box.x < sphere.x, "the box is left of the sphere");
});

test("clicking a mesh runs its handler", async () => {
  const { page } = app;
  const box = await find(page, "box");
  await page.mouse.click(box.x, box.y);
  await waitFor(page, (t) => t.text("p") === "Hits: 1");
  const sphere = await find(page, "sphere");
  await page.mouse.click(sphere.x, sphere.y);
  await new Promise((r) => setTimeout(r, 200));
  assert.equal((await tree(page)).text("p"), "Hits: 1", "the sphere has no handler");
});

test("state bound to mesh attributes updates the scene", async () => {
  const { page } = app;
  const before = await find(page, "box");
  await click(page, 'button:text("Move")');
  const moved = await waitForColor(page, "box", (f) => f.x > before.x + 20);
  await click(page, 'button:text("Wireframe")');
  const wire = await waitForColor(page, "box", (f) => f.n < moved.n / 3);
  assert.ok(wire.n > 0, "the wireframe edges are drawn");
});

test("dragging orbits the camera without clicking", async () => {
  const { page } = app;
  const scene = (await tree(page)).one("scene");
  // The sphere sits on the orbit target, so follow the box (wireframe off again).
  await click(page, 'button:text("Wireframe")');
  const before = await waitForColor(page, "box");
  const [x, y, w, h] = scene.rect;
  await page.mouse.move(x + 40, y + h - 30);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(x + 40 + i * 20, y + h - 30);
  await page.mouse.up();
  await waitForColor(page, "box", (f) => Math.abs(f.x - before.x) > 15);
  assert.equal((await tree(page)).text("p"), "Hits: 1");
});

test("no errors beyond software-WebGL driver notices", () => {
  // willReadFrequently: this test's own readbacks of the page canvas.
  const real = app.errors.filter((e) => !/WebGL|SwiftShader|GPU stall|GL Driver|willReadFrequently/i.test(e));
  assert.deepEqual(real, []);
});
