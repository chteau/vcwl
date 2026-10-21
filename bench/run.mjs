// Reproducible benchmark: node bench/run.mjs (after `cargo build` and `npm ci` in tests/browser).
// Times each action from the click to the end of the forced style/layout, in headless Chrome.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build, open } from "../tests/browser/harness.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const RUNS = Number(process.env.RUNS ?? 25);
const WARMUP = 5;
const ACTIONS = ["create", "update", "swap", "clear"];

async function measure(name, dist, ready) {
  const app = await open(dist, { ready });
  const times = Object.fromEntries(ACTIONS.map((a) => [a, []]));
  for (let i = 0; i < WARMUP + RUNS; i++) {
    for (const a of ACTIONS) {
      const ms = await app.page.evaluate((label) => {
        const btn = document.querySelector(`[aria-label=${label}]`);
        const t0 = performance.now();
        btn.click();
        void document.body.offsetHeight;
        return performance.now() - t0;
      }, a);
      if (i >= WARMUP) times[a].push(ms);
    }
    if (i === WARMUP) {
      // Sanity check: both implementations render the same rows.
      await app.page.evaluate(() => document.querySelector("[aria-label=create]").click());
      const n = await app.page.$$eval("li", (ls) => ls.length);
      if (n !== 1000) throw new Error(`${name}: expected 1000 rows, found ${n}`);
      await app.page.evaluate(() => document.querySelector("[aria-label=clear]").click());
    }
  }
  const errors = app.errors;
  await app.close();
  if (errors.length) throw new Error(`${name}: ${errors.join("\n")}`);
  const median = (xs) => xs.sort((a, b) => a - b)[xs.length >> 1];
  return Object.fromEntries(ACTIONS.map((a) => [a, median(times[a])]));
}

const results = {
  vcwl: await measure("vcwl", build(path.join(here, "rows")), () => window.vcwl),
  vanilla: await measure("vanilla", path.join(here, "vanilla"), () => window.ready),
};
console.log(`median of ${RUNS} runs (ms), headless Chrome, ${new Date().toISOString().slice(0, 10)}`);
console.log("action".padEnd(10) + "vcwl".padStart(10) + "vanilla".padStart(10) + "ratio".padStart(8));
for (const a of ACTIONS) {
  const [v, j] = [results.vcwl[a], results.vanilla[a]];
  console.log(a.padEnd(10) + v.toFixed(2).padStart(10) + j.toFixed(2).padStart(10) + (v / j).toFixed(1).padStart(8));
}
