import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import { build, open, text, count, repo } from "./harness.mjs";

const ITEMS = [
  { id: 1, name: "Dune", tags: ["sf", "classic"], note: null, kind: "Book", extra: true },
  { id: 2, name: "<img src=x onerror=alert(1)>", tags: [], kind: "Film" },
];
const requests = [];
let releaseSlow = null;

function api(req, res, rel) {
  if (!rel.startsWith("api/")) return false;
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    requests.push({ method: req.method, rel, body, type: req.headers["content-type"] ?? null });
    const json = (status, value) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(value));
    if (rel === "api/items" && req.method === "GET") json(200, ITEMS);
    else if (rel === "api/items" && req.method === "POST") json(201, { id: 3, tags: [], note: "new", ...JSON.parse(body) });
    else if (rel === "api/items/1" && req.method === "DELETE") res.writeHead(204).end();
    else if (rel === "api/bad") json(200, [ITEMS[0], { ...ITEMS[1], tags: ["ok", 7] }]);
    else if (rel === "api/invalid") res.writeHead(200).end("{not json");
    else if (rel === "api/slow") releaseSlow = () => json(200, ITEMS);
    else res.writeHead(404, { "content-type": "text/plain" }).end("no such thing");
  });
  return true;
}

let app;
before(async () => {
  app = await open(build(path.join(repo, "tests/browser/fixtures/fetch")), { api });
});
after(async () => app?.close());

async function click(label) {
  const [btn] = await app.page.$$(`xpath/.//button[text()=${JSON.stringify(label)}]`);
  await btn.click();
}
const status = () => text(app.page, "p");
async function waitStatus(re) {
  await app.page.waitForFunction((src) => new RegExp(src).test(document.querySelector("p").textContent), { timeout: 5000 }, re.source);
  return status();
}

test("GET decodes typed JSON into state", async () => {
  await click("Load");
  assert.equal(await waitStatus(/^loaded/), "loaded 2");
  assert.deepEqual(await app.page.$$eval("li", (ls) => ls.map((l) => l.textContent)), ["1: Dune [sf, classic] -", "2: <img src=x onerror=alert(1)> [] -"]);
  // Response data is inserted as text, never parsed as HTML.
  assert.equal(await count(app.page, "img"), 0);
});

test("POST sends the encoded body", async () => {
  await click("Create");
  assert.equal(await waitStatus(/^created/), "created 3 Dune");
  const req = requests.find((r) => r.method === "POST");
  assert.equal(req.body, '{"name":"Dune","kind":"Book"}');
  assert.equal(req.type, "application/json");
});

test("DELETE with an empty response decodes as ()", async () => {
  await click("Delete");
  assert.equal(await waitStatus(/^deleted|error|unexpected/), "deleted");
});

test("responses are validated: shape errors name the path", async () => {
  await click("Bad shape");
  assert.equal(await waitStatus(/^decode/), "decode: $[1].tags[1]: expected a string, found a number");
  assert.equal(await count(app.page, "li"), 2, "state is unchanged on error");
});

test("HTTP errors and invalid JSON are reported", async () => {
  await click("Missing");
  assert.equal(await waitStatus(/^status/), "status 404: no such thing");
  await click("Invalid");
  assert.match(await waitStatus(/^unexpected response/), /^unexpected response: invalid JSON/);
});

test("hosts outside the network capability are refused by the host", async () => {
  const before = requests.length;
  await click("Blocked");
  assert.equal(await waitStatus(/^network/), "network error: https://blocked.example is not allowed by the app's network capability");
  assert.equal(requests.length, before);
});

test("a response for a destroyed component is discarded", async () => {
  await click("Load api/slow");
  while (!releaseSlow) await new Promise((r) => setTimeout(r, 20));
  const slots = await app.page.evaluate(() => window.vcwl.exports.live_slots());
  await click("Toggle loader");
  assert.equal(await count(app.page, "button"), 8);
  releaseSlow();
  await new Promise((r) => setTimeout(r, 200));
  assert.doesNotMatch(await status(), /^child/);
  // The request's slot is released when its response arrives.
  const after = await app.page.evaluate(() => window.vcwl.exports.live_slots());
  assert.equal(after, slots - 2, "the loader's click listener and the request slot are released");
});

test("no console errors", () => {
  assert.deepEqual(app.errors.filter((e) => !/404|Failed to load resource/.test(e)), []);
});
