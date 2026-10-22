import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/reorder")));
});
after(async () => app?.close());

/** Types `keys` into the form and submits; returns how many attached nodes were moved. */
async function apply(keys) {
  const { page } = app;
  const before = await page.evaluate(() => window.vcwl.moves());
  await click(page, "input");
  await page.keyboard.down("Control");
  await page.keyboard.press("a");
  await page.keyboard.up("Control");
  await page.keyboard.press("Backspace");
  await page.keyboard.type(keys.join(","));
  await page.keyboard.press("Enter");
  return (await page.evaluate(() => window.vcwl.moves())) - before;
}

/** Keys of the rendered items, and the node id of each. */
async function rendered() {
  const t = await tree(app.page);
  return t.all("li").map((l) => [Number(t.text(l)), l.id]);
}

function lisLength(xs) {
  const tails = [];
  for (const x of xs) {
    let lo = 0, hi = tails.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (tails[mid] < x) lo = mid + 1;
      else hi = mid;
    }
    tails[lo] = x;
  }
  return tails.length;
}

// Deterministic pseudo-random generator so failures are reproducible.
let seed = 12345;
const rand = (n) => ((seed = (seed * 1103515245 + 12345) % 2 ** 31) % n);

test("random permutations, insertions and removals render in order with minimal moves", async () => {
  let prev = [];
  for (let round = 0; round < 60; round++) {
    const pool = Array.from({ length: 40 }, (_, i) => i + 1);
    const keys = pool.filter(() => rand(3) !== 0);
    for (let i = keys.length - 1; i > 0; i--) {
      if (rand(4) === 0) {
        const j = rand(i + 1);
        [keys[i], keys[j]] = [keys[j], keys[i]];
      }
    }
    const ids = new Map(await rendered());
    const moves = await apply(keys);
    const now = await rendered();
    assert.deepEqual(now.map(([k]) => k), keys, `round ${round}`);
    // Nodes of keys present before and after are reused, never recreated.
    const kept = keys.filter((k) => prev.includes(k));
    assert.deepEqual(now.filter(([k, id]) => ids.get(k) === id).map(([k]) => k), kept, `round ${round}: kept nodes`);
    const oldIndex = kept.map((k) => prev.indexOf(k));
    assert.equal(moves, kept.length - lisLength(oldIndex), `round ${round}: moves`);
    prev = keys;
  }
  assert.deepEqual(app.errors, []);
});

test("reversal and clearing", async () => {
  const keys = Array.from({ length: 30 }, (_, i) => i + 1);
  await apply(keys);
  assert.equal(await apply([...keys].reverse()), 29);
  assert.deepEqual((await rendered()).map(([k]) => k), [...keys].reverse());
  await apply([]);
  assert.deepEqual(await rendered(), []);
  assert.equal(await app.page.evaluate(() => window.vcwl.liveListeners()), 2, "only the form's listeners remain");
});
