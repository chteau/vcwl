import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, tree, click, waitFor, repo } from "./harness.mjs";

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/closures")));
});
after(async () => app?.close());


test("closures see the component state they capture, and follow its changes", async () => {
  const { page } = app;
  let t = await tree(page);
  assert.deepEqual(t.all("li").map((l) => t.text(l)), ["#2", "#3", "#4"]);
  assert.equal(t.text("p"), "star 2");
  await click(page, "button:text('More')");
  await waitFor(page, (t) => t.count("li") === 2);
  t = await tree(page);
  assert.deepEqual(t.all("li").map((l) => t.text(l)), ["n3", "n4"]);
  assert.equal(t.text("p"), "star 3");
});
