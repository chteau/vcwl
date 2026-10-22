// Reproducible benchmark: node bench/run.mjs (after `cargo build` and `npm ci` in tests/browser).
//
// Two timings per action, in headless Chrome:
// - "sync": from the click to the end of the synchronous work. For VCWL that is event
//   handling, the app update, style, layout, display list and canvas drawing calls. For the
//   hand-written DOM version it is the DOM update plus a forced style/layout (its painting
//   happens later, inside the browser, and is not included).
// - "frame": from the click until the next frame has been produced (requestAnimationFrame
//   followed by a task), which includes the browser's own rendering for both.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build, open, tree } from "../tests/browser/harness.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const RUNS = Number(process.env.RUNS ?? 25);
const WARMUP = 5;
const ACTIONS = ["create", "update", "swap", "clear"];

/** Returns `act(label, frame)` that performs an action and reports its time in ms. */
async function vcwlActions(page) {
  const t = await tree(page);
  const at = Object.fromEntries(ACTIONS.map((a) => {
    const [x, y, w, h] = t.one(`button[aria-label=${a}]`).rect;
    return [a, [x + w / 2, y + h / 2]];
  }));
  return (label, frame) =>
    page.evaluate(async ([x, y], frame) => {
      const canvas = document.querySelector("canvas");
      const ev = (type) => canvas.dispatchEvent(new PointerEvent(type, { clientX: x, clientY: y, pointerId: 1, bubbles: true }));
      const t0 = performance.now();
      ev("pointerdown");
      ev("pointerup");
      if (frame) await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
      else window.vcwl.paintNow();
      return performance.now() - t0;
    }, at[label], frame);
}

function domActions(page) {
  return (label, frame) =>
    page.evaluate(async (label, frame) => {
      const btn = document.querySelector(`[aria-label=${label}]`);
      const t0 = performance.now();
      btn.click();
      if (frame) await new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));
      else void document.body.offsetHeight;
      return performance.now() - t0;
    }, label, frame);
}

async function measure(name, dist, rows) {
  const app = await open(dist, { ready: name === "vanilla" ? () => window.ready : undefined });
  const act = name === "vanilla" ? domActions(app.page) : await vcwlActions(app.page);
  const times = Object.fromEntries(ACTIONS.flatMap((a) => [[a, []], [a + "/frame", []]]));
  for (let i = 0; i < WARMUP + RUNS; i++) {
    for (const frame of [false, true]) {
      for (const a of ACTIONS) {
        const ms = await act(a, frame);
        if (i >= WARMUP) times[frame ? a + "/frame" : a].push(ms);
      }
    }
    if (i === WARMUP) {
      // Sanity check: both implementations render the same rows.
      await act("create", false);
      const n = await rows(app.page);
      if (n !== 1000) throw new Error(`${name}: expected 1000 rows, found ${n}`);
      await act("clear", false);
    }
  }
  const errors = app.errors;
  await app.close();
  if (errors.length) throw new Error(`${name}: ${errors.join("\n")}`);
  const median = (xs) => xs.sort((a, b) => a - b)[xs.length >> 1];
  return Object.fromEntries(Object.entries(times).map(([k, v]) => [k, median(v)]));
}

const results = {
  vcwl: await measure("vcwl", build(path.join(here, "rows")), async (page) => (await tree(page)).count("li")),
  vanilla: await measure("vanilla", path.join(here, "vanilla"), (page) => page.$$eval("li", (ls) => ls.length)),
};
console.log(`median of ${RUNS} runs (ms), headless Chrome, ${new Date().toISOString().slice(0, 10)}`);
console.log("action".padEnd(16) + "vcwl".padStart(10) + "vanilla".padStart(10) + "ratio".padStart(8));
for (const a of Object.keys(results.vcwl)) {
  const [v, j] = [results.vcwl[a], results.vanilla[a]];
  console.log(a.padEnd(16) + v.toFixed(2).padStart(10) + j.toFixed(2).padStart(10) + (v / j).toFixed(1).padStart(8));
}
