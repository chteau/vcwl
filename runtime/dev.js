// Development builds only: reloads the page when `vcwl dev` rebuilds the app.
let version = null;
async function poll() {
  try {
    const r = await fetch("/__vcwl/version", { cache: "no-store" });
    const v = await r.text();
    if (version !== null && v !== version) location.reload();
    version = v;
  } catch {
    // The dev server may be restarting.
  }
  setTimeout(poll, 500);
}
poll();
