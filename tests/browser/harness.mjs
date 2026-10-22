// Shared browser-test setup: build an example, serve it, open it in headless Chrome, and
// query the renderer's element tree (the page itself is a single canvas).
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer-core";
import { serve } from "./serve.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
export const repo = path.resolve(here, "../..");
const vcwl = path.join(repo, "target/debug/vcwl");
export const chrome = process.env.CHROME ?? "/usr/bin/google-chrome";

export function build(projectDir, args = []) {
  execFileSync(vcwl, ["build", ...args], { cwd: projectDir, stdio: "pipe" });
  return path.join(projectDir, "dist");
}

export const launch = () => puppeteer.launch({ executablePath: chrome, headless: true, args: ["--no-sandbox"] });

export async function open(dist, { base = "/", hash = "", viewport = { width: 1000, height: 800 }, api = null, ready: until = null } = {}) {
  const { server, url } = await serve(dist, base, api);
  const browser = await launch();
  const page = await browser.newPage();
  await page.setViewport(viewport);
  const errors = [];
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warn") errors.push(m.text());
  });
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto(url + hash);
  await (until ? page.waitForFunction(until, { timeout: 10000 }) : ready(page));
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

export async function ready(page) {
  await page.waitForFunction(() => window.vcwl?.tree, { timeout: 10000 });
  // The accessibility mirror follows the first frame.
  await page.evaluate(() => window.vcwl.mirrored());
}

/** A snapshot of the renderer's element tree. */
export class Tree {
  constructor(nodes) {
    this.nodes = nodes;
    this.byId = new Map(nodes.map((n) => [n.id, n]));
    for (const n of nodes) n.children = [];
    for (const n of nodes) this.byId.get(n.parent)?.children.push(n);
  }

  /** Concatenated text of a node and its descendants. */
  text(sel) {
    const n = typeof sel === "string" ? this.one(sel) : sel;
    return n.text ?? n.children.map((c) => this.text(c)).join("");
  }

  one(sel) {
    const [n] = this.all(sel);
    if (!n) throw new Error(`no element matches ${sel}`);
    return n;
  }

  count(sel) {
    return this.all(sel).length;
  }

  /** Elements matching a small CSS subset: tags, `.class`, `[attr]`, `[attr=value]`,
   *  `:nth-child(n)`, `:checked`, `:focus`, `:text(value)`, descendant and `>` combinators. */
  all(sel) {
    const steps = sel.match(/>|(?:[^\s>'"\[(]|\[[^\]]*\]|\([^)]*\))+/g) ?? [];
    let rel = null;
    const out = [];
    const parts = [];
    for (const s of steps) {
      if (s === ">") rel = ">";
      else {
        parts.push([rel, compile(s)]);
        rel = null;
      }
    }
    const matches = (n, i) => {
      const [, test] = parts[i];
      if (!n.tag || !test(n, this)) return false;
      if (i === 0) return true;
      const [rel2] = parts[i];
      if (rel2 === ">") return this.byId.has(n.parent) && matches(this.byId.get(n.parent), i - 1);
      for (let p = this.byId.get(n.parent); p; p = this.byId.get(p.parent)) if (matches(p, i - 1)) return true;
      return false;
    };
    for (const n of this.nodes) if (matches(n, parts.length - 1)) out.push(n);
    return out;
  }
}

function compile(compound) {
  const tests = [];
  const re = /^[a-z0-9]+|\.[\w-]+|\[([\w-]+)(?:=(['"]?)(.*?)\2)?\]|:nth-child\((\d+)\)|:checked|:focus|:text\((['"])(.*?)\5\)/gy;
  let m;
  while (re.lastIndex < compound.length && (m = re.exec(compound))) {
    const t = m[0];
    if (/^[a-z0-9]/.test(t)) tests.push((n) => n.tag === t);
    else if (t[0] === ".") tests.push((n) => n.classes.includes(t.slice(1)));
    else if (t[0] === "[") {
      const [k, v] = [m[1], m[3]];
      tests.push((n) => (v === undefined ? k in n.attrs : n.attrs[k] === v));
    } else if (t.startsWith(":nth-child")) {
      const i = Number(m[4]);
      tests.push((n, tree) => tree.byId.get(n.parent).children.filter((c) => c.tag).indexOf(n) === i - 1);
    } else if (t === ":checked") tests.push((n) => n.checked);
    else if (t === ":focus") tests.push((n) => n.focused);
    else tests.push((n, tree) => tree.text(n).trim() === m[6]);
  }
  if (re.lastIndex !== compound.length && m === null) throw new Error(`unsupported selector ${compound}`);
  return (n, tree) => tests.every((f) => f(n, tree));
}

export async function tree(page) {
  return new Tree(await page.evaluate(() => window.vcwl.tree()));
}

export async function text(page, sel) {
  return (await tree(page)).text(sel);
}

export async function count(page, sel) {
  return (await tree(page)).count(sel);
}

/** Clicks the center of the element matching `sel` with the mouse. */
export async function click(page, sel) {
  const n = (await tree(page)).one(sel);
  const [x, y, w, h] = n.rect;
  await page.mouse.click(x + w / 2, y + h / 2);
}

/** Polls the tree until `pred(tree)` is truthy. */
export async function waitFor(page, pred, timeout = 5000) {
  const end = Date.now() + timeout;
  for (;;) {
    const t = await tree(page);
    if (pred(t)) return t;
    if (Date.now() > end) throw new Error("timed out waiting for the page");
    await new Promise((r) => setTimeout(r, 25));
  }
}
