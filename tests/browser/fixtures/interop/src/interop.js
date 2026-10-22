// The app's own JavaScript: every export can be declared as an `extern fn`.
export function add_big(a, b) {
  return a + b;
}

export function point(x) {
  return { x, label: `p${x}` };
}

export async function slow_upper(s) {
  await new Promise((r) => setTimeout(r, 20));
  return s.toUpperCase();
}

export async function always_fails() {
  throw new Error("nope");
}
