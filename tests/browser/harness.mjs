// Shared browser-test setup: build an example, serve it, open it in headless Chrome.
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer-core";
import { serve } from "./serve.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
export const repo = path.resolve(here, "../..");
const vcwl = path.join(repo, "target/debug/vcwl");
const chrome = process.env.CHROME ?? "/usr/bin/google-chrome";

export function build(projectDir, args = []) {
  execFileSync(vcwl, ["build", ...args], { cwd: projectDir, stdio: "pipe" });
  return path.join(projectDir, "dist");
}

export async function open(dist, { base = "/", hash = "", viewport = { width: 1000, height: 800 }, api = null, ready = () => window.vcwl } = {}) {
  const { server, url } = await serve(dist, base, api);
  const browser = await puppeteer.launch({ executablePath: chrome, headless: true, args: ["--no-sandbox"] });
  const page = await browser.newPage();
  await page.setViewport(viewport);
  const errors = [];
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warn") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(url + hash);
  await page.waitForFunction(ready, { timeout: 10000 });
  return {
    page,
    errors,
    url,
    close: async () => {
      await browser.close();
      server.close();
    },
  };
}

export const text = (page, sel) => page.$eval(sel, (e) => e.textContent);
export const count = (page, sel) => page.$$eval(sel, (es) => es.length);
