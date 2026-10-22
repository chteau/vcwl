// 3D scenes (the `scene` element), loaded by the host the first time a page paints one.
// A scene is drawn with WebGL into one offscreen canvas, which the host then copies into the
// page's canvas at the scene's place: the scene stays clipped and layered like any element.

const VS = `
attribute vec3 position;
attribute vec3 normal;
uniform mat4 model, viewProj;
uniform mat3 normalMat;
varying vec3 vNormal, vWorld;
void main() {
  vec4 w = model * vec4(position, 1.0);
  vWorld = w.xyz;
  vNormal = normalMat * normal;
  gl_Position = viewProj * w;
}`;

const FS = `
precision mediump float;
uniform vec3 color, ambient, eye, pickColor;
uniform float shininess;
uniform bool pick;
uniform int lightCount;
uniform vec4 lightPos[8];
uniform vec3 lightColor[8];
varying vec3 vNormal, vWorld;
void main() {
  if (pick) { gl_FragColor = vec4(pickColor, 1.0); return; }
  vec3 v = normalize(eye - vWorld);
  // Two-sided: light the side facing the camera, whatever the triangle winding.
  vec3 n = normalize(vNormal);
  if (dot(n, v) < 0.0) n = -n;
  vec3 c = ambient * color;
  for (int i = 0; i < 8; i++) {
    if (i >= lightCount) break;
    vec3 l = lightPos[i].w > 0.5 ? normalize(lightPos[i].xyz - vWorld) : normalize(lightPos[i].xyz);
    float d = max(dot(n, l), 0.0);
    float s = d > 0.0 && shininess > 0.0 ? pow(max(dot(n, normalize(l + v)), 0.0), shininess) * 0.5 : 0.0;
    c += lightColor[i] * (color * d + vec3(s));
  }
  gl_FragColor = vec4(c, 1.0);
}`;

// ----- matrices (column-major, as WebGL expects) -----

const mul = (a, b) => {
  const o = new Float32Array(16);
  for (let c = 0; c < 4; c++)
    for (let r = 0; r < 4; r++) {
      let s = 0;
      for (let k = 0; k < 4; k++) s += a[k * 4 + r] * b[c * 4 + k];
      o[c * 4 + r] = s;
    }
  return o;
};
const ident = () => new Float32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);

function perspective(fovDeg, aspect, near, far) {
  const f = 1 / Math.tan((fovDeg * Math.PI) / 360), nf = 1 / (near - far);
  return new Float32Array([f / aspect, 0, 0, 0, 0, f, 0, 0, 0, 0, (far + near) * nf, -1, 0, 0, 2 * far * near * nf, 0]);
}

const sub = (a, b) => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const cross = (a, b) => [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
const norm = (a) => {
  const l = Math.hypot(...a) || 1;
  return [a[0] / l, a[1] / l, a[2] / l];
};
const dot = (a, b) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

function lookAt(eye, target) {
  const z = norm(sub(eye, target));
  const x = norm(cross(Math.abs(z[1]) > 0.999 ? [0, 0, 1] : [0, 1, 0], z));
  const y = cross(z, x);
  return new Float32Array([x[0], y[0], z[0], 0, x[1], y[1], z[1], 0, x[2], y[2], z[2], 0, -dot(x, eye), -dot(y, eye), -dot(z, eye), 1]);
}

/** Translation × rotation (degrees, X then Y then Z as in `Rx·Ry·Rz`) × scale. */
function transform(p, rDeg, s) {
  const [a, b, c] = rDeg.map((d) => (d * Math.PI) / 180);
  const [ca, sa, cb, sb, cc, sc] = [Math.cos(a), Math.sin(a), Math.cos(b), Math.sin(b), Math.cos(c), Math.sin(c)];
  // Rx·Ry·Rz, row by row.
  const r = [
    [cb * cc, -cb * sc, sb],
    [sa * sb * cc + ca * sc, -sa * sb * sc + ca * cc, -sa * cb],
    [-ca * sb * cc + sa * sc, ca * sb * sc + sa * cc, ca * cb],
  ];
  const m = ident();
  for (let col = 0; col < 3; col++) for (let row = 0; row < 3; row++) m[col * 4 + row] = r[row][col] * s[col];
  m[12] = p[0];
  m[13] = p[1];
  m[14] = p[2];
  return m;
}

/** The inverse transpose of a model matrix's upper 3×3 (normals under non-uniform scale). */
function normalMatrix(m) {
  const [a, b, c, d, e, f, g, h, i] = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
  const A = e * i - f * h, B = -(d * i - f * g), C = d * h - e * g;
  const det = a * A + b * B + c * C || 1;
  // Inverse transpose = cofactor matrix / det, column-major.
  return new Float32Array([
    A / det, B / det, C / det,
    -(b * i - c * h) / det, (a * i - c * g) / det, -(a * h - b * g) / det,
    (b * f - c * e) / det, -(a * f - c * d) / det, (a * e - b * d) / det,
  ]);
}

// ----- geometry (unit size, centered on the origin) -----

function build(shape) {
  const pos = [], nrm = [], idx = [];
  const vert = (p, n) => {
    pos.push(...p);
    nrm.push(...n);
    return pos.length / 3 - 1;
  };
  const quad = (a, b, c, d) => idx.push(a, b, c, a, c, d);
  // A surface of revolution or a parametric patch: f(u, v) -> [position, normal].
  const grid = (nu, nv, f) => {
    const base = pos.length / 3;
    for (let j = 0; j <= nv; j++) for (let i = 0; i <= nu; i++) vert(...f(i / nu, j / nv));
    for (let j = 0; j < nv; j++)
      for (let i = 0; i < nu; i++) {
        const k = base + j * (nu + 1) + i;
        quad(k, k + nu + 1, k + nu + 2, k + 1);
      }
  };
  const disc = (y, ny, r, n) => {
    const c = vert([0, y, 0], [0, ny, 0]);
    const ring = [];
    for (let i = 0; i <= n; i++) {
      const t = (i / n) * Math.PI * 2;
      ring.push(vert([Math.cos(t) * r, y, Math.sin(t) * r], [0, ny, 0]));
    }
    for (let i = 0; i < n; i++) ny > 0 ? idx.push(c, ring[i + 1], ring[i]) : idx.push(c, ring[i], ring[i + 1]);
  };
  const TAU = Math.PI * 2;
  switch (shape) {
    case "sphere":
      grid(32, 16, (u, v) => {
        const t = u * TAU, p = v * Math.PI;
        const n = [Math.cos(t) * Math.sin(p), Math.cos(p), Math.sin(t) * Math.sin(p)];
        return [n.map((x) => x * 0.5), n];
      });
      break;
    case "plane":
      grid(1, 1, (u, v) => [[u - 0.5, 0, v - 0.5], [0, 1, 0]]);
      break;
    case "cylinder":
    case "cone": {
      const top = shape === "cone" ? 0 : 0.5;
      const slope = shape === "cone" ? 0.5 : 0;
      grid(32, 1, (u, v) => {
        const t = u * TAU, r = 0.5 + (top - 0.5) * (1 - v);
        const n = norm([Math.cos(t), slope, Math.sin(t)]);
        return [[Math.cos(t) * r, v - 0.5, Math.sin(t) * r], n];
      });
      disc(-0.5, -1, 0.5, 32);
      if (top > 0) disc(0.5, 1, top, 32);
      break;
    }
    case "torus":
      grid(48, 16, (u, v) => {
        const t = u * TAU, p = v * TAU, R = 0.35, r = 0.15;
        const n = [Math.cos(t) * Math.cos(p), Math.sin(p), Math.sin(t) * Math.cos(p)];
        return [[Math.cos(t) * (R + r * Math.cos(p)), r * Math.sin(p), Math.sin(t) * (R + r * Math.cos(p))], n];
      });
      break;
    default: // box
      for (const [n, u, v] of [
        [[1, 0, 0], [0, 0, -1], [0, 1, 0]],
        [[-1, 0, 0], [0, 0, 1], [0, 1, 0]],
        [[0, 1, 0], [1, 0, 0], [0, 0, -1]],
        [[0, -1, 0], [1, 0, 0], [0, 0, 1]],
        [[0, 0, 1], [1, 0, 0], [0, 1, 0]],
        [[0, 0, -1], [-1, 0, 0], [0, 1, 0]],
      ]) {
        const at = (su, sv) => vert([0, 1, 2].map((k) => (n[k] + su * u[k] + sv * v[k]) * 0.5), n);
        quad(at(-1, -1), at(1, -1), at(1, 1), at(-1, 1));
      }
  }
  // Wireframes draw each triangle edge once.
  const seen = new Set(), edges = [];
  for (let i = 0; i < idx.length; i += 3)
    for (const [a, b] of [[idx[i], idx[i + 1]], [idx[i + 1], idx[i + 2]], [idx[i + 2], idx[i]]]) {
      const key = a < b ? a * 65536 + b : b * 65536 + a;
      if (!seen.has(key)) {
        seen.add(key);
        edges.push(a, b);
      }
    }
  return { pos: new Float32Array(pos), nrm: new Float32Array(nrm), idx: new Uint16Array(idx), edges: new Uint16Array(edges) };
}

// ----- attributes -----

const vec3 = (s, d) => {
  const v = (s ?? "").split(" ").map(Number);
  return v.length === 3 && v.every(Number.isFinite) ? v : d;
};
const num = (s, d) => (s !== undefined && Number.isFinite(Number(s)) ? Number(s) : d);

/** Creates the WebGL renderer shared by every scene of a page. */
export function createScenes(doc) {
  const canvas = doc.createElement("canvas");
  const gl = canvas.getContext("webgl", { antialias: true, premultipliedAlpha: true, alpha: true });
  const win = doc.defaultView;
  const colors = new Map();
  const probe = doc.createElement("canvas").getContext("2d", { willReadFrequently: true });
  /** Any CSS color as [r, g, b, a] in 0..1. */
  const color = (s, d) => {
    if (!s) return d;
    let c = colors.get(s);
    if (!c) {
      probe.clearRect(0, 0, 1, 1);
      probe.fillStyle = "#000";
      probe.fillStyle = s;
      probe.fillRect(0, 0, 1, 1);
      const p = probe.getImageData(0, 0, 1, 1).data;
      colors.set(s, (c = [p[0] / 255, p[1] / 255, p[2] / 255, p[3] / 255]));
    }
    return c;
  };
  if (!gl) return null;

  const compile = (type, src) => {
    const s = gl.createShader(type);
    gl.shaderSource(s, src);
    gl.compileShader(s);
    if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(s));
    return s;
  };
  const prog = gl.createProgram();
  gl.attachShader(prog, compile(gl.VERTEX_SHADER, VS));
  gl.attachShader(prog, compile(gl.FRAGMENT_SHADER, FS));
  gl.linkProgram(prog);
  gl.useProgram(prog);
  const u = {};
  for (const n of ["model", "viewProj", "normalMat", "color", "ambient", "eye", "pickColor", "shininess", "pick", "lightCount", "lightPos", "lightColor"]) u[n] = gl.getUniformLocation(prog, n);
  const aPos = gl.getAttribLocation(prog, "position"), aNrm = gl.getAttribLocation(prog, "normal");
  gl.enableVertexAttribArray(aPos);
  gl.enableVertexAttribArray(aNrm);
  gl.enable(gl.DEPTH_TEST);

  const meshes = new Map();
  const geometry = (shape) => {
    let g = meshes.get(shape);
    if (!g) {
      const d = build(shape);
      const buf = (target, data) => {
        const b = gl.createBuffer();
        gl.bindBuffer(target, b);
        gl.bufferData(target, data, gl.STATIC_DRAW);
        return b;
      };
      g = {
        pos: buf(gl.ARRAY_BUFFER, d.pos),
        nrm: buf(gl.ARRAY_BUFFER, d.nrm),
        idx: buf(gl.ELEMENT_ARRAY_BUFFER, d.idx),
        edges: buf(gl.ELEMENT_ARRAY_BUFFER, d.edges),
        count: d.idx.length,
        edgeCount: d.edges.length,
      };
      meshes.set(shape, g);
    }
    return g;
  };

  const reduced = win.matchMedia?.("(prefers-reduced-motion: reduce)");
  let pickTarget = null;

  /** World matrices of a scene's items at time `t` (seconds), and its camera. */
  function layout(scene, view, t) {
    const a = scene.attrs;
    const spinT = reduced?.matches ? 0 : t;
    const world = [];
    for (const it of scene.items) {
      const p = vec3(it.attrs.position, it.kind === 2 ? [3, 5, 4] : [0, 0, 0]);
      const spin = vec3(it.attrs.spin, [0, 0, 0]);
      const r = vec3(it.attrs.rotation, [0, 0, 0]).map((v, i) => v + spin[i] * spinT);
      const local = transform(p, r, vec3(it.attrs.size, [1, 1, 1]));
      world.push(it.parent >= 0 ? mul(world[it.parent], local) : local);
    }
    const target = vec3(a.target, [0, 0, 0]);
    // Orbiting turns the camera around the target.
    const off = sub(vec3(a.camera, [0, 1.5, 5]), target);
    const dist = Math.hypot(...off) || 1;
    const yaw = Math.atan2(off[0], off[2]) + view.yaw;
    const pitch = Math.max(-1.5, Math.min(1.5, Math.asin(off[1] / dist) + view.pitch));
    const eye = [target[0] + dist * Math.cos(pitch) * Math.sin(yaw), target[1] + dist * Math.sin(pitch), target[2] + dist * Math.cos(pitch) * Math.cos(yaw)];
    return { world, eye, target, fov: num(a.fov, 50) };
  }

  function render(scene, view, w, h, t, picking) {
    const { world, eye, target, fov } = layout(scene, view, t);
    gl.viewport(0, 0, w, h);
    const bg = picking ? [0, 0, 0, 1] : scene.attrs.background === "transparent" ? [0, 0, 0, 0] : color(scene.attrs.background, [0.06, 0.07, 0.08, 1]);
    gl.clearColor(bg[0] * bg[3], bg[1] * bg[3], bg[2] * bg[3], bg[3]);
    gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.uniformMatrix4fv(u.viewProj, false, mul(perspective(fov, w / h, 0.05, 500), lookAt(eye, target)));
    gl.uniform3fv(u.eye, eye);
    gl.uniform1i(u.pick, picking ? 1 : 0);
    // Lights: without any, a soft ambient light and one from above.
    let ambient = [0, 0, 0];
    const pos = [], col = [];
    const lights = scene.items.flatMap((it, i) => (it.kind === 2 ? [[it, [world[i][12], world[i][13], world[i][14]]]] : []));
    const defaults = [[{ attrs: { kind: "ambient", intensity: "0.35" } }, null], [{ attrs: {} }, [3, 5, 4]]];
    for (const [it, at] of lights.length ? lights : defaults) {
      const c = color(it.attrs.color, [1, 1, 1, 1]).map((v) => v * num(it.attrs.intensity, 1));
      const kind = it.attrs.kind ?? "directional";
      if (kind === "ambient") ambient = ambient.map((v, i) => v + c[i]);
      else if (pos.length < 32) {
        pos.push(...at, kind === "point" ? 1 : 0);
        col.push(c[0], c[1], c[2]);
      }
    }
    gl.uniform3fv(u.ambient, ambient);
    gl.uniform1i(u.lightCount, pos.length / 4);
    if (pos.length) {
      gl.uniform4fv(u.lightPos, pos);
      gl.uniform3fv(u.lightColor, col);
    }
    scene.items.forEach((it, i) => {
      if (it.kind !== 1 || (picking && !it.clickable)) return;
      const g = geometry(it.attrs.shape ?? "box");
      gl.bindBuffer(gl.ARRAY_BUFFER, g.pos);
      gl.vertexAttribPointer(aPos, 3, gl.FLOAT, false, 0, 0);
      gl.bindBuffer(gl.ARRAY_BUFFER, g.nrm);
      gl.vertexAttribPointer(aNrm, 3, gl.FLOAT, false, 0, 0);
      gl.uniformMatrix4fv(u.model, false, world[i]);
      gl.uniformMatrix3fv(u.normalMat, false, normalMatrix(world[i]));
      const c = color(it.attrs.color, [0.82, 0.42, 0.28, 1]);
      gl.uniform3f(u.color, c[0], c[1], c[2]);
      gl.uniform1f(u.shininess, num(it.attrs.shininess, 32));
      // Pick colors are item numbers + 1 (0 is the background).
      const k = i + 1;
      gl.uniform3f(u.pickColor, (k & 255) / 255, ((k >> 8) & 255) / 255, ((k >> 16) & 255) / 255);
      const wire = it.attrs.wireframe === "true" && !picking;
      gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, wire ? g.edges : g.idx);
      gl.drawElements(wire ? gl.LINES : gl.TRIANGLES, wire ? g.edgeCount : g.count, gl.UNSIGNED_SHORT, 0);
    });
  }

  return {
    /** Whether a scene moves by itself (spinning items). */
    animated(scene) {
      return !reduced?.matches && scene.items.some((it) => it.attrs.spin && vec3(it.attrs.spin, [0, 0, 0]).some((v) => v !== 0));
    },
    /** Draws `scene` at `w`×`h` device pixels; returns the canvas holding the image. */
    draw(scene, view, w, h, t) {
      if (canvas.width !== w || canvas.height !== h) {
        canvas.width = w;
        canvas.height = h;
      }
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      render(scene, view, w, h, t, false);
      return canvas;
    },
    /** The node id of the clickable mesh at device pixel (x, y), or -1. */
    pick(scene, view, w, h, t, x, y) {
      // An offscreen target without antialiasing, so pick colors are exact.
      if (!pickTarget || pickTarget.w !== w || pickTarget.h !== h) {
        const fb = gl.createFramebuffer(), tex = gl.createTexture(), depth = gl.createRenderbuffer();
        gl.bindTexture(gl.TEXTURE_2D, tex);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
        gl.bindRenderbuffer(gl.RENDERBUFFER, depth);
        gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, w, h);
        gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
        gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
        gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth);
        pickTarget = { fb, w, h };
      }
      gl.bindFramebuffer(gl.FRAMEBUFFER, pickTarget.fb);
      render(scene, view, w, h, t, true);
      const px = new Uint8Array(4);
      gl.readPixels(Math.round(x), Math.round(h - 1 - y), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      const k = px[0] + (px[1] << 8) + (px[2] << 16);
      return k > 0 && scene.items[k - 1] ? scene.items[k - 1].id : -1;
    },
  };
}
