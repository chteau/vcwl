// Runs the extension against the real `vcwl lsp` with a stand-in for the `vscode` module:
// `node --test editors/vscode/test.mjs` (after `cargo build`).
import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import Module from "node:module";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import path from "node:path";

const here = path.dirname(new URL(import.meta.url).pathname);
const repo = path.resolve(here, "../..");

test("the grammar's patterns are valid regular expressions", () => {
  const g = JSON.parse(readFileSync(path.join(here, "syntaxes/vcwl.tmLanguage.json"), "utf8"));
  const walk = (o) => {
    if (Array.isArray(o)) return o.forEach(walk);
    if (o && typeof o === "object")
      for (const [k, v] of Object.entries(o)) {
        if (["match", "begin", "end"].includes(k)) new RegExp(v);
        else walk(v);
      }
  };
  walk(g);
});

test("diagnostics, formatting and completion come from vcwl lsp", async () => {
  const dir = path.join(repo, "target/tmp/vscode-ext");
  mkdirSync(dir, { recursive: true });
  const file = path.join(dir, "prog.vcwl");
  writeFileSync(file, "");
  const uri = `file://${file}`;
  const providers = {}, listeners = {}, sets = [];
  class Range { constructor(a, b, c, d) { Object.assign(this, { a, b, c, d }); } }
  const vscode = {
    Range,
    Diagnostic: class { constructor(range, message, severity) { Object.assign(this, { range, message, severity }); } },
    DiagnosticSeverity: { Error: 0, Warning: 1 },
    TextEdit: { replace: (range, newText) => ({ range, newText }) },
    CompletionItem: class { constructor(label, kind) { Object.assign(this, { label, kind }); } },
    CompletionItemKind: { Keyword: 1, Function: 2, Property: 3 },
    Uri: { parse: (s) => ({ toString: () => s }) },
    window: { showErrorMessage: (m) => assert.fail(m) },
    workspace: {
      getConfiguration: () => ({ get: () => path.join(repo, "target/debug/vcwl") }),
      textDocuments: [],
      onDidOpenTextDocument: (f) => (listeners.open = f),
      onDidChangeTextDocument: (f) => (listeners.change = f),
      onDidSaveTextDocument: () => {},
      onDidCloseTextDocument: () => {},
    },
    languages: {
      createDiagnosticCollection: () => ({ set: (u, d) => sets.push([u.toString(), d]), delete: () => {} }),
      registerDocumentFormattingEditProvider: (_, p) => (providers.format = p),
      registerCompletionItemProvider: (_, p) => (providers.complete = p),
    },
  };
  const load = Module._load;
  Module._load = (req, ...rest) => (req === "vscode" ? vscode : load(req, ...rest));
  const ext = createRequire(import.meta.url)("./extension.js");
  Module._load = load;
  ext.activate({ subscriptions: [] });

  let text = "fn helper() -> Int {\n  1\n}\n\nfn main() {\n}\n";
  const doc = { languageId: "vcwl", uri: { toString: () => uri }, version: 1, getText: () => text };
  listeners.open(doc);
  const waitFor = async (pred) => {
    for (let i = 0; i < 100; i++) {
      const hit = sets.find(pred);
      if (hit) return hit;
      await new Promise((r) => setTimeout(r, 30));
    }
    assert.fail(JSON.stringify(sets));
  };
  const [, warn] = await waitFor(([u, d]) => u === uri && d.some((x) => /helper/.test(x.message)));
  assert.equal(warn[0].severity, 1);
  text = "fn main() {\n    let x: Int = \"no\"\n}\n";
  doc.version = 2;
  listeners.change({ document: doc });
  await waitFor(([u, d]) => u === uri && d.some((x) => x.severity === 0));
  text = "fn main() {\nprint(\"hi\")\n}\n";
  listeners.change({ document: doc });
  const edits = await providers.format.provideDocumentFormattingEdits(doc);
  assert.match(edits[0].newText, /^    print/m);
  const items = await providers.complete.provideCompletionItems(doc, { line: 0, character: 0 });
  assert.ok(items.some((i) => i.label === "component") && items.some((i) => i.label === "radius"));
  ext.deactivate();
});
