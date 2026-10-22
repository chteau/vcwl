import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  const dist = build(path.join(repo, "tests/browser/fixtures/scene"));
  app = await open(dist, { viewport: { width: 1000, height: 1700 } });
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
  // Third scene: a glTF model with a red material (metallic by default, so darker) and a
  // green checker texture.
  red: "r > 70 && g < 50 && b < 50 && r > g * 2.5",
  green: "g > 150 && r < 100 && b < 100",
  // Fourth scene: where the panes cross, the nearer one's color dominates the mix.
  redOverBlue: "r > 60 && b > 30 && r > b * 1.5",
  blueOverRed: "b > 60 && r > 30 && b > r * 1.5",
  // Fifth and sixth scenes.
  glow: "g > 200 && r < 40 && b < 40",
  orange: "r > 150 && g > 60 && b < 40",
  any: "true",
};

/** Brightness of each pixel along the middle row of scene `n`, from `from` to `to` (fractions of its width). */
async function row(page, n, from, to) {
  const [x, y, w, h] = (await tree(page)).all("scene")[n].rect;
  return page.evaluate(
    (x0, x1, y) => {
      const c = document.querySelector("canvas");
      const k = c.width / c.getBoundingClientRect().width;
      const d = c.getContext("2d").getImageData(Math.round(x0 * k), Math.round(y * k), Math.round((x1 - x0) * k), 1).data;
      const out = [];
      for (let i = 0; i < d.length; i += 4) out.push((d[i] + d[i + 1] + d[i + 2]) / 3);
      return out;
    },
    x + w * from,
    x + w * to,
    y + h / 2,
  );
}
const mean = (a) => a.reduce((s, v) => s + v, 0) / a.length;

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

test("glTF metallic-roughness materials: emissive, metal, occlusion and normal maps", async () => {
  const { page } = app;
  await waitForColor(page, "glow", (f) => f.n > 300, 10000, 3);
  // Front faces of the boxes, centered at 32%, 50%, 68% and 85.5% of the scene's width.
  const face = (at) => row(page, 3, at - 0.035, at + 0.035);
  const metal = mean(await face(0.32)), plastic = mean(await face(0.68));
  assert.ok(metal < plastic * 0.8, `metal reflects no diffuse light: ${metal} vs ${plastic}`);
  const occluded = await face(0.5);
  const half = occluded.length / 2;
  assert.ok(mean(occluded.slice(0, half)) < mean(occluded.slice(half)) - 20, "the occlusion map darkens the left half");
  const bumpy = await face(0.855);
  const steps = bumpy.slice(1).filter((v, i) => Math.abs(v - bumpy[i]) > 25).length;
  assert.ok(steps >= 3, `the normal map's stripes light the flat face unevenly: ${steps} steps`);
});

test("a glTF model plays its first animation", async () => {
  const { page } = app;
  const a = await waitForColor(page, "orange", (f) => f.n > 50, 10000, 4);
  await waitForColor(page, "orange", (f) => f.n > 50 && Math.abs(f.x - a.x) > 20, 3000, 4);
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
  const red = await find(page, "redOverBlue", 5), blue = await find(page, "blueOverRed", 5);
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
