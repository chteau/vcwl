import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";
import puppeteer from "puppeteer-core";
import { repo } from "./harness.mjs";

const APP = 'app "Dev"\n\ncomponent App {\n    paragraph("one")\n}\n';

test("vcwl dev reloads on change and shows compile errors", async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "vcwl-dev-"));
  fs.mkdirSync(path.join(dir, "src"));
  fs.writeFileSync(path.join(dir, "vcwl.toml"), 'entry = "src/main.vcwl"\n');
  const main = path.join(dir, "src/main.vcwl");
  fs.writeFileSync(main, APP);
  const server = spawn(path.join(repo, "target/debug/vcwl"), ["dev", "--port", "0"], { cwd: dir, stdio: ["ignore", "pipe", "ignore"] });
  const browser = await puppeteer.launch({ executablePath: process.env.CHROME ?? "/usr/bin/google-chrome", headless: true });
  t.after(async () => {
    await browser.close();
    server.kill();
    fs.rmSync(dir, { recursive: true, force: true });
  });
  const url = await new Promise((resolve) => {
    createInterface({ input: server.stdout }).on("line", (l) => {
      const m = l.match(/http:\/\/\S+/);
      if (m) resolve(m[0]);
    });
  });
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(url);
  await page.waitForFunction(() => document.querySelector("p")?.textContent === "one");

  fs.writeFileSync(main, APP.replace('"one"', "1 +"));
  await page.waitForSelector("[role=alert]", { timeout: 5000 });
  assert.match(await page.$eval("[role=alert]", (e) => e.textContent), /compile error:[\s\S]*main\.vcwl:4/);

  fs.writeFileSync(main, APP.replace('"one"', '"two"'));
  await page.waitForFunction(() => document.querySelector("p")?.textContent === "two", { timeout: 5000 });
  assert.equal(await page.$("[role=alert]"), null);
  assert.deepEqual(errors, []);
});
