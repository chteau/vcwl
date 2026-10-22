import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "site")), { viewport: { width: 1200, height: 5200 } });
});
after(async () => app?.close());

test("the project site renders, satire and disclaimer included", async () => {
  const t = await tree(app.page);
  assert.ok(t.nodes.some((n) => n.text === "VCWL is going to revolutionize web development."));
  assert.ok(t.nodes.some((n) => n.text?.startsWith("// satire.")));
  assert.equal(t.all("article").length, 15);
});

test("the terminal types its lines one by one", async () => {
  await waitFor(app.page, (t) => t.nodes.some((n) => n.text === "  * it has not"), 10000);
});

test("the hot take generator draws a take and a score through @web", async () => {
  await click(app.page, "button:text('Generate hot take')");
  await waitFor(app.page, (t) => t.nodes.some((n) => /^\d+\/100 · (?!unmeasured)/.test(n.text ?? "")));
});
