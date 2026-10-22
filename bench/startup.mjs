// Start-up benchmark: node bench/startup.mjs [project] (after `cargo build` and `npm ci` in
// tests/browser). Loads the built app with an empty cache and reports the median time from
// navigation to the first contentful paint and to the first canvas frame, locally and over an
// emulated 40 ms / 20 Mbit/s link.
import path from "node:path";
import { build, repo, launch } from "../tests/browser/harness.mjs";
import { serve } from "../tests/browser/serve.mjs";
const dist = build(path.join(repo, process.argv[2] ?? "examples/tokenforge"));
const { server, url } = await serve(dist, "/");
const browser = await launch();
for (const [name, cond] of [["local", null], ["40ms/20Mbps", { offline: false, latency: 40, downloadThroughput: 20e6 / 8, uploadThroughput: 5e6 / 8 }]]) {
  const xs = [];
  for (let i = 0; i < 7; i++) {
    const ctx = await browser.createBrowserContext();
    const page = await ctx.newPage();
    await page.setViewport({ width: 1280, height: 800 });
    const cdp = await page.createCDPSession();
    await cdp.send("Network.setCacheDisabled", { cacheDisabled: true });
    if (cond) await cdp.send("Network.emulateNetworkConditions", cond);
    await page.goto(url);
    await page.waitForFunction(() => performance.getEntriesByName("vcwl:first-frame").length, { timeout: 20000 });
    xs.push(await page.evaluate(() => {
      const at = (n) => performance.getEntriesByName(n)[0]?.startTime.toFixed(0);
      const f = performance.getEntriesByName("vcwl:first-frame")[0].startTime;
      const res = performance.getEntriesByType("resource").map(r => `${r.name.split("/").pop()} ${r.startTime.toFixed(0)}-${r.responseEnd.toFixed(0)}`);
      res.push(`compiled ${at("vcwl:compiled")}`, `started ${at("vcwl:started")}`);
      const fcp = performance.getEntriesByName("first-contentful-paint")[0]?.startTime ?? NaN;
      return { f, fcp, res };
    }));
    await ctx.close();
  }
  const median = (k) => xs.map((x) => x[k]).sort((a, b) => a - b)[xs.length >> 1];
  // Chrome reports no first-contentful-paint for a page that is only a canvas.
  const fcp = median("fcp") ?? NaN;
  console.log(`${name}: first contentful paint ${Number.isNaN(fcp) ? "n/a" : fcp.toFixed(1) + " ms"}, first canvas frame ${median("f").toFixed(1)} ms`);
  if (process.env.VERBOSE) console.log("  " + xs[0].res.join(" | "));
}
await browser.close(); server.close();
