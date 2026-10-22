import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/styles")), { viewport: { width: 1000, height: 300 } });
});
after(async () => app?.close());

/** RGB of the canvas pixel at (dx, dy) from the top left corner of element `id`. */
async function pixel(id, dx, dy) {
  const [x, y] = (await tree(app.page)).one(`[id=${id}]`).rect;
  return app.page.evaluate(
    (x, y) => {
      const c = document.querySelector("canvas");
      // One readback per test run.
      window.__snap ??= c.getContext("2d").getImageData(0, 0, c.width, c.height);
      const k = c.width / c.getBoundingClientRect().width;
      const i = (Math.round(y * k) * c.width + Math.round(x * k)) * 4;
      return [...window.__snap.data.slice(i, i + 3)];
    },
    x + dx,
    y + dy,
  );
}
const blue = ([r, g, b]) => b > 200 && r < 100;

test("corner shapes: cut and scoop remove what a round corner keeps", async () => {
  // (12, 12) is inside a 30px round corner, outside a 30px cut; (18, 18) is inside the cut,
  // outside the scoop.
  assert.ok(blue(await pixel("round", 12, 12)));
  assert.ok(!blue(await pixel("cut", 12, 12)) && blue(await pixel("cut", 18, 18)));
  assert.ok(!blue(await pixel("scoop", 18, 18)) && blue(await pixel("scoop", 50, 50)));
});

test("a squircle corner is shaped differently from a round one", async () => {
  let differ = 0;
  for (let d = 0; d < 30; d++) for (const [dx, dy] of [[d, 3], [3, d], [d, d]]) if (blue(await pixel("round", dx, dy)) !== blue(await pixel("squircle", dx, dy))) differ++;
  assert.ok(differ > 3, `${differ} pixels differ`);
});

test("grain varies the background from pixel to pixel", async () => {
  const values = new Set();
  for (let i = 0; i < 20; i++) values.add((await pixel("grain", 40 + i, 50)).join());
  assert.ok(values.size > 5, [...values].join(" "));
  assert.equal(new Set(await Promise.all([40, 50, 60].map((i) => pixel("round", i, 50).then(String)))).size, 1);
});

test("a sketched border wobbles past the box", async () => {
  let ink = 0;
  for (let i = -6; i <= 6; i++) for (const [dx, dy] of [[i, 50], [50, i], [100 + i, 50]]) if ((await pixel("sketch", dx, dy))[0] < 120) ink++;
  assert.ok(ink >= 3, `${ink} dark pixels around the edges`);
  // Not a straight solid border: the line moves along the side.
  const rows = [];
  for (const dy of [10, 50, 90]) {
    for (let dx = -6; dx <= 6; dx++) if ((await pixel("sketch", dx, dy))[0] < 120) { rows.push(dx); break; }
  }
  assert.ok(new Set(rows).size > 1 || rows.length < 3, `left edge at ${rows}`);
});

test("no console errors", () => assert.deepEqual(app.errors, []));
