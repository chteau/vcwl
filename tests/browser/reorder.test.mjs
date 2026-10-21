import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/reorder")));
});
after(async () => app?.close());

/** Applies `keys`; returns the DOM moves (existing nodes re-inserted) the update made. */
async function apply(keys) {
  return app.page.evaluate(async (text) => {
    const ul = document.querySelector("ul");
    const before = new Set(ul.children);
    const moved = new Set();
    const obs = new MutationObserver((recs) => recs.forEach((r) => r.addedNodes.forEach((n) => before.has(n) && moved.add(n))));
    obs.observe(ul, { childList: true });
    const input = document.querySelector("input");
    input.value = text;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    document.querySelector("form").requestSubmit();
    await Promise.resolve();
    obs.takeRecords().forEach((r) => r.addedNodes.forEach((n) => before.has(n) && moved.add(n)));
    obs.disconnect();
    return moved.size;
  }, keys.join(","));
}

const rendered = () => app.page.$$eval("li", (ls) => ls.map((l) => Number(l.textContent)));

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
    await app.page.$$eval("li", (ls) => ls.forEach((l) => (l.__key = Number(l.textContent))));
    const moves = await apply(keys);
    assert.deepEqual(await rendered(), keys, `round ${round}`);
    // Nodes of keys present before and after are reused, never recreated.
    const reused = await app.page.$$eval("li", (ls) => ls.filter((l) => l.__key !== undefined).map((l) => l.__key));
    const kept = keys.filter((k) => prev.includes(k));
    assert.deepEqual(reused, kept, `round ${round}: kept nodes`);
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
  assert.deepEqual(await rendered(), [...keys].reverse());
  await apply([]);
  assert.deepEqual(await rendered(), []);
  assert.equal(await app.page.evaluate(() => window.vcwl.liveListeners()), 2, "only the form's listeners remain");
});
