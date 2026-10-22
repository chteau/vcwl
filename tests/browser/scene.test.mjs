import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  const dist = build(path.join(repo, "tests/browser/fixtures/scene"));
  app = await open(dist, { viewport: { width: 1000, height: 1200 } });
  await waitForColor(app.page, "box");
});
after(async () => app?.close());

// Shaded versions of the fixture's mesh colors: the red channel dominates for the box,
// blue for the sphere.
const KINDS = {
  box: "r > 110 && r > g * 1.5 && r > b * 1.5",
  sphere: "b > 120 && b > r * 1.6 && b > g * 1.2",
  // Second scene.
  pyramid: "r > 150 && g > 120 && b < 70",
  tetra: "g > 120 && b > 80 && r < 60 && g > b",
  checker: "r > 120 && b > 120 && g < 90",
  glass: "b > 170 && g > 110 && r < 90",
  // Third scene: a glTF model with a red material and a green checker texture.
  red: "r > 150 && g < 70 && b < 70",
  green: "g > 150 && r < 100 && b < 100",
  // Fourth scene: where the panes cross, the nearer one's color dominates the mix.
  redOverBlue: "r > 60 && b > 30 && r > b * 1.5",
  blueOverRed: "b > 60 && r > 30 && b > r * 1.5",
  any: "true",
};

/** Count, centroid and mean brightness of the pixels of one mesh inside scene `n`'s box. */
async function find(page, kind, n = 0) {
  const scene = (await tree(page)).all("scene")[n];
  return page.evaluate(
    (rect, test) => {
      const c = document.querySelector("canvas");
      const k = c.width / c.getBoundingClientRect().width;
      const [x0, y0, w0, h0] = rect.map((v) => Math.round(v * k));
      const d = c.getContext("2d").getImageData(x0, y0, w0, h0).data;
      const match = new Function("r", "g", "b", `return ${test}`);
      let n = 0, sx = 0, sy = 0, light = 0;
      for (let i = 0; i < d.length; i += 4) {
        if (!match(d[i], d[i + 1], d[i + 2])) continue;
        const p = i / 4;
        n++;
        light += d[i] + d[i + 1] + d[i + 2];
        sx += p % w0;
        sy += Math.floor(p / w0);
      }
      return { n, x: (x0 + sx / n) / k, y: (y0 + sy / n) / k, light: light / n / 3 };
    },
    scene.rect,
    KINDS[kind],
  );
}

async function waitForColor(page, kind, pred = (f) => f.n > 200, timeout = 10000, scene = 0) {
  const end = Date.now() + timeout;
  for (;;) {
    const f = await find(page, kind, scene);
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

test("meshes load OBJ and glTF models and image textures", async () => {
  const { page } = app;
  for (const kind of ["pyramid", "tetra", "checker"]) await waitForColor(page, kind, (f) => f.n > 150, 10000, 1);
});

test("glTF materials give each part its base color or embedded texture", async () => {
  const { page } = app;
  for (const kind of ["red", "green"]) await waitForColor(page, kind, (f) => f.n > 100, 10000, 2);
});

test("shadows darken the floor under the meshes", async () => {
  const { page } = app;
  const before = await find(page, "any", 1);
  const point = await find(page, "any", 2);
  await click(page, 'button:text("Shadows")');
  await waitForColor(page, "any", (f) => f.light < before.light - 1, 5000, 1);
  // The third scene's shadows come from a point light.
  await waitForColor(page, "any", (f) => f.light < point.light - 1, 5000, 2);
});

test("a translucent mesh lets what is behind it show through", async () => {
  const { page } = app;
  const before = await find(page, "glass", 1);
  assert.ok(before.n > 400, JSON.stringify(before));
  await click(page, 'button:text("Fade")');
  await waitForColor(page, "glass", (f) => f.n < before.n / 2, 5000, 1);
});

test("crossing translucent meshes blend in depth order at each pixel", async () => {
  const { page } = app;
  const red = await find(page, "redOverBlue", 3), blue = await find(page, "blueOverRed", 3);
  // The red pane is in front on the left of the crossing, the blue one on the right.
  assert.ok(red.n > 300 && blue.n > 300, JSON.stringify({ red, blue }));
  assert.ok(red.x < blue.x - 20, JSON.stringify({ red, blue }));
});

test("keyboard: Tab reaches the orbiting scene and its clickable mesh", async () => {
  const { page } = app;
  const label = await page.evaluate(() => document.querySelector('button[aria-label="Box"]')?.textContent ?? null);
  assert.equal(label, "", "the clickable mesh is a button in the accessibility mirror");
  await page.mouse.click(5, 995);
  await page.keyboard.press("Tab");
  assert.equal((await tree(page)).one(":focus").tag, "scene");
  // Arrow keys turn the camera.
  const before = await find(page, "box");
  for (let i = 0; i < 4; i++) await page.keyboard.press("ArrowLeft");
  await waitForColor(page, "box", (f) => Math.abs(f.x - before.x) > 15);
  await page.keyboard.press("Tab");
  assert.equal((await tree(page)).one(":focus").attrs["aria-label"], "Box");
  const hits = (await tree(page)).text("p");
  await page.keyboard.press("Enter");
  await waitFor(page, (t) => t.text("p") !== hits);
});

test("no errors beyond software-WebGL driver notices", () => {
  // willReadFrequently: this test's own readbacks of the page canvas.
  const real = app.errors.filter((e) => !/WebGL|SwiftShader|GPU stall|GL Driver|willReadFrequently/i.test(e));
  assert.deepEqual(real, []);
});
