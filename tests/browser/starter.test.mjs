import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { build, open, tree, click, repo } from "./harness.mjs";

// `vcwl new` output must build and run unchanged, also under a GitHub Pages-style base path.
let dir, app;
before(async () => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "vcwl-starter-"));
  execFileSync(path.join(repo, "target/debug/vcwl"), ["new", "hello"], { cwd: dir, stdio: "pipe" });
  app = await open(build(path.join(dir, "hello")), { base: "/hello/" });
});
after(async () => {
  await app?.close();
  fs.rmSync(dir, { recursive: true, force: true });
});

test("counter increments on click", async () => {
  const { page } = app;
  const row = "div.v-row";
  let t = await tree(page);
  assert.equal(t.text("h1"), "Hello, hello!");
  assert.match(t.text(row), /^Clicks: 0/);
  await click(page, "button");
  await click(page, "button");
  t = await tree(page);
  assert.match(t.text(row), /^Clicks: 2/);
  assert.deepEqual(app.errors, []);
});
