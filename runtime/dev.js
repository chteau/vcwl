// Development builds only: reloads the page when `vcwl dev` rebuilds the app and shows
// compile errors from failed rebuilds.
import { showError } from "./vcwl.js";

let version = null;
let shown = "";
async function poll() {
  try {
    const r = await fetch("/__vcwl/status", { cache: "no-store" });
    const text = await r.text();
    const nl = text.indexOf("\n");
    const v = text.slice(0, nl), error = text.slice(nl + 1);
    if (version !== null && v !== version) location.reload();
    version = v;
    if (error !== shown) showError(document, error ? "compile error:\n" + error : null);
    shown = error;
  } catch {
    // The dev server may be restarting.
  }
  setTimeout(poll, 500);
}
poll();

// ----- dev tools -----
// A badge in the corner opens a panel (also Alt+Shift+D) showing the renderer's element tree, the
// box and computed style of the selected element, and frame timings. "Pick" selects the element
// under the pointer. The panel lives in a shadow root above the canvas; the app never sees it.

const CSS = `
:host{all:initial;font:12px/1.4 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace;color:#e6e6e6}
.badge{position:fixed;left:12px;bottom:12px;z-index:2147483647;display:flex;gap:6px;align-items:center;
  padding:6px 10px;border-radius:999px;border:1px solid #333;background:#111c;color:#e6e6e6;cursor:pointer;font:inherit;
  backdrop-filter:blur(6px)}
.badge b{color:#b6ff6c}
.panel{position:fixed;top:0;right:0;bottom:0;width:min(420px,100vw);z-index:2147483647;display:none;flex-direction:column;
  background:#111;border-left:1px solid #333;box-shadow:-8px 0 24px #0008}
.panel.open{display:flex}
header{display:flex;gap:4px;align-items:center;padding:6px;border-bottom:1px solid #333}
header button{font:inherit;color:inherit;background:#1d1d1d;border:1px solid #333;border-radius:4px;padding:3px 8px;cursor:pointer}
header button[aria-pressed=true]{background:#b6ff6c;color:#111;border-color:#b6ff6c}
header .grow{flex:1}
.tree{flex:1 1 55%;overflow:auto;padding:4px 0;border-bottom:1px solid #333}
.row{white-space:pre;padding:0 8px;cursor:default}
.row:hover{background:#1f2a1a}.row.sel{background:#2c3d22}
.row.off{opacity:.45}
.tw{display:inline-block;width:12px;color:#888;cursor:pointer}
.tag{color:#7cc4ff}.cls{color:#ffb86c}.attr{color:#c792ea}.txt{color:#aaa}
.details,.perf{flex:1 1 45%;overflow:auto;padding:8px}
.perf{display:none}.perf.on{display:block}.details.off,.tree.off{display:none}
table{border-collapse:collapse;width:100%}td{padding:1px 6px 1px 0;vertical-align:top}td:first-child{color:#888;white-space:nowrap}
h3{font:inherit;color:#b6ff6c;margin:8px 0 4px}
.hl{position:fixed;pointer-events:none;z-index:2147483646;background:#7cc4ff33;outline:1px solid #7cc4ff;display:none}
.hl span{position:absolute;left:0;top:-18px;background:#7cc4ff;color:#111;padding:0 4px;white-space:nowrap}
`;

function devtools(host) {
  const el = document.createElement("vcwl-devtools");
  const root = el.attachShadow({ mode: "open" });
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(CSS);
  root.adoptedStyleSheets = [sheet];
  const make = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  };
  const badge = make("button", "badge");
  badge.setAttribute("aria-label", "VCWL dev tools");
  const panel = make("div", "panel");
  const header = make("header");
  const tabElements = make("button", "", "Elements"), tabPerf = make("button", "", "Performance");
  const pick = make("button", "", "Pick"), close = make("button", "", "×");
  close.setAttribute("aria-label", "Close dev tools");
  header.append(tabElements, tabPerf, make("span", "grow"), pick, close);
  const treeEl = make("div", "tree"), details = make("div", "details"), perf = make("div", "perf");
  panel.append(header, treeEl, details, perf);
  const hl = make("div", "hl");
  const hlLabel = make("span");
  hl.append(hlLabel);
  root.append(badge, panel, hl);
  document.body.append(el);

  let open = false, tab = "elements", picking = false, selected = -1, nodes = [], byId = new Map();
  let lastCommands = -1, lastFrames = -1;
  const collapsed = new Set(), expanded = new Set();

  const highlight = (id) => {
    const n = byId.get(id);
    if (!n || !n.rect || (n.rect[2] === 0 && n.rect[3] === 0)) {
      hl.style.display = "none";
      return;
    }
    const [x, y, w, h] = n.rect;
    Object.assign(hl.style, { display: "block", left: x + "px", top: y + "px", width: w + "px", height: h + "px" });
    hlLabel.textContent = `${n.tag ?? "text"} ${Math.round(w)}×${Math.round(h)}`;
  };

  const label = (row, n) => {
    if (n.text !== undefined) {
      row.append(make("span", "txt", JSON.stringify(n.text.length > 60 ? n.text.slice(0, 60) + "…" : n.text)));
      return;
    }
    row.append(make("span", "tag", n.id === 0 ? "#root" : n.tag));
    for (const c of n.classes ?? []) row.append(make("span", "cls", "." + c));
    for (const [k, v] of Object.entries(n.attrs ?? {})) row.append(make("span", "attr", ` ${k}="${v.length > 30 ? v.slice(0, 30) + "…" : v}"`));
  };

  const isOpen = (n, depth) => !collapsed.has(n.id) && (depth < 4 || expanded.has(n.id));

  function renderTree() {
    const frag = document.createDocumentFragment();
    const walk = (n, depth) => {
      if (n.comment) return;
      const row = make("div", "row");
      row.dataset.id = n.id;
      if (n.id === selected) row.classList.add("sel");
      if (n.shown === false && n.id !== 0) row.classList.add("off");
      row.style.paddingLeft = 8 + depth * 12 + "px";
      const kids = n.children.filter((c) => !c.comment);
      const tw = make("span", "tw", kids.length ? (isOpen(n, depth) ? "▾" : "▸") : "");
      row.append(tw);
      label(row, n);
      frag.append(row);
      if (kids.length && isOpen(n, depth)) for (const c of kids) walk(c, depth + 1);
    };
    if (byId.has(0)) walk(byId.get(0), 0);
    treeEl.replaceChildren(frag);
  }

  function renderDetails() {
    details.replaceChildren();
    const n = byId.get(selected);
    if (!n) {
      details.append(make("div", "txt", "Select an element in the tree, or Pick one on the page."));
      return;
    }
    const table = (rows) => {
      const t = make("table");
      for (const [k, v] of rows) {
        const tr = make("tr");
        tr.append(make("td", "", k), make("td", "", String(v)));
        t.append(tr);
      }
      return t;
    };
    if (n.text !== undefined) {
      details.append(make("h3", "", "text"), table([["content", n.text], ["box", (n.rect ?? []).map(Math.round).join(", ")]]));
      return;
    }
    const info = host.inspect(selected);
    if (!info) return;
    const [x, y, w, h] = info.box.map((v) => Math.round(v * 10) / 10);
    details.append(
      make("h3", "", `${info.tag} #${info.id}`),
      table([["box", `${x}, ${y}  ${w} × ${h}`], ["padding", info.padding.join(" ")], ["border", info.border.join(" ")], ["classes", (n.classes ?? []).join(" ") || "—"]]),
    );
    const attrs = Object.entries(n.attrs ?? {});
    if (attrs.length) details.append(make("h3", "", "attributes"), table(attrs));
    details.append(make("h3", "", "computed style"), table(Object.entries(info.style)));
  }

  let lastStats = null;
  function renderPerf() {
    const s = host.stats, now = performance.now();
    const prev = lastStats ?? { ...s, t: now - 1000 };
    const frames = s.frames - prev.frames || 1, secs = (now - prev.t) / 1000;
    const per = (k) => ((s[k] - prev[k]) / frames).toFixed(2) + " ms";
    lastStats = { ...s, t: now };
    perf.replaceChildren(
      make("h3", "", "last " + secs.toFixed(1) + " s"),
      (() => {
        const t = make("table");
        for (const [k, v] of [
          ["frames / s", ((s.frames - prev.frames) / secs).toFixed(1)],
          ["render / frame", per("renderMs")],
          ["draw / frame", per("drawMs")],
          ["mirror / frame", per("mirrorMs")],
          ["app flushes", s.flushes - prev.flushes],
          ["command bytes", s.bytes - prev.bytes],
          ["elements", host.liveNodes()],
          ["listeners", host.liveListeners()],
        ]) {
          const tr = make("tr");
          tr.append(make("td", "", k), make("td", "", String(v)));
          t.append(tr);
        }
        return t;
      })(),
    );
  }

  function refresh(force = false) {
    const commands = host.commands(), frames = host.stats.frames;
    if (force || commands !== lastCommands || frames !== lastFrames) {
      nodes = host.tree();
      byId = new Map(nodes.map((n) => [n.id, { ...n, children: [] }]));
      for (const n of byId.values()) byId.get(n.parent)?.children.push(n);
      if (force || commands !== lastCommands) renderTree();
      if (tab === "elements") renderDetails();
      lastCommands = commands;
      lastFrames = frames;
    }
  }

  function select(id) {
    selected = id;
    for (let p = byId.get(id)?.parent; p !== undefined && p >= 0; p = byId.get(p)?.parent) {
      collapsed.delete(p);
      expanded.add(p);
    }
    renderTree();
    renderDetails();
    highlight(id);
    treeEl.querySelector(".sel")?.scrollIntoView({ block: "nearest" });
  }

  function show(on) {
    open = on;
    panel.classList.toggle("open", on);
    if (!on) {
      setPicking(false);
      hl.style.display = "none";
    } else refresh(true);
    try {
      sessionStorage.setItem("vcwl-devtools", on ? "1" : "");
    } catch {}
  }

  function setTab(t) {
    tab = t;
    tabElements.setAttribute("aria-pressed", t === "elements");
    tabPerf.setAttribute("aria-pressed", t === "perf");
    treeEl.classList.toggle("off", t !== "elements");
    details.classList.toggle("off", t !== "elements");
    perf.classList.toggle("on", t === "perf");
    if (t === "perf") renderPerf();
  }

  function setPicking(on) {
    picking = on;
    pick.setAttribute("aria-pressed", on);
    if (!on && selected < 0) hl.style.display = "none";
  }

  // While picking, the page's pointer events go to the dev tools only.
  const pickEvent = (e) => {
    if (!picking || e.composedPath().includes(el)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const id = host.nodeAt(e.clientX, e.clientY);
    if (e.type === "pointermove") {
      refresh();
      highlight(id);
    } else if (e.type === "pointerdown" && id >= 0) {
      refresh(true);
      select(id);
      setPicking(false);
    }
  };
  for (const t of ["pointermove", "pointerdown", "pointerup", "click", "mousedown", "mouseup"]) window.addEventListener(t, pickEvent, true);

  badge.addEventListener("click", () => show(!open));
  close.addEventListener("click", () => show(false));
  tabElements.addEventListener("click", () => setTab("elements"));
  tabPerf.addEventListener("click", () => setTab("perf"));
  pick.addEventListener("click", () => setPicking(!picking));
  treeEl.addEventListener("click", (e) => {
    const row = e.target.closest(".row");
    if (!row) return;
    const id = Number(row.dataset.id);
    if (e.target.classList.contains("tw")) {
      const depth = Math.round((parseFloat(row.style.paddingLeft) - 8) / 12);
      if (isOpen(byId.get(id), depth)) {
        collapsed.add(id);
        expanded.delete(id);
      } else {
        collapsed.delete(id);
        expanded.add(id);
      }
      renderTree();
    } else select(id);
  });
  treeEl.addEventListener("pointerover", (e) => {
    const row = e.target.closest(".row");
    if (row) highlight(Number(row.dataset.id));
  });
  treeEl.addEventListener("pointerleave", () => (selected >= 0 ? highlight(selected) : (hl.style.display = "none")));
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.altKey && e.shiftKey && e.code === "KeyD") {
        e.preventDefault();
        e.stopImmediatePropagation();
        show(!open);
      } else if (e.key === "Escape" && picking) setPicking(false);
    },
    true,
  );

  setTab("elements");
  // The badge shows the average frame cost (render + draw) of the last second.
  let prev = { ...host.stats };
  setInterval(() => {
    const s = host.stats, frames = s.frames - prev.frames;
    const ms = frames ? (s.renderMs + s.drawMs - prev.renderMs - prev.drawMs) / frames : 0;
    badge.replaceChildren(make("b", "", "VCWL"), make("span", "", frames ? `${ms.toFixed(1)} ms` : "idle"));
    prev = { ...s };
    if (open) {
      refresh();
      if (tab === "perf") renderPerf();
    }
  }, 500);
  badge.replaceChildren(make("b", "", "VCWL"), make("span", "", "dev"));
  try {
    if (sessionStorage.getItem("vcwl-devtools")) show(true);
  } catch {}
  return { show, select, setTab, setPicking, refresh };
}

(function waitForApp() {
  if (window.vcwl?.tree) window.vcwlDevtools = devtools(window.vcwl);
  else setTimeout(waitForApp, 100);
})();
