// Development builds only: reloads the page when `vcwl dev` rebuilds the app and shows
// compile errors from failed rebuilds.
import { showError } from "./vcwl.js";

let version = null;
let shown = null;
async function poll() {
  try {
    const r = await fetch("/__vcwl/status", { cache: "no-store" });
    const text = await r.text();
    const nl = text.indexOf("\n");
    const v = text.slice(0, nl), error = text.slice(nl + 1);
    if (version !== null && v !== version) location.reload();
    version = v;
    shown?.remove();
    shown = error ? showError(document, "compile error:\n" + error) : null;
  } catch {
    // The dev server may be restarting.
  }
  setTimeout(poll, 500);
}
poll();
