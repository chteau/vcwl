// VCWL for VS Code: a minimal language client for `vcwl lsp` (no dependencies). It sends the
// open .vcwl documents to the server and shows its diagnostics, completions and formatting.
const vscode = require("vscode");
const { spawn } = require("child_process");

let server = null;

/** JSON-RPC over the server's stdio with Content-Length framing. */
function connect(command, onNotification) {
  const child = spawn(command, ["lsp"], { stdio: ["pipe", "pipe", "inherit"] });
  const pending = new Map();
  let nextId = 1;
  let buf = Buffer.alloc(0);
  child.stdout.on("data", (chunk) => {
    buf = Buffer.concat([buf, chunk]);
    for (;;) {
      const end = buf.indexOf("\r\n\r\n");
      if (end < 0) return;
      const len = Number(/Content-Length: *(\d+)/i.exec(buf.subarray(0, end).toString())?.[1]);
      if (buf.length < end + 4 + len) return;
      const msg = JSON.parse(buf.subarray(end + 4, end + 4 + len).toString());
      buf = buf.subarray(end + 4 + len);
      if (msg.id !== undefined && pending.has(msg.id)) {
        pending.get(msg.id)(msg.result ?? null);
        pending.delete(msg.id);
      } else if (msg.method) onNotification(msg.method, msg.params);
    }
  });
  const write = (msg) => {
    const body = Buffer.from(JSON.stringify({ jsonrpc: "2.0", ...msg }));
    child.stdin.write(`Content-Length: ${body.length}\r\n\r\n`);
    child.stdin.write(body);
  };
  return {
    child,
    notify: (method, params) => write({ method, params }),
    request: (method, params) =>
      new Promise((resolve) => {
        const id = nextId++;
        pending.set(id, resolve);
        write({ id, method, params });
      }),
  };
}

const toRange = (r) => new vscode.Range(r.start.line, r.start.character, r.end.line, r.end.character);

function activate(context) {
  const command = vscode.workspace.getConfiguration("vcwl").get("path") || "vcwl";
  const diagnostics = vscode.languages.createDiagnosticCollection("vcwl");
  context.subscriptions.push(diagnostics);
  const rpc = connect(command, (method, params) => {
    if (method !== "textDocument/publishDiagnostics") return;
    diagnostics.set(
      vscode.Uri.parse(params.uri),
      params.diagnostics.map((d) => {
        const diag = new vscode.Diagnostic(toRange(d.range), d.message, d.severity === 1 ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning);
        diag.source = d.source;
        if (d.code) diag.code = d.code;
        return diag;
      }),
    );
  });
  server = rpc;
  rpc.child.on("error", (e) => vscode.window.showErrorMessage(`VCWL: could not start \`${command} lsp\`: ${e.message}`));
  const ready = rpc.request("initialize", { processId: process.pid, rootUri: null, capabilities: {} }).then(() => rpc.notify("initialized", {}));

  const isVcwl = (doc) => doc.languageId === "vcwl";
  const open = (doc) => isVcwl(doc) && ready.then(() => rpc.notify("textDocument/didOpen", { textDocument: { uri: doc.uri.toString(), languageId: "vcwl", version: doc.version, text: doc.getText() } }));
  vscode.workspace.textDocuments.forEach(open);
  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(open),
    vscode.workspace.onDidChangeTextDocument((e) => {
      if (isVcwl(e.document)) rpc.notify("textDocument/didChange", { textDocument: { uri: e.document.uri.toString(), version: e.document.version }, contentChanges: [{ text: e.document.getText() }] });
    }),
    vscode.workspace.onDidSaveTextDocument((doc) => isVcwl(doc) && rpc.notify("textDocument/didSave", { textDocument: { uri: doc.uri.toString() } })),
    vscode.workspace.onDidCloseTextDocument((doc) => {
      if (!isVcwl(doc)) return;
      rpc.notify("textDocument/didClose", { textDocument: { uri: doc.uri.toString() } });
      diagnostics.delete(doc.uri);
    }),
    vscode.languages.registerDocumentFormattingEditProvider("vcwl", {
      async provideDocumentFormattingEdits(doc) {
        const edits = (await rpc.request("textDocument/formatting", { textDocument: { uri: doc.uri.toString() }, options: {} })) ?? [];
        return edits.map((e) => vscode.TextEdit.replace(toRange(e.range), e.newText));
      },
    }),
    vscode.languages.registerCompletionItemProvider("vcwl", {
      async provideCompletionItems(doc, position) {
        const items = (await rpc.request("textDocument/completion", { textDocument: { uri: doc.uri.toString() }, position })) ?? [];
        const kinds = { 14: vscode.CompletionItemKind.Keyword, 3: vscode.CompletionItemKind.Function, 10: vscode.CompletionItemKind.Property };
        return items.map((i) => new vscode.CompletionItem(i.label, kinds[i.kind]));
      },
    }),
  );
}

function deactivate() {
  if (!server) return;
  server.request("shutdown").then(() => {
    server.notify("exit");
    server = null;
  });
}

module.exports = { activate, deactivate };
