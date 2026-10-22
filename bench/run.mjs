// Reproducible benchmark: node bench/run.mjs (after `cargo build` and `npm ci` in tests/browser).
//
// Three timings per action, in headless Chrome:
// - "sync": from the click to the end of the synchronous work. For VCWL that is event
//   handling, the app update, style, layout, display list and canvas drawing calls. For the
//   hand-written DOM version it is the DOM update plus a forced style/layout (its painting
//   happens later, inside the browser, and is not included).
// - "frame": from the click until the next frame has been produced (requestAnimationFrame
//   followed by a task), which includes the browser's own rendering for both.
// - "settled": for VCWL, until the accessibility mirror (DOM elements updated in a task after
//   the frame) is up to date and laid out. The DOM version is its own accessibility tree, so
//   this is its frame time plus a forced layout.
// Between actions both wait until nothing is pending, so work does not spill into the next one.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build, open, tree } from "../tests/browser/harness.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const RUNS = Number(process.env.RUNS ?? 25);
const WARMUP = 5;
const ACTIONS = ["create", "update", "swap", "clear"];
const MODES = ["sync", "frame", "settled"];

const nextFrame = () => new Promise((r) => requestAnimationFrame(() => setTimeout(r, 0)));

/** Returns `act(label, mode)` that performs an action and reports its time in ms. */
async function vcwlActions(page) {
  const t = await tree(page);
  const at = Object.fromEntries(ACTIONS.map((a) => {
    const [x, y, w, h] = t.one(`button[aria-label=${a}]`).rect;
    return [a, [x + w / 2, y + h / 2]];
  }));
  return (label, mode) =>
    page.evaluate(async ([x, y], mode, nextFrame) => {
      nextFrame = eval(nextFrame);
      const canvas = document.querySelector("canvas");
      const ev = (type) => canvas.dispatchEvent(new PointerEvent(type, { clientX: x, clientY: y, pointerId: 1, bubbles: true }));
      const t0 = performance.now();
      ev("pointerdown");
      ev("pointerup");
      if (mode === "sync") window.vcwl.paintNow();
      else if (mode === "frame") await nextFrame();
      else {
        await window.vcwl.mirrored();
        void document.body.offsetHeight;
      }
      const ms = performance.now() - t0;
      await window.vcwl.mirrored();
      await nextFrame();
      return ms;
    }, at[label], mode, nextFrame.toString());
}

function domActions(page) {
  return (label, mode) =>
    page.evaluate(async (label, mode, nextFrame) => {
      nextFrame = eval(nextFrame);
      const btn = document.querySelector(`[aria-label=${label}]`);
      const t0 = performance.now();
      btn.click();
      if (mode !== "sync") await nextFrame();
      if (mode !== "frame") void document.body.offsetHeight;
      const ms = performance.now() - t0;
      await nextFrame();
      return ms;
    }, label, mode, nextFrame.toString());
}

async function measure(name, dist, rows) {
  const app = await open(dist, { ready: name === "vanilla" ? () => window.ready : undefined });
  const act = name === "vanilla" ? domActions(app.page) : await vcwlActions(app.page);
  const key = (a, mode) => (mode === "sync" ? a : `${a}/${mode}`);
  const times = Object.fromEntries(MODES.flatMap((m) => ACTIONS.map((a) => [key(a, m), []])));
  for (let i = 0; i < WARMUP + RUNS; i++) {
    for (const mode of MODES) {
      for (const a of ACTIONS) {
        const ms = await act(a, mode);
        if (i >= WARMUP) times[key(a, mode)].push(ms);
      }
    }
    if (i === WARMUP) {
      // Sanity check: both implementations render the same rows.
      await act("create", "sync");
      const n = await rows(app.page);
      if (n !== 1000) throw new Error(`${name}: expected 1000 rows, found ${n}`);
      await act("clear", "sync");
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
