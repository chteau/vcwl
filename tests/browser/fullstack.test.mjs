import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { repo, launch, ready, tree, click, waitFor } from "./harness.mjs";

const project = path.join(repo, "tests/browser/fixtures/fullstack");
let server, url, browser, page;
const errors = [], log = [];

before(async () => {
  fs.rmSync(path.join(project, "data"), { recursive: true, force: true });
  server = spawn(path.join(repo, "target/debug/vcwl"), ["serve", "--port", "0"], { cwd: project, env: { ...process.env, GUESTBOOK_SECRET: "s3cret" } });
  url = await new Promise((resolve, reject) => {
    let out = "";
    server.stdout.on("data", (d) => {
      out += d;
      log.push(String(d));
      const m = out.match(/at (http:\/\/\S+\/)/);
      if (m) resolve(m[1]);
    });
    server.stderr.on("data", (d) => log.push(String(d)));
    server.on("exit", (code) => reject(new Error(`vcwl serve exited (${code}): ${out}`)));
  });
  browser = await launch();
  page = await browser.newPage();
  await page.setViewport({ width: 900, height: 700 });
  page.on("console", (m) => (m.type() === "error" || m.type() === "warning") && errors.push(m.text()));
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(url);
  await ready(page);
});
after(async () => {
  await browser?.close();
  server?.kill();
});

const sign = async (message) => {
  await click(page, "input");
  await page.keyboard.type(message);
  await click(page, 'button:text("Sign")');
};

test("the page loads its data from a server function", async () => {
  await waitFor(page, (t) => t.text("p") === "ready");
  assert.equal((await tree(page)).count("li"), 0);
});

test("server functions change the data, and it persists", async () => {
  await sign("first!");
  await waitFor(page, (t) => t.text("p") === "signed");
  await sign("second");
  await waitFor(page, (t) => t.count("li") === 2);
  assert.ok(log.join("").includes("signing as Ada (s3cret)"), "env() reads the server's environment");
  await page.reload();
  await ready(page);
  await waitFor(page, (t) => t.text("p") === "ready" && t.count("li") === 2);
  const t = await tree(page);
  assert.deepEqual(t.all("li").map((li) => t.text(li)), ["Ada: first!", "Ada: second"]);
  const store = JSON.parse(fs.readFileSync(path.join(project, "data/store.json"), "utf8"));
  assert.deepEqual(store.entries.map((e) => e.text), ["first!", "second"]);
});

test("a failing server function reaches the client as an HTTP error", async () => {
  await click(page, 'button:text("Sign")');
  await waitFor(page, (t) => /500/.test(t.text("p")));
  assert.ok(log.join("").includes("panic: empty message"), log.join(""));
});

test("clearing goes through the server too", async () => {
  await click(page, 'button:text("Clear")');
  await waitFor(page, (t) => t.text("p") === "cleared" && t.count("li") === 0);
  await page.reload();
  await ready(page);
  await waitFor(page, (t) => t.text("p") === "ready" && t.count("li") === 0);
});

test("no console errors besides the failed call", () => assert.deepEqual(errors.filter((e) => !e.includes("500")), []));
